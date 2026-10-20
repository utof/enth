//! The host side: callbacks the plugin may make, plugin loading and instantiation.

use std::ffi::{CStr, CString};
use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use clack_extensions::gui::{GuiSize, HostGui, HostGuiImpl};
use clack_extensions::log::{HostLog, HostLogImpl, LogSeverity};
use clack_extensions::params::{
    HostParams, HostParamsImplMainThread, HostParamsImplShared, ParamClearFlags, ParamRescanFlags,
};
use clack_extensions::state::{HostState, HostStateImpl};
use clack_extensions::thread_check::{HostThreadCheck, HostThreadCheckImpl};
use clack_host::prelude::*;

/// Thread-safe host state, shared with the plugin. Requests are recorded here and serviced by
/// whichever loop drives the plugin (`editor` and `render` poll them).
#[derive(Default)]
pub struct Shared {
    pub restart_requested: AtomicBool,
    pub process_requested: AtomicBool,
    pub callback_requested: AtomicBool,
    pub flush_requested: AtomicBool,
    /// Number of `gui.closed` notifications.
    pub gui_closed: AtomicU32,
    /// Number of `gui.request_resize` calls the host accepted.
    pub resize_requests: AtomicU32,
    pub state_dirty: AtomicBool,
    /// Latest size the plugin asked for; the editor loop applies it to the parent window.
    pub pending_resize: Mutex<Option<(u32, u32)>>,
    /// Lines the plugin sent through the host's log extension.
    pub log: Mutex<Vec<String>>,
    /// The thread that created the instance: the main thread for the thread-check extension.
    pub main_thread: MainThreadId,
}

/// The id of the thread that made it (`Default` captures the current thread).
pub struct MainThreadId(std::thread::ThreadId);

impl Default for MainThreadId {
    fn default() -> Self {
        Self(std::thread::current().id())
    }
}

/// `ENTH_TEST_HOST_THREAD_CHECK=1` makes the host offer CLAP's thread-check extension, as REAPER and Bitwig do
/// (R1/S1 lane INT, `cargo xtask rtsan`). Off by default, so the other gates keep the host they were measured with.
/// Without it, nice-plug's `is_main_thread` falls back to `std::thread::current()` in the plugin's own std, which
/// allocates the first time it runs on a thread, the audio thread included (a `permit_alloc`, its FIXME).
pub fn thread_check_offered() -> bool {
    std::env::var_os("ENTH_TEST_HOST_THREAD_CHECK").is_some_and(|v| v == "1")
}

impl HostThreadCheckImpl for Shared {
    fn is_main_thread(&self) -> bool {
        std::thread::current().id() == self.main_thread.0
    }

    fn is_audio_thread(&self) -> bool {
        !self.is_main_thread()
    }
}

impl SharedHandler<'_> for Shared {
    fn request_restart(&self) {
        self.restart_requested.store(true, Ordering::SeqCst);
    }
    fn request_process(&self) {
        self.process_requested.store(true, Ordering::SeqCst);
    }
    fn request_callback(&self) {
        self.callback_requested.store(true, Ordering::SeqCst);
    }
}

impl HostLogImpl for Shared {
    fn log(&self, severity: LogSeverity, message: &str) {
        if let Ok(mut lines) = self.log.lock() {
            lines.push(format!("[{severity}] {message}"));
        }
    }
}

impl HostGuiImpl for Shared {
    fn resize_hints_changed(&self) {}

    fn request_resize(&self, new_size: GuiSize) -> Result<(), HostError> {
        self.resize_requests.fetch_add(1, Ordering::SeqCst);
        if let Ok(mut pending) = self.pending_resize.lock() {
            *pending = Some((new_size.width, new_size.height));
        }
        Ok(())
    }

    fn request_show(&self) -> Result<(), HostError> {
        Ok(())
    }

    fn request_hide(&self) -> Result<(), HostError> {
        Ok(())
    }

    fn closed(&self, _was_destroyed: bool) {
        self.gui_closed.fetch_add(1, Ordering::SeqCst);
    }
}

impl HostParamsImplShared for Shared {
    fn request_flush(&self) {
        self.flush_requested.store(true, Ordering::SeqCst);
    }
}

pub struct MainThread<'a> {
    shared: &'a Shared,
}

impl<'a> MainThreadHandler<'a> for MainThread<'a> {}

impl HostParamsImplMainThread for MainThread<'_> {
    fn rescan(&self, _flags: ParamRescanFlags) {}
    fn clear(&self, _param_id: ClapId, _flags: ParamClearFlags) {}
}

impl HostStateImpl for MainThread<'_> {
    fn mark_dirty(&self) {
        self.shared.state_dirty.store(true, Ordering::SeqCst);
    }
}

pub struct Host;

impl HostHandlers for Host {
    type Shared<'a> = Shared;
    type MainThread<'a> = MainThread<'a>;
    type AudioProcessor<'a> = ();

    fn declare_extensions(builder: &mut HostExtensions<'_, Self>, _shared: &Self::Shared<'_>) {
        builder
            .register::<HostLog>()
            .register::<HostGui>()
            .register::<HostParams>()
            .register::<HostState>();
        if thread_check_offered() {
            builder.register::<HostThreadCheck>();
        }
    }
}

/// Loads a `.clap` file.
pub fn load_entry(path: &Path) -> Result<PluginEntry, String> {
    // SAFETY: loading a plugin library runs its initialisation code; that is what a host does.
    unsafe { PluginEntry::load(path) }.map_err(|e| format!("cannot load {}: {e}", path.display()))
}

pub fn host_info() -> Result<HostInfo, String> {
    HostInfo::new(
        "enth-test-host",
        "enth",
        "https://github.com/utof/enth",
        env!("CARGO_PKG_VERSION"),
    )
    .map_err(|e| format!("bad host info: {e}"))
}

/// The ids of the plugins in `entry`, in factory order.
pub fn plugin_ids(entry: &PluginEntry) -> Result<Vec<CString>, String> {
    let factory = entry
        .get_plugin_factory()
        .ok_or("the library has no CLAP plugin factory")?;
    Ok(factory
        .plugin_descriptors()
        .filter_map(|d| d.id().map(CStr::to_owned))
        .collect())
}

/// Picks `wanted`, or the only plugin in the library.
pub fn resolve_plugin_id(entry: &PluginEntry, wanted: Option<&str>) -> Result<CString, String> {
    let ids = plugin_ids(entry)?;
    match (wanted, ids.as_slice()) {
        (Some(w), ids) => ids
            .iter()
            .find(|id| id.to_bytes() == w.as_bytes())
            .cloned()
            .ok_or_else(|| {
                format!(
                    "no plugin with id {w:?}; the library has {:?}",
                    ids.iter().map(|i| i.to_string_lossy()).collect::<Vec<_>>()
                )
            }),
        (None, [only]) => Ok(only.clone()),
        (None, ids) => Err(format!(
            "the library has {} plugins; pick one with --plugin-id ({:?})",
            ids.len(),
            ids.iter().map(|i| i.to_string_lossy()).collect::<Vec<_>>()
        )),
    }
}

/// The `version` the plugin's descriptor reports (nice-plug writes the same string into its saved state).
pub fn plugin_version(entry: &PluginEntry, id: &CStr) -> String {
    entry
        .get_plugin_factory()
        .and_then(|f| f.plugin_descriptors().find(|d| d.id() == Some(id)))
        .and_then(|d| d.version().map(|v| v.to_string_lossy().into_owned()))
        .unwrap_or_default()
}

pub fn instantiate(entry: &PluginEntry, id: &CStr) -> Result<PluginInstance<Host>, String> {
    let info = host_info()?;
    PluginInstance::<Host>::new(
        |()| Shared::default(),
        |shared| MainThread { shared },
        entry,
        id,
        &info,
    )
    .map_err(|e| format!("cannot instantiate {}: {e}", id.to_string_lossy()))
}

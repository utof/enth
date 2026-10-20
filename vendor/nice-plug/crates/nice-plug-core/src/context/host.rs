//! Requests a plugin makes of the host outside the activate, process and GUI contexts (vendored fork,
//! enth patches 6 and 8).

use std::sync::Arc;

/// A handle for requests a plugin makes of the host from outside the activate, process and GUI
/// contexts: the state-load path (a persistent field's `set`), the GUI thread, a background task.
///
/// The wrapper performs every request on the host's main thread: at once when it is called there,
/// and otherwise queued for it (CLAP: the task queue and `clap_host::request_callback()`; VST3: the
/// host's `IRunLoop` while the editor is open, else nice-plug's event loop). A request that is
/// still waiting absorbs later ones of the same kind, so calling it often costs one host call.
///
/// The wrapper passes it once to [`Plugin::host_requests()`][crate::plugin::Plugin::host_requests()],
/// right after the plugin is created and before the host calls anything else. It is cheap to clone,
/// `Send` and `Sync`.
///
/// # Note
///
/// Never call these from `process()` or `reset()`. A host may run the audio thread on its main
/// thread (offline rendering may), and the request would then call the host from inside
/// `process()`.
#[derive(Clone)]
pub struct HostRequests {
    inner: Arc<dyn HostRequestsInner>,
}

impl HostRequests {
    pub fn new(inner: Arc<dyn HostRequestsInner>) -> Self {
        Self { inner }
    }

    /// Tell the host that the plugin's state changed in a way it cannot see (not through a
    /// parameter), so the project needs saving. CLAP: `clap_host_state.mark_dirty()`; VST3:
    /// `IComponentHandler2::setDirty(true)`. Both are main-thread calls. Does nothing if the host
    /// does not offer it.
    pub fn mark_dirty(&self) {
        self.inner.mark_dirty();
    }

    /// Ask the host to re-read parameter info (names) after `Param::set_display_name()`. CLAP:
    /// `clap_host_params.rescan(INFO | TEXT)`; VST3: `restartComponent(kParamTitlesChanged)`. Both
    /// are main-thread calls. Unlike the process context's request, this one can be made from the
    /// state-load path, so a load's new names reach the host before it processes again, never
    /// from inside `process()`. (Vendored fork, enth patch 8.)
    pub fn request_param_info_rescan(&self) {
        self.inner.request_param_info_rescan();
    }
}

/// What a wrapper implements for [`HostRequests`].
//
// # Safety
//
// The implementation may be called from any thread except the audio thread, concurrently, and must
// perform the host call on the host's main thread.
pub trait HostRequestsInner: Send + Sync + 'static {
    /// See [`HostRequests::mark_dirty()`].
    fn mark_dirty(&self);

    /// See [`HostRequests::request_param_info_rescan()`].
    fn request_param_info_rescan(&self);
}

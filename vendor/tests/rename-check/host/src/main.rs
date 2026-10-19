//! Scratch CLAP host for the FK lane: does a plugin's runtime parameter rename reach the host?
//!
//! Loads a .clap, reads the name of parameter `Slot 1` through `clap_plugin_params.get_info`, optionally sets
//! the trigger parameter (`Slot 64` = 1.0) in the first block, runs blocks, services `request_callback` by
//! calling `on_main_thread`, records every `clap_host_params.rescan` the plugin makes (with its flags) and
//! reads the names again.
//!
//!   rename-host <plugin.clap> [--no-trigger]
use std::ffi::CString;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use clack_extensions::params::{
    HostParams, HostParamsImplMainThread, HostParamsImplShared, ParamClearFlags, ParamInfoBuffer,
    ParamRescanFlags, PluginParams,
};
use clack_host::events::event_types::ParamValueEvent;
use clack_host::prelude::*;

static CALLBACK: AtomicBool = AtomicBool::new(false);
static RESCANS: Mutex<Vec<String>> = Mutex::new(Vec::new());

#[derive(Default)]
struct Shared;

impl SharedHandler<'_> for Shared {
    fn request_restart(&self) {}
    fn request_process(&self) {}
    fn request_callback(&self) {
        CALLBACK.store(true, Ordering::SeqCst);
    }
}

impl HostParamsImplShared for Shared {
    fn request_flush(&self) {}
}

struct MainThread<'a> {
    _shared: &'a Shared,
}

impl<'a> MainThreadHandler<'a> for MainThread<'a> {}

impl HostParamsImplMainThread for MainThread<'_> {
    fn rescan(&self, flags: ParamRescanFlags) {
        RESCANS.lock().unwrap().push(format!("{flags:?} (bits {:#x})", flags.bits()));
    }
    fn clear(&self, _param_id: ClapId, _flags: ParamClearFlags) {}
}

struct Host;

impl HostHandlers for Host {
    type Shared<'a> = Shared;
    type MainThread<'a> = MainThread<'a>;
    type AudioProcessor<'a> = ();

    fn declare_extensions(builder: &mut HostExtensions<'_, Self>, _shared: &Self::Shared<'_>) {
        builder.register::<HostParams>();
    }
}

fn names(instance: &mut PluginInstance<Host>) -> Vec<(u32, String)> {
    let handle = instance.plugin_handle();
    let ext = handle.get_extension::<PluginParams>().expect("the plugin has no params extension");
    let mut buffer = ParamInfoBuffer::new();
    (0..ext.count(&handle))
        .filter_map(|i| {
            let info = ext.get_info(&handle, i, &mut buffer)?;
            Some((info.id.get(), String::from_utf8_lossy(info.name).into_owned()))
        })
        .collect()
}

fn name_of(all: &[(u32, String)], wanted: &str) -> Option<u32> {
    all.iter().find(|(_, n)| n == wanted).map(|(id, _)| *id)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let path = PathBuf::from(args.first().expect("usage: rename-host <plugin.clap> [--no-trigger]"));
    let trigger = !args.iter().any(|a| a == "--no-trigger");

    // SAFETY: loading a plugin library runs its initialisation code; that is what a host does.
    let entry = unsafe { PluginEntry::load(&path) }.expect("cannot load the plugin");
    let id: CString = entry
        .get_plugin_factory()
        .unwrap()
        .plugin_descriptors()
        .next()
        .unwrap()
        .id()
        .unwrap()
        .to_owned();
    let info = HostInfo::new("rename-host", "enth", "", "0").unwrap();
    let mut instance = PluginInstance::<Host>::new(
        |()| Shared::default(),
        |shared| MainThread { _shared: shared },
        &entry,
        &id,
        &info,
    )
    .expect("cannot instantiate");

    let before = names(&mut instance);
    let slot1 = name_of(&before, "Slot 1").expect("no parameter named Slot 1");
    let slot64 = name_of(&before, "Slot 64").expect("no parameter named Slot 64");
    let slot2 = name_of(&before, "Slot 2").expect("no parameter named Slot 2");
    println!("before: id {slot1} is {:?}", before.iter().find(|(i, _)| *i == slot1).unwrap().1);

    let block = 256u32;
    let stopped = instance
        .activate(
            |_, _| (),
            PluginAudioConfiguration { sample_rate: 48_000.0, min_frames_count: block, max_frames_count: block },
        )
        .expect("activation failed");
    let _ = slot2;

    // The audio thread: what a real host does. nice-plug sees a process() call from a thread that is not the
    // main thread, so the rename is queued for the main thread and the host is asked for a callback.
    let audio = std::thread::spawn(move || {
        let mut processor = stopped.start_processing().expect("start_processing failed");
        let mut out = vec![vec![0.0f32; block as usize]; 2];
        let mut out_ports = AudioPorts::with_capacity(2, 1);
        let mut events_in = EventBuffer::with_capacity(8);
        let mut events_out = EventBuffer::with_capacity(8);
        for index in 0..40u64 {
            events_in.clear();
            events_out.clear();
            if index == 0 && trigger {
                events_in.push(&ParamValueEvent::new(0, ClapId::new(slot64), Pckn::match_all(), 1.0));
            }
            let input_events = InputEvents::from_buffer(&events_in);
            let mut output_events = OutputEvents::from_buffer(&mut events_out);
            let mut audio = out_ports.with_output_buffers([AudioPortBuffer {
                latency: 0,
                channels: AudioPortBufferType::f32_output_only(out.iter_mut().map(Vec::as_mut_slice)),
            }]);
            processor
                .process(&InputAudioBuffers::empty(), &mut audio, &input_events, &mut output_events, Some(index * u64::from(block)), None)
                .expect("process failed");
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        processor.stop_processing()
    });

    // The main thread: wait for the plugin's request_callback, look at the names, then service it.
    let mut mid = before.clone();
    let mut callback_seen = false;
    while !audio.is_finished() || CALLBACK.load(Ordering::SeqCst) {
        if CALLBACK.swap(false, Ordering::SeqCst) {
            if !callback_seen {
                mid = names(&mut instance);
                callback_seen = true;
            }
            instance.call_on_main_thread_callback();
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    let stopped = audio.join().expect("audio thread panicked");
    println!(
        "plugin asked for a main-thread callback: {callback_seen}; before servicing it, id {slot1} was {:?}",
        mid.iter().find(|(i, _)| *i == slot1).unwrap().1
    );
    let after = names(&mut instance);
    println!("after on_main_thread: id {slot1} is {:?}", after.iter().find(|(i, _)| *i == slot1).unwrap().1);
    println!("rescans seen by the host: {}", RESCANS.lock().unwrap().len());
    for r in RESCANS.lock().unwrap().iter() {
        println!("  rescan({r})");
    }
    let changed: Vec<_> = before.iter().zip(&after).filter(|(b, a)| b != a).map(|(b, a)| format!("{}: {:?} -> {:?}", b.0, b.1, a.1)).collect();
    println!("parameters whose name differs: {}", changed.len());
    for c in &changed {
        println!("  {c}");
    }
    instance.deactivate(stopped);
    let renamed = !changed.is_empty();
    let saw_rescan = !RESCANS.lock().unwrap().is_empty();
    println!("RESULT renamed={renamed} rescan_seen={saw_rescan} trigger={trigger}");
    // trigger: both must hold; no trigger: neither may hold (negative control).
    let ok = if trigger { renamed && saw_rescan } else { !renamed && !saw_rescan };
    std::process::exit(if ok { 0 } else { 1 });
}

//! The panic hook writes a crash file (lane PBC of R1/S1). A test binary of its own, because the hook is process-wide.

use std::fs;
use std::path::PathBuf;

use enth_plugin::crash;

fn scratch() -> PathBuf {
    std::env::temp_dir().join(format!("enth-pbc-crash-it-{}", std::process::id()))
}

/// One test, in order: the hook is installed once per process and the panics must come after it.
#[test]
fn a_panic_writes_one_crash_file_and_the_chained_hook_still_runs() {
    let dir = scratch().join("Crashes");
    let _ = fs::remove_dir_all(scratch());

    // A hook that was there before ours must still run (the host's, or the default one).
    let chained = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
    let seen = std::sync::Arc::clone(&chained);
    std::panic::set_hook(Box::new(move |_| {
        seen.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }));

    // The home folder is hidden whatever it is; use the real one so a leak would show.
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .ok();
    assert!(crash::install_at(Some(dir.clone()), home.clone()));
    assert!(!crash::install_at(Some(dir.clone()), home.clone()), "once");

    // A panic on a named thread, with a message that names the home folder.
    let secret = home.clone().unwrap_or_else(|| "/home/someone".to_owned());
    let message = format!("could not open {secret}/Documents/enth/Presets/x.patch.json");
    let handle = std::thread::Builder::new()
        .name("audio-test".to_owned())
        .spawn(move || {
            let r = std::panic::catch_unwind(|| {
                panic!("{message}");
            });
            assert!(r.is_err());
        })
        .unwrap();
    handle.join().unwrap();
    assert_eq!(
        chained.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "the previous hook ran"
    );

    let files = crash::pending(&dir);
    assert_eq!(files.len(), 1, "{files:?}");
    let name = files[0].file_name().unwrap().to_str().unwrap();
    assert!(
        name.starts_with("enth-crash-") && name.ends_with("txt"),
        "{name}"
    );
    let text = crash::read_report(&files[0]).unwrap();
    for needle in [
        "enth crash report",
        "version: ",
        "time: 20",
        "thread: audio-test (ThreadId(",
        "location: ",
        "tests/crash.rs",
        "message: could not open ",
        "backtrace:",
    ] {
        assert!(text.contains(needle), "{needle:?} missing from\n{text}");
    }
    if let Some(home) = home.filter(|h| h.len() > 1) {
        assert!(
            !text.contains(home.trim_end_matches('/')),
            "the home folder leaked:\n{text}"
        );
        assert!(text.contains("message: could not open ~/Documents/enth/Presets/x.patch.json"));
    }

    // A second and a third panic write; the fourth is over the per-process cap.
    for _ in 0..4 {
        let _ = std::panic::catch_unwind(|| panic!("again"));
    }
    assert_eq!(
        crash::pending(&dir).len() as u32,
        crash::MAX_FILES_PER_PROCESS
    );

    // The editor marks one seen; it is not offered again but stays on disk.
    let seen = crash::mark_seen(&files[0]).unwrap();
    assert!(seen.exists());
    assert_eq!(
        crash::pending(&dir).len() as u32,
        crash::MAX_FILES_PER_PROCESS - 1
    );

    let _ = fs::remove_dir_all(scratch());
}

//! "Create from sound" through the plugin's shell (lane SK3, feature `create`): the decoders, the `enth-create`
//! thread and its per-job pool, cancel within one batch, the wall-clock cap, auditions, the bank lookup, and an unload
//! that never waits for a running job.
//!
//! The decoder fixtures in `tests/fixtures/create/` are our own renders (CC0 like the patches), made 2026-10-21 with
//! `enth render` and `FFmpeg` 6.1.1 run as a program (nothing of it is linked or shipped):
//!
//! | File | From |
//! | --- | --- |
//! | `tone.wav` | `content/templates/pluck_1osc.patch.json` at A3, held 0.3 s, 0.6 s, mixed to mono, 16-bit PCM, 48 kHz |
//! | `tone.flac` | the same, FLAC (so it decodes to exactly `tone.wav`'s samples) |
//! | `tone.ogg` | the same, Ogg Vorbis (libvorbis, `-q:a 3`) |
//! | `tone.mp3` | the same, MP3 (libmp3lame, 128 kbps) |
//! | `high-a7-48k.mp3` | `content/templates/init.patch.json` at A7 (3520 Hz), held 0.5 s, 0.7 s, mono, MP3 at 48 kbps (the encoder low-passes near 11 kHz, where the note has two partials in the octave under the cut) |

#![allow(
    clippy::disallowed_methods,
    reason = "test signals and timing; their bits do not matter"
)]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use enth_analysis::intake::{Codec, Intake, IntakeCfg};
use enth_session::create::{
    CreateEdit, CreateJobId, CreatePhase, CreatePost, CreateRequest, CreateRun, CreateSettings,
    CreateSource, CreateStop, Which,
};

use crate::create::decode::{self, Format};
use crate::create::{self, AuditionClip, AuditionPlayer, AuditionSlot, Create};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/create")
        .join(name)
}

fn never() -> bool {
    false
}

/// FNV-1a (64-bit) of the samples' bits, in hex.
fn hash(x: &[f32]) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for s in x {
        for b in s.to_le_bytes() {
            h ^= u64::from(b);
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
    }
    format!("{h:016x}")
}

#[test]
fn every_format_decodes_and_repeats_bit_for_bit() {
    let wav = decode::decode_path(&fixture("tone.wav"), &never).unwrap();
    for (name, format, lossy) in [
        ("tone.wav", Format::Wav, false),
        ("tone.flac", Format::Flac, false),
        ("tone.ogg", Format::Vorbis, true),
        ("tone.mp3", Format::Mp3, true),
        ("high-a7-48k.mp3", Format::Mp3, true),
    ] {
        let a = decode::decode_path(&fixture(name), &never).unwrap();
        let b = decode::decode_path(&fixture(name), &never).unwrap();
        assert_eq!(a.interleaved, b.interleaved, "{name}: the same bits twice");
        assert_eq!(
            (a.format, a.channels, a.sample_rate),
            (format, 1, 48_000),
            "{name}"
        );
        assert_eq!(a.codec.lossy, lossy, "{name}");
        assert_eq!(a.decimation, 1);
        let seconds = a.frames() as f64 / 48_000.0;
        assert!((0.55..0.8).contains(&seconds), "{name}: {seconds} s");
        println!(
            "{name}: {} frames, {} — FNV-1a of the f32 samples {}",
            a.frames(),
            a.describe(),
            hash(&a.interleaved)
        );
    }
    let flac = decode::decode_path(&fixture("tone.flac"), &never).unwrap();
    assert_eq!(
        flac.interleaved, wav.interleaved,
        "FLAC decodes to the WAV's samples"
    );
    assert_eq!(
        decode::decode_path(&fixture("tone.mp3"), &never)
            .unwrap()
            .codec
            .codec,
        Codec::Mp3
    );
}

/// A decoded fixture's bits on this machine (x86-64 Linux, 2026-10-21): a change in a decoder crate or the
/// decimator shows here. Other platforms are unverified (docs/plugin.md, "Create").
#[test]
fn the_decoded_bits_are_pinned_on_this_machine() {
    let pinned = [
        ("tone.wav", "71d5ba4860b84bf9"),
        ("tone.flac", "71d5ba4860b84bf9"),
        ("tone.ogg", "ea44c23bf406601f"),
        ("tone.mp3", "63b0442838b62a93"),
    ];
    for (name, want) in pinned {
        let d = decode::decode_path(&fixture(name), &never).unwrap();
        assert_eq!(hash(&d.interleaved), want, "{name}");
    }
}

/// `tone.wav`'s samples as an AIFF (`sowt`: an AIFF-C with little-endian samples), written in the test (lane `INTa`):
/// the repository commits no AIFF fixture.
fn tone_as_aiff(path: &Path, sowt: bool) {
    let mut r = hound::WavReader::open(fixture("tone.wav")).unwrap();
    let spec = r.spec();
    assert_eq!((spec.channels, spec.bits_per_sample), (1, 16));
    let samples: Vec<i16> = r.samples::<i16>().map(Result::unwrap).collect();
    let chunk = |id: &[u8; 4], body: &[u8]| -> Vec<u8> {
        let mut c = id.to_vec();
        c.extend_from_slice(&(body.len() as u32).to_be_bytes());
        c.extend_from_slice(body);
        if body.len() % 2 == 1 {
            c.push(0);
        }
        c
    };
    // 48,000 Hz as an 80-bit extended number.
    let rate = [0x40, 0x0E, 0xBB, 0x80, 0, 0, 0, 0, 0, 0];
    let mut comm = 1i16.to_be_bytes().to_vec();
    comm.extend_from_slice(&(samples.len() as u32).to_be_bytes());
    comm.extend_from_slice(&16i16.to_be_bytes());
    comm.extend_from_slice(&rate);
    if sowt {
        comm.extend_from_slice(b"sowt\x00\x00");
    }
    let mut ssnd = vec![0u8; 8];
    for s in &samples {
        ssnd.extend_from_slice(&if sowt {
            s.to_le_bytes()
        } else {
            s.to_be_bytes()
        });
    }
    let body = [
        chunk(b"COMM", &comm),
        chunk(b"ANNO", b"enth test"),
        chunk(b"SSND", &ssnd),
    ]
    .concat();
    let mut file = b"FORM".to_vec();
    file.extend_from_slice(&(body.len() as u32 + 4).to_be_bytes());
    file.extend_from_slice(if sowt { b"AIFC" } else { b"AIFF" });
    file.extend_from_slice(&body);
    std::fs::write(path, file).unwrap();
}

#[test]
fn aiff_and_aiff_c_decode_to_exactly_the_wavs_samples() {
    let wav = decode::decode_path(&fixture("tone.wav"), &never).unwrap();
    for (name, sowt) in [("tone.aiff", false), ("tone-sowt.aifc", true)] {
        let path = scratch(name);
        tone_as_aiff(&path, sowt);
        let d = decode::decode_path(&path, &never).unwrap();
        assert_eq!(d.interleaved, wav.interleaved, "{name}: the WAV's samples");
        assert_eq!(
            (d.format, d.channels, d.sample_rate, d.bits),
            (Format::Aiff, 1, 48_000, Some(16))
        );
        assert_eq!(d.describe(), "AIFF, 16-bit, 48000 Hz, mono");
        assert_eq!(d.codec.codec, Codec::Pcm);
        assert!(!d.codec.lossy);
        let it = Intake::with_cfg(
            &d.interleaved,
            d.channels,
            d.sample_rate,
            &IntakeCfg {
                codec: Some(d.codec),
                ..IntakeCfg::default()
            },
        )
        .unwrap();
        assert_eq!(it.report.perf.key, 57, "{name}: A3");
        let _ = std::fs::remove_file(path);
    }
}

fn write_wav(path: &Path, rate: u32, bits: u16, float: bool, seconds: f64) {
    let spec = hound::WavSpec {
        channels: 2,
        sample_rate: rate,
        bits_per_sample: bits,
        sample_format: if float {
            hound::SampleFormat::Float
        } else {
            hound::SampleFormat::Int
        },
    };
    let mut w = hound::WavWriter::create(path, spec).unwrap();
    let n = (seconds * f64::from(rate)) as usize;
    for i in 0..n {
        let t = i as f64 / f64::from(rate);
        let env = if t < 0.3 {
            1.0
        } else {
            (-(t - 0.3) * 8.0).exp()
        };
        let v = (2.0 * std::f64::consts::PI * 220.0 * t).sin() * 0.5 * env;
        for _ in 0..2 {
            if float {
                w.write_sample(v as f32).unwrap();
            } else {
                let scale = f64::from((1u32 << (bits - 1)) - 1);
                w.write_sample((v * scale).round() as i32).unwrap();
            }
        }
    }
    w.finalize().unwrap();
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("enth-create-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(name)
}

#[test]
fn wav_depths_and_rates_up_to_384_khz_decode_and_others_are_refused_with_the_rate_hint() {
    for (rate, bits, float, out_rate, factor) in [
        (44_100, 8, false, 44_100, 1),
        (48_000, 24, false, 48_000, 1),
        (48_000, 32, true, 48_000, 1),
        (96_000, 16, false, 96_000, 1),
        (176_400, 16, false, 176_400, 1),
        (192_000, 24, false, 96_000, 2),
        (352_800, 16, false, 176_400, 2),
        (384_000, 32, true, 96_000, 4),
    ] {
        let path = scratch(&format!("w-{rate}-{bits}-{float}.wav"));
        write_wav(&path, rate, bits, float, 0.5);
        let d = decode::decode_path(&path, &never).unwrap();
        assert_eq!(
            (d.sample_rate, d.decimation, d.file_rate),
            (out_rate, factor, rate),
            "{rate}"
        );
        assert_eq!(
            d.frames(),
            (0.5 * f64::from(rate)) as usize / factor as usize,
            "{rate}"
        );
        if factor > 1 {
            assert!(d.notes()[0].contains("decimated"), "{:?}", d.notes());
        }
        // The intake takes the decimated file.
        let it = Intake::new(&d.interleaved, d.channels, d.sample_rate).unwrap();
        assert_eq!(it.report.sample_rate, out_rate);
        assert_eq!(it.report.perf.key, 57, "{rate}: A3");
        let _ = std::fs::remove_file(path);
    }
    for rate in [7_000, 192_001] {
        let path = scratch(&format!("bad-{rate}.wav"));
        write_wav(&path, rate, 16, false, 0.2);
        let e = decode::decode_path(&path, &never).unwrap_err();
        assert_eq!(e.code, "sample_rate", "{rate}");
        assert!(e.hint.contains("decimated exactly 2:1"), "{}", e.hint);
        let _ = std::fs::remove_file(path);
    }
    let e = decode::decode_path(&fixture("missing.wav"), &never).unwrap_err();
    assert_eq!(e.code, "io");
    let junk = scratch("junk.bin");
    std::fs::write(&junk, b"not a sound at all, just some bytes").unwrap();
    assert_eq!(
        decode::decode_path(&junk, &never).unwrap_err().code,
        "unknown_format"
    );
    // A stop is seen between chunks.
    let long = scratch("long.wav");
    write_wav(&long, 48_000, 16, false, 3.0);
    assert_eq!(
        decode::decode_path(&long, &|| true).unwrap_err().code,
        "cancelled"
    );
}

#[test]
fn the_lossy_hint_reaches_the_intake_and_finds_a_high_notes_cut_off() {
    let d = decode::decode_path(&fixture("high-a7-48k.mp3"), &never).unwrap();
    assert!(d.codec.lossy);
    let clean = Intake::new(&d.interleaved, d.channels, d.sample_rate).unwrap();
    let hinted = Intake::with_cfg(
        &d.interleaved,
        d.channels,
        d.sample_rate,
        &IntakeCfg {
            codec: Some(d.codec),
            ..IntakeCfg::default()
        },
    )
    .unwrap();
    println!(
        "high-a7-48k.mp3 ({}): bandwidth without the hint {:?}, with it {:?}",
        d.describe(),
        clean.report.degradation.bandwidth_hz,
        hinted.report.degradation.bandwidth_hz
    );
    assert_eq!(
        hinted.report.cfg.codec,
        Some(d.codec),
        "the report records the hint"
    );
    assert!(clean.report.cfg.codec.is_none());
    assert_eq!(
        clean.report.degradation.bandwidth_hz, None,
        "two partials: the clean rule finds no cut"
    );
    let cut = hinted
        .report
        .degradation
        .bandwidth_hz
        .expect("the lossy hint finds the encoder's low-pass");
    assert!((9_000.0..13_000.0).contains(&cut), "{cut}");
}

// --- The thread and the pool -----------------------------------------------------------------------------------

/// Wait for posts of job `id` until `done` says so (or `limit` passes), collecting them all.
fn collect(
    c: &Create,
    limit: Duration,
    mut done: impl FnMut(&[(CreateJobId, CreatePost)]) -> bool,
) -> Vec<(CreateJobId, CreatePost)> {
    let t0 = Instant::now();
    let mut all = Vec::new();
    while t0.elapsed() < limit {
        all.extend(c.take_posts());
        if done(&all) {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    all
}

fn ended(posts: &[(CreateJobId, CreatePost)]) -> bool {
    posts
        .iter()
        .any(|(_, p)| matches!(p, CreatePost::Done { .. } | CreatePost::Failed { .. }))
}

fn rounds(posts: &[(CreateJobId, CreatePost)]) -> usize {
    posts
        .iter()
        .filter(|(_, p)| matches!(p, CreatePost::Round { .. }))
        .count()
}

fn run(id: u64, file: &str, seconds: f64, budget: Option<usize>, k: usize) -> CreateRequest {
    CreateRequest::Run(CreateRun {
        id: CreateJobId(id),
        source: CreateSource::file(fixture(file)),
        settings: CreateSettings {
            candidates: k,
            seconds,
            budget,
            seed: 1,
        },
    })
}

/// The tests that run jobs one at a time, so a test that reads the process's thread names sees only its own.
pub(crate) static SERIAL: parking_lot::Mutex<()> = parking_lot::Mutex::new(());

fn quiet() -> Create {
    let c = Create::new();
    c.set_bank_paths(Some(Vec::new()));
    // No model either (lane PUIb): a model file in the Documents folder must not change these tests.
    c.set_model_dirs(Some(Vec::new()));
    c.set_workers(2);
    c
}

/// The names of this process's threads (Linux).
#[cfg(target_os = "linux")]
fn thread_names() -> Vec<String> {
    std::fs::read_dir("/proc/self/task")
        .map(|d| {
            d.filter_map(|e| std::fs::read_to_string(e.ok()?.path().join("comm")).ok())
                .map(|s| s.trim().to_owned())
                .collect()
        })
        .unwrap_or_default()
}

/// Whether no thread whose name `matches` is listed, within a second (an exited thread can linger in
/// `/proc/self/task` for a moment after its join returns).
#[cfg(target_os = "linux")]
fn gone(matches: impl Fn(&str) -> bool) -> bool {
    let t0 = Instant::now();
    while t0.elapsed() < Duration::from_secs(1) {
        if !thread_names().iter().any(|n| matches(n)) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    false
}

#[test]
fn a_job_runs_from_the_file_to_k_candidates_and_its_threads_go_with_it() {
    let _serial = SERIAL.lock();
    let c = quiet();
    c.act(vec![run(1, "tone.ogg", 120.0, Some(120), 3)]);
    #[cfg(target_os = "linux")]
    let mut saw_search_threads = false;
    let mut posts = Vec::new();
    let t0 = Instant::now();
    while t0.elapsed() < Duration::from_secs(120) && !ended(&posts) {
        posts.extend(c.take_posts());
        #[cfg(target_os = "linux")]
        if thread_names().iter().any(|n| n.starts_with("enth-search")) {
            saw_search_threads = true;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    let intake = posts.iter().find_map(|(_, p)| match p {
        CreatePost::Intake(i) => Some(i.clone()),
        _ => None,
    });
    let intake = intake.expect("an intake post");
    assert!(
        intake.decoded.starts_with("Ogg Vorbis"),
        "{}",
        intake.decoded
    );
    assert_eq!(intake.report.perf.key, 57);
    assert!(
        intake.notes.iter().any(|n| n.contains("no k-NN bank")),
        "{:?}",
        intake.notes
    );
    // Scored by lane OBJ's robust distance, and the intake post says so.
    assert_eq!(create::OBJECTIVE, "logmel-pool85-robust/1");
    assert!(
        intake
            .notes
            .iter()
            .any(|n| *n == format!("objective: {}", create::OBJECTIVE)),
        "{:?}",
        intake.notes
    );
    assert!(rounds(&posts) >= 2, "a post per round");
    let Some((
        id,
        CreatePost::Done {
            stop,
            candidates,
            progress,
            ..
        },
    )) = posts.last()
    else {
        panic!("ends with Done: {:?}", posts.last());
    };
    assert_eq!(*id, CreateJobId(1));
    assert_eq!(*stop, CreateStop::Budget);
    assert_eq!(candidates.len(), 3, "K candidates");
    assert!(progress.renders <= 120);
    assert!(c.idle());
    #[cfg(target_os = "linux")]
    {
        assert!(
            saw_search_threads,
            "the pool's threads are named enth-search-N"
        );
        assert!(
            gone(|n| n.starts_with("enth-search")),
            "no search thread outlives its job: {:?}",
            thread_names()
        );
    }
    // Audition the best: a clip at the host's rate reaches the audio side and plays once.
    c.set_host_rate(44_100.0);
    let best = candidates
        .iter()
        .min_by(|a, b| a.distance.total_cmp(&b.distance))
        .unwrap();
    let slot = c.audition_slot();
    let mut player = AuditionPlayer::new(Arc::clone(&slot));
    c.act(vec![CreateRequest::Audition {
        id: CreateJobId(1),
        branch: best.branch,
        which: Which::Refined,
        patch: Box::new(best.patch.clone()),
    }]);
    let t0 = Instant::now();
    let (mut l, mut r) = (vec![0.0f32; 512], vec![0.0f32; 512]);
    let mut energy = 0.0f64;
    let mut blocks = 0;
    while t0.elapsed() < Duration::from_secs(30) && (energy == 0.0 || player.playing()) {
        l.fill(0.0);
        r.fill(0.0);
        player.mix(&mut l, &mut r);
        energy += l.iter().map(|v| f64::from(*v) * f64::from(*v)).sum::<f64>();
        if player.playing() {
            blocks += 1;
        } else {
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    assert!(energy > 0.0, "the audition played");
    // About the clip's length at 44.1 kHz, then silence.
    assert!((10..200).contains(&blocks), "{blocks} blocks of 512");
    l.fill(0.0);
    player.mix(&mut l, &mut r);
    assert!(l.iter().all(|v| *v == 0.0), "played once");
    let _ = c.take_posts();
}

#[test]
fn a_cancel_lands_within_one_batch_and_the_dropped_batch_is_never_told() {
    let _serial = SERIAL.lock();
    let c = quiet();
    c.act(vec![run(7, "tone.wav", 300.0, Some(1_000_000), 4)]);
    let posts = collect(&c, Duration::from_secs(120), |p| rounds(p) >= 2);
    assert!(rounds(&posts) >= 2, "two rounds before the cancel");
    let told = posts
        .iter()
        .rev()
        .find_map(|(_, p)| match p {
            CreatePost::Round { progress, .. } => Some(progress.renders),
            _ => None,
        })
        .unwrap();
    let t0 = Instant::now();
    c.act(vec![CreateRequest::Cancel(CreateJobId(7))]);
    let after = collect(&c, Duration::from_secs(60), ended);
    let latency = t0.elapsed();
    let Some((
        _,
        CreatePost::Done {
            stop,
            progress,
            notes,
            ..
        },
    )) = after.last()
    else {
        panic!("{after:?}");
    };
    assert_eq!(*stop, CreateStop::Cancelled);
    assert!(notes.iter().any(|n| n == "cancelled"), "{notes:?}");
    // At most the rounds already told when the cancel arrived, plus the one in flight then, which counts only if it
    // finished before the flag was seen.
    let last_round = after
        .iter()
        .rev()
        .find_map(|(_, p)| match p {
            CreatePost::Round { progress, .. } => Some(progress.renders),
            _ => None,
        })
        .unwrap_or(told);
    assert_eq!(
        progress.renders, last_round,
        "the batch out at the cancel is dropped"
    );
    assert!(
        rounds(&after) <= 1,
        "at most the round in flight: {}",
        rounds(&after)
    );
    println!(
        "cancel to Done: {:.1} ms (debug build, 2 workers)",
        latency.as_secs_f64() * 1000.0
    );
    assert!(latency < Duration::from_secs(5), "{latency:?}");
}

#[test]
fn the_wall_clock_cap_is_enforced() {
    let _serial = SERIAL.lock();
    let c = quiet();
    let cap = 3.0;
    let t0 = Instant::now();
    c.act(vec![run(3, "tone.flac", cap, Some(1_000_000), 8)]);
    let posts = collect(&c, Duration::from_secs(60), ended);
    let took = t0.elapsed().as_secs_f64();
    let Some((
        _,
        CreatePost::Done {
            stop,
            progress,
            notes,
            candidates,
        },
    )) = posts.last()
    else {
        panic!("{posts:?}");
    };
    assert_eq!(*stop, CreateStop::Cancelled);
    assert!(notes.iter().any(|n| n.contains("time cap")), "{notes:?}");
    assert_ne!(candidates.len(), 0);
    println!(
        "cap {cap} s: Done after {took:.2} s (job clock {:.2} s), {} renders",
        progress.seconds, progress.renders
    );
    // The cap plus one render per worker (debug build).
    assert!(progress.seconds < cap + 1.5, "{}", progress.seconds);
    assert!(took < cap + 2.5, "{took}");
}

#[test]
fn a_cancel_before_the_job_starts_ends_it_at_once() {
    let _serial = SERIAL.lock();
    let c = quiet();
    // The first job holds the thread; the second waits and is cancelled while queued.
    c.act(vec![
        run(1, "tone.wav", 300.0, Some(1_000_000), 2),
        run(2, "tone.wav", 60.0, None, 2),
    ]);
    c.act(vec![CreateRequest::Cancel(CreateJobId(2))]);
    let posts = collect(&c, Duration::from_secs(10), |p| {
        p.iter().any(|(id, _)| *id == CreateJobId(2))
    });
    let second: Vec<_> = posts
        .iter()
        .filter(|(id, _)| *id == CreateJobId(2))
        .collect();
    assert!(
        matches!(
            second.first(),
            Some((
                _,
                CreatePost::Done {
                    stop: CreateStop::Cancelled,
                    ..
                }
            ))
        ),
        "{second:?}"
    );
    c.act(vec![CreateRequest::Cancel(CreateJobId(1))]);
}

#[test]
fn a_bad_file_fails_with_its_code_and_hint() {
    let _serial = SERIAL.lock();
    let c = quiet();
    c.act(vec![CreateRequest::Run(CreateRun {
        id: CreateJobId(4),
        source: CreateSource::file("/no/such/file.wav"),
        settings: CreateSettings::default(),
    })]);
    let posts = collect(&c, Duration::from_secs(30), ended);
    let Some((_, CreatePost::Failed { code, hint, .. })) = posts.last() else {
        panic!("{posts:?}");
    };
    assert_eq!(code, "io");
    assert_ne!(hint, "");
}

#[test]
fn dropping_the_shell_never_waits_for_a_running_job() {
    let _serial = SERIAL.lock();
    let c = quiet();
    c.set_workers(0);
    c.act(vec![run(9, "tone.wav", 300.0, Some(1_000_000), 8)]);
    let posts = collect(&c, Duration::from_secs(120), |p| rounds(p) >= 1);
    assert!(rounds(&posts) >= 1, "the search is running");
    let t0 = Instant::now();
    drop(c);
    let took = t0.elapsed();
    println!(
        "drop with a job running: {:.1} ms",
        took.as_secs_f64() * 1000.0
    );
    assert!(took < Duration::from_secs(3), "{took:?}");
    // Joined; the kernel may list an exited thread for a moment after its join returns.
    #[cfg(target_os = "linux")]
    assert!(
        gone(|n| n == "enth-create" || n.starts_with("enth-search")),
        "{:?}",
        thread_names()
    );
}

/// The real plugin: a job started through the session (the editor's path), then the plugin dropped as a host
/// unloads it, while the search runs.
#[test]
fn unloading_the_plugin_never_hangs_on_a_running_job() {
    let _serial = SERIAL.lock();
    let plugin = crate::Enth::default();
    let shared = Arc::clone(plugin.shared());
    shared.create.set_bank_paths(Some(Vec::new()));
    shared.create.set_model_dirs(Some(Vec::new()));
    let (started, _) = shared.with_session(|s| {
        s.create_start(
            CreateSource::file(fixture("tone.wav")),
            CreateSettings {
                seconds: 300.0,
                budget: Some(1_000_000),
                ..CreateSettings::default()
            },
        )
    });
    let id = started.unwrap();
    let t0 = Instant::now();
    let mut searching = false;
    while t0.elapsed() < Duration::from_secs(120) && !searching {
        let posts = shared.create.take_posts();
        let ((), _) = shared.with_session(|s| {
            for (pid, post) in posts {
                let _ = s.create_post(pid, post);
            }
        });
        searching = shared
            .with_session(|s| {
                s.create().is_some_and(|v| {
                    v.id == id && v.phase == CreatePhase::Searching && !v.candidates.is_empty()
                })
            })
            .0;
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(searching, "the job reached its search");
    let t0 = Instant::now();
    drop(shared);
    drop(plugin);
    let took = t0.elapsed();
    println!(
        "plugin dropped with a job running: {:.1} ms",
        took.as_secs_f64() * 1000.0
    );
    assert!(took < Duration::from_secs(3), "{took:?}");
}

#[test]
fn the_bank_is_looked_for_beside_the_plugin_then_in_the_models_folder() {
    struct Env(Vec<(&'static str, String)>);
    impl crate::presets::Environment for Env {
        fn var(&self, name: &str) -> Option<std::ffi::OsString> {
            self.0
                .iter()
                .find(|(k, _)| *k == name)
                .map(|(_, v)| v.into())
        }
        fn read_text(&self, _path: &Path) -> Option<String> {
            None
        }
    }
    let dir = scratch("models");
    let env = Env(vec![("ENTH_MODELS_DIR", dir.display().to_string())]);
    let paths = create::bank_paths(&env, crate::presets::Os::Linux);
    assert_eq!(paths.last(), Some(&dir.join(create::BANK_FILE)));
    #[cfg(unix)]
    assert_eq!(paths.len(), 2, "beside this binary first: {paths:?}");
    let home = Env(vec![("HOME", "/home/someone".to_owned())]);
    let p = create::models_dir(&home, crate::presets::Os::MacOs).unwrap();
    assert_eq!(p, Path::new("/home/someone/Documents/enth/Models"));
    // Missing: a note, no bank.
    let (bank, notes) = create::load_bank(&[dir.join("none.bin")]);
    assert!(bank.is_none());
    assert!(notes[0].contains("no k-NN bank"), "{notes:?}");
    // Refused: a note naming it.
    let junk = dir.join(create::BANK_FILE);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(&junk, b"ENTHBANK but not really").unwrap();
    let (bank, notes) = create::load_bank(std::slice::from_ref(&junk));
    assert!(bank.is_none());
    assert!(notes[0].contains("refused"), "{notes:?}");
    let _ = std::fs::remove_file(junk);
}

#[test]
fn the_audition_player_plays_once_and_hands_every_clip_back() {
    let slot = Arc::new(AuditionSlot::default());
    let mut p = AuditionPlayer::new(Arc::clone(&slot));
    let (mut l, mut r) = (vec![0.0f32; 4], vec![0.0f32; 4]);
    p.mix(&mut l, &mut r);
    assert!(!p.playing());
    slot.publish(Box::new(AuditionClip::new(vec![1.0; 6], vec![2.0; 6])));
    p.mix(&mut l, &mut r);
    assert_eq!((l.clone(), r.clone()), (vec![1.0; 4], vec![2.0; 4]));
    l.fill(0.0);
    r.fill(0.0);
    p.mix(&mut l, &mut r);
    assert_eq!(l, vec![1.0, 1.0, 0.0, 0.0], "the rest, then nothing");
    assert!(!p.playing());
    // A clip replaced mid-play: the newer one plays from its start.
    slot.publish(Box::new(AuditionClip::new(vec![1.0; 100], vec![1.0; 100])));
    p.mix(&mut l, &mut r);
    slot.publish(Box::new(AuditionClip::new(vec![3.0; 4], vec![3.0; 4])));
    l.fill(0.0);
    p.mix(&mut l, &mut r);
    assert_eq!(l, vec![3.0; 4], "the newer clip plays");
    // It ended while the retired slot still held the first: it waits on the audio side, and nothing is freed there.
    l.fill(0.0);
    p.mix(&mut l, &mut r);
    assert_eq!(l, vec![0.0; 4]);
    assert!(!p.playing());
    slot.collect();
    p.mix(&mut l, &mut r);
    slot.collect();
    // The stop clip ends it.
    slot.publish(Box::default());
    l.fill(0.0);
    p.mix(&mut l, &mut r);
    p.mix(&mut l, &mut r);
    assert!(!p.playing());
    assert_eq!(l, vec![0.0; 4]);
    slot.collect();
}

#[test]
fn workers_are_half_the_cpus_at_most_eight() {
    let n = create::search_workers();
    assert!((1..=8).contains(&n));
    let cpus = std::thread::available_parallelism().map_or(2, std::num::NonZero::get);
    assert_eq!(n, (cpus / 2).clamp(1, 8));
}

#[test]
fn a_finished_view_in_the_session_lets_use_this_load_the_best() {
    let _serial = SERIAL.lock();
    // The posts of a real job, handed to a session, then Use this.
    let c = quiet();
    let mut s = enth_session::Session::new(crate::init::patch());
    let id = s
        .create_start(
            CreateSource::file(fixture("tone.wav")),
            CreateSettings {
                candidates: 2,
                budget: Some(40),
                ..CreateSettings::default()
            },
        )
        .unwrap();
    c.act(s.drain_create_requests().collect());
    let posts = collect(&c, Duration::from_secs(120), ended);
    for (pid, post) in posts {
        s.create_post(pid, post).unwrap();
    }
    let v = s.create().unwrap();
    assert_eq!((v.id, v.phase), (id, CreatePhase::Done(CreateStop::Budget)));
    let best = v.best().unwrap();
    s.create_use(best, Which::Refined).unwrap();
    assert_eq!(s.patch().meta.name, "tone");
    assert!(s.history().can_undo());
}

// --- The A/B against the upload (lane PUI, additive) ------------------------------------------------------------

/// Everything `player` plays from now until it has played and stopped (or 30 s pass), left channel then right.
fn played(player: &mut AuditionPlayer) -> (Vec<f32>, Vec<f32>) {
    let t0 = Instant::now();
    let (mut left, mut right) = (Vec::new(), Vec::new());
    let (mut l, mut r) = (vec![0.0f32; 512], vec![0.0f32; 512]);
    let mut started = false;
    while t0.elapsed() < Duration::from_secs(30) {
        l.fill(0.0);
        r.fill(0.0);
        player.mix(&mut l, &mut r);
        if player.playing() {
            started = true;
            left.extend_from_slice(&l);
            right.extend_from_slice(&r);
        } else if started {
            left.extend_from_slice(&l);
            right.extend_from_slice(&r);
            break;
        } else {
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    (left, right)
}

fn rms_of(x: &[f32]) -> f64 {
    (x.iter().map(|v| f64::from(*v) * f64::from(*v)).sum::<f64>() / x.len().max(1) as f64).sqrt()
}

#[test]
fn the_upload_plays_level_matched_against_the_candidate_played_last() {
    let _serial = SERIAL.lock();
    let c = quiet();
    c.act(vec![run(1, "tone.wav", 120.0, Some(60), 2)]);
    let posts = collect(&c, Duration::from_secs(120), ended);
    let Some((_, CreatePost::Done { candidates, .. })) = posts.last() else {
        panic!("ends with Done: {:?}", posts.last());
    };
    c.set_host_rate(44_100.0);
    let slot = c.audition_slot();
    let mut player = AuditionPlayer::new(Arc::clone(&slot));
    // A candidate, then the upload: the louder of the two comes down to the quieter one's RMS.
    let best = candidates
        .iter()
        .min_by(|a, b| a.distance.total_cmp(&b.distance))
        .unwrap();
    for patch in [best.patch.clone(), {
        // The same candidate 24 dB quieter: it plays as it is, and the upload comes down to its level.
        let mut p = best.patch.clone();
        let master: enth_patch::ParamAddr = "master.level".parse().unwrap();
        let level = p.value(&master).unwrap_or(0.0);
        enth_patch::apply_all(
            &mut p,
            &[enth_patch::Op::SetParam {
                addr: master,
                value: level - 24.0,
            }],
        )
        .unwrap();
        p
    }] {
        c.act(vec![CreateRequest::Audition {
            id: CreateJobId(1),
            branch: best.branch,
            which: Which::Refined,
            patch: Box::new(patch),
        }]);
        let (cl, cr) = played(&mut player);
        assert!(rms_of(&cl) > 0.0, "the candidate played");
        c.act(vec![CreateRequest::AuditionUpload { id: CreateJobId(1) }]);
        let (ul, ur) = played(&mut player);
        assert_eq!(ul, ur, "the upload is mono on both channels");
        // The scored clip at 44.1 kHz is as long as the candidate's render of it (to a 512-frame block).
        assert!(
            ul.len().abs_diff(cl.len()) <= 512,
            "upload {} frames, candidate {}",
            ul.len(),
            cl.len()
        );
        let (c_rms, u_rms) = (rms_of(&[cl, cr].concat()), rms_of(&ul));
        println!("candidate RMS {c_rms:.5}, upload RMS {u_rms:.5}");
        assert!(
            (c_rms / u_rms - 1.0).abs() < 0.05,
            "level-matched: candidate {c_rms}, upload {u_rms}"
        );
    }
    // A stale job's upload plays nothing.
    c.act(vec![CreateRequest::AuditionUpload { id: CreateJobId(9) }]);
    std::thread::sleep(Duration::from_millis(50));
    let (mut l, mut r) = (vec![0.0f32; 512], vec![0.0f32; 512]);
    player.mix(&mut l, &mut r);
    assert!(!player.playing() && l.iter().all(|v| *v == 0.0));
    let _ = c.take_posts();
}

#[test]
fn the_level_match_never_amplifies_and_the_resampler_keeps_the_length() {
    assert_eq!(create::match_gain(0.5, 0.25), 0.5);
    assert_eq!(create::match_gain(0.25, 0.5), 1.0, "never louder");
    assert_eq!(create::match_gain(0.0, 0.5), 1.0);
    assert_eq!(
        create::match_gain(0.5, 0.0),
        1.0,
        "a silent side changes nothing"
    );
    let x: Vec<f32> = (0..48_000).map(|i| (i as f32 / 48.0).sin()).collect();
    let y = create::resample_linear(&x, 48_000, 44_100);
    assert_eq!(y.len(), 44_100);
    assert_eq!(create::resample_linear(&x, 48_000, 48_000), x);
    let up = create::resample_linear(&[0.0, 1.0], 1, 2);
    assert_eq!(up, [0.0, 0.5, 1.0, 1.0]);
}

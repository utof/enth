//! `bursts` and `note-end`: how a plugin copes with event floods, and whether its `NOTE_END` reports are complete.
//!
//! Both are about the event lists, not the sound. nice-plug #87 says "frame count doesn't bound event count":
//! a host may put thousands of events into a 64-frame block, and a plugin that copies them into a fixed
//! queue (`INPUT_EVENT_CAPACITY` in `enth-plugin`) must do something defined when it is full. CLAP
//! says a plugin sends `NOTE_END` when a voice is done, with the note tuple the host gave in the note-on, from
//! inside `process()`, in time order. Host-side voice tracking (and polyphonic modulation) leaks a voice for
//! every note that never ends.
//!
//! # `bursts`
//!
//! For each event count N, one instance plays a held chord for a few blocks, then **one block** holding N events
//! (note-on and note-off pairs, parameter changes, or a mix of the two, spread over the block's frames, so many
//! share a frame), then a few ordinary blocks, a probe note, and a tail long enough for releases to finish. Checks:
//!
//! - **process**: no `process()` error (a plugin that panics or aborts takes the host down: that is a failure
//!   too, with the signal as exit status);
//! - **finite**: no NaN or infinite sample;
//! - **deterministic**: a second fresh instance gives the same samples and the same `NOTE_END` list;
//! - **recovered**: the probe note after the burst sounds and ends, so the event path was not left wedged;
//! - **`NOTE_END`** ([`check_note_ends`]): every note the host started ended exactly once, with the right key,
//!   inside the block, in time order;
//! - **applied**: the same events, fed one frame per block (where no queue can overflow unless more than
//!   `--capacity` events share one frame), give the same samples. If so the plugin applied every event; if not it
//!   dropped some. With `--capacity C`, N events with N <= C **must** all be applied, and the report says whether
//!   a larger N was dropped, which is the plugin's documented behaviour, not a failure.
//!
//! # `note-end`
//!
//! Plays a notes script at several host block sizes, with a tail, and runs [`check_note_ends`] on what the plugin
//! sent back. The script should include steals (more notes than voices), zero-length notes and notes that end
//! between blocks, which is where `NOTE_END` bugs live.
//!
//! **Negative controls** (`--inject`): `bursts` takes `drop-after=K` (the host delivers only K events of a block,
//! as a plugin with a K-event queue would see them) and `note-end` takes `drop-end=K` (every K-th `NOTE_END` is
//! hidden from the check) and `late-end=F` (each `NOTE_END` is moved F frames later). Each must fail the check
//! it targets, or the check cannot fail.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::CStr;

use clack_host::prelude::PluginEntry;
use serde_json::{Value, json};

use crate::notes::{self, Kind, Schedule, Scheduled};
use crate::query::{self, ParamDesc};
use crate::render::{
    OutEvent, OutRecord, ProcessThread, RenderConfig, Session, hash_samples, run_job_ext, stats,
};

/// A note the host started, for [`check_note_ends`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SentNote {
    pub id: u32,
    pub key: u16,
    /// Absolute frame of the note-on.
    pub on_frame: u64,
}

/// The notes a schedule starts.
pub fn sent_notes(schedule: &Schedule) -> Vec<SentNote> {
    schedule
        .events
        .iter()
        .filter_map(|e| match e.kind {
            Kind::NoteOn { key, note_id, .. } => Some(SentNote {
                id: note_id,
                key,
                on_frame: e.frame,
            }),
            _ => None,
        })
        .collect()
}

/// What [`check_note_ends`] found.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NoteEndReport {
    pub sent: usize,
    /// `NOTE_END` events received.
    pub received: usize,
    /// Notes that ended exactly once.
    pub ended_once: usize,
    /// Note ids that never ended.
    pub unended: Vec<u32>,
    /// Note ids that ended more than once.
    pub duplicated: Vec<u32>,
    /// `NOTE_END`s for a note id the host never started, or with the wildcard id.
    pub unknown: usize,
    /// `NOTE_END`s whose offset is not inside the block (`time >= block`).
    pub outside_block: usize,
    /// `NOTE_END`s earlier than their note-on.
    pub premature: usize,
    /// `NOTE_END`s whose key is not the note-on's.
    pub key_mismatch: usize,
    /// Blocks in which the plugin's output events are not in time order.
    pub unsorted_blocks: usize,
}

impl NoteEndReport {
    /// Whether every check passed. `allow_unended` accepts notes that never ended (a plugin that documents it).
    pub fn ok(&self, allow_unended: bool) -> bool {
        (allow_unended || self.unended.is_empty())
            && self.duplicated.is_empty()
            && self.unknown == 0
            && self.outside_block == 0
            && self.premature == 0
            && self.key_mismatch == 0
            && self.unsorted_blocks == 0
    }

    pub fn to_json(&self) -> Value {
        let head = |v: &[u32]| v.iter().take(8).copied().collect::<Vec<_>>();
        json!({
            "notes_sent": self.sent,
            "note_end_events": self.received,
            "ended_once": self.ended_once,
            "unended": self.unended.len(),
            "unended_first": head(&self.unended),
            "duplicated": self.duplicated.len(),
            "duplicated_first": head(&self.duplicated),
            "unknown": self.unknown,
            "outside_block": self.outside_block,
            "premature": self.premature,
            "key_mismatch": self.key_mismatch,
            "unsorted_blocks": self.unsorted_blocks,
        })
    }

    /// One line per failed check.
    pub fn failures(&self, allow_unended: bool) -> Vec<String> {
        let mut out = Vec::new();
        if !allow_unended && !self.unended.is_empty() {
            out.push(format!(
                "{} of {} notes never got a NOTE_END (first ids {:?})",
                self.unended.len(),
                self.sent,
                self.unended.iter().take(5).collect::<Vec<_>>()
            ));
        }
        for (count, what) in [
            (self.duplicated.len(), "notes ended more than once"),
            (
                self.unknown,
                "NOTE_ENDs for a note the host never started (or the wildcard id)",
            ),
            (
                self.outside_block,
                "NOTE_ENDs with an offset past the end of their block",
            ),
            (self.premature, "NOTE_ENDs before their note-on"),
            (
                self.key_mismatch,
                "NOTE_ENDs with a different key than the note-on",
            ),
            (
                self.unsorted_blocks,
                "blocks whose output events are not in time order",
            ),
        ] {
            if count > 0 {
                out.push(format!("{count} {what}"));
            }
        }
        out
    }
}

/// Checks the `NOTE_END`s a plugin sent back against the notes the host started.
///
/// `block` is the host block size: offsets are checked against it, absolute frames are
/// `record.block * block + record.time`.
pub fn check_note_ends(sent: &[SentNote], block: u32, records: &[OutRecord]) -> NoteEndReport {
    let by_id: BTreeMap<u32, &SentNote> = sent.iter().map(|n| (n.id, n)).collect();
    let mut report = NoteEndReport {
        sent: sent.len(),
        ..NoteEndReport::default()
    };
    let mut ends: BTreeMap<u32, usize> = BTreeMap::new();
    let mut last_time: Option<(u64, u32)> = None;
    let mut unsorted: BTreeSet<u64> = BTreeSet::new();
    for r in records {
        if let Some((b, t)) = last_time
            && b == r.block
            && r.time < t
        {
            unsorted.insert(r.block);
        }
        last_time = Some((r.block, r.time));
        let OutEvent::NoteEnd { key, note_id, .. } = &r.event else {
            continue;
        };
        report.received += 1;
        if r.time >= block {
            report.outside_block += 1;
        }
        let Some(id) = note_id else {
            report.unknown += 1;
            continue;
        };
        let Some(note) = by_id.get(id) else {
            report.unknown += 1;
            continue;
        };
        *ends.entry(*id).or_insert(0) += 1;
        if r.block * u64::from(block) + u64::from(r.time) < note.on_frame {
            report.premature += 1;
        }
        if key.is_some_and(|k| k != note.key) {
            report.key_mismatch += 1;
        }
    }
    report.unsorted_blocks = unsorted.len();
    for note in sent {
        match ends.get(&note.id).copied().unwrap_or(0) {
            0 => report.unended.push(note.id),
            1 => report.ended_once += 1,
            _ => report.duplicated.push(note.id),
        }
    }
    report
}

// ---------------------------------------------------------------------------------------------------------------
// Bursts

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BurstKind {
    Mixed,
    Notes,
    Params,
}

impl BurstKind {
    pub fn parse(text: &str) -> Result<Self, String> {
        match text {
            "mixed" => Ok(Self::Mixed),
            "notes" => Ok(Self::Notes),
            "params" => Ok(Self::Params),
            other => Err(format!("--kind {other:?}: use mixed, notes or params")),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Mixed => "mixed",
            Self::Notes => "notes",
            Self::Params => "params",
        }
    }
}

/// Which of a note event and a parameter event comes first in the list when they share a frame. CLAP only requires
/// the list to be sorted by time; hosts differ, and a plugin that splits its block at parameter events (nice-plug's
/// `SAMPLE_ACCURATE_AUTOMATION`) can lose a note event that sits right before a parameter event on the same frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SameFrame {
    NotesFirst,
    ParamsFirst,
}

impl SameFrame {
    pub fn name(self) -> &'static str {
        match self {
            Self::NotesFirst => "notes-first",
            Self::ParamsFirst => "params-first",
        }
    }

    /// `notes-first`, `params-first` or `both`.
    pub fn parse_list(text: &str) -> Result<Vec<Self>, String> {
        match text {
            "notes-first" => Ok(vec![Self::NotesFirst]),
            "params-first" => Ok(vec![Self::ParamsFirst]),
            "both" => Ok(vec![Self::NotesFirst, Self::ParamsFirst]),
            other => Err(format!(
                "--order {other:?}: use notes-first, params-first or both"
            )),
        }
    }
}

/// A deterministic generator: the same N always gives the same burst.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.0 >> 33
    }
}

const CHORD_ID: u32 = 1000;
const PROBE_ID: u32 = 5000;
const BURST_ID: u32 = 10_000;

/// The parameters a burst may move: automatable, writable, with a finite range.
pub fn burst_params(params: &[ParamDesc]) -> Vec<&ParamDesc> {
    params
        .iter()
        .filter(|p| {
            p.flags.contains(&"IS_AUTOMATABLE")
                && !p.flags.contains(&"IS_READONLY")
                && !p.flags.contains(&"IS_HIDDEN")
                && p.min.is_finite()
                && p.max.is_finite()
                && p.max > p.min
        })
        .take(16)
        .collect()
}

/// A burst scenario laid out in blocks.
#[derive(Clone, Debug)]
pub struct Burst {
    pub schedule: Schedule,
    pub sent: Vec<SentNote>,
    pub blocks: u64,
    pub burst_block: u64,
    pub probe_block: u64,
    /// Events in the burst block.
    pub events: usize,
    pub note_ons: usize,
    pub param_events: usize,
    /// The most events any single frame holds (what a 1-frame-block reference has to queue).
    pub max_per_frame: usize,
}

/// Lays out a burst of `n` events in block `warm`, for host block size `block`.
///
/// Block 0..warm: a held 3-note chord. Block `warm`: the burst. The chord is released two blocks later; the probe
/// note starts `after` blocks after the burst and lasts four blocks; `tail_blocks` more blocks follow so releases end.
#[allow(
    clippy::too_many_arguments,
    reason = "a burst is described by its shape; a struct would only rename the arguments"
)]
pub fn build_burst(
    n: usize,
    kind: BurstKind,
    block: u32,
    params: &[&ParamDesc],
    warm: u64,
    after: u64,
    tail_blocks: u64,
    order: SameFrame,
) -> Burst {
    let b = u64::from(block);
    let burst_block = warm;
    let probe_block = burst_block + 1 + after;
    let blocks = probe_block + 4 + tail_blocks;
    let mut events: Vec<Scheduled> = Vec::with_capacity(n + 16);
    // The chord.
    for (i, key) in [48u16, 55, 64].into_iter().enumerate() {
        events.push(Scheduled {
            frame: 0,
            kind: Kind::NoteOn {
                key,
                note_id: CHORD_ID + i as u32,
                velocity: 0.75,
            },
        });
        events.push(Scheduled {
            frame: (burst_block + 2) * b,
            kind: Kind::NoteOff {
                key,
                note_id: CHORD_ID + i as u32,
            },
        });
    }
    // The burst: event e at frame burst_block * b + e * b / n, so up to ceil(n / b) events share a frame.
    let kind = if params.is_empty() && kind != BurstKind::Notes {
        BurstKind::Notes
    } else {
        kind
    };
    let mut rng = Lcg(0x0E17_5EED ^ n as u64);
    let mut note_index = 0u32;
    let mut open: Vec<(u16, u32)> = Vec::new();
    let mut param_events = 0usize;
    let mut note_ons = 0usize;
    let mut late_offs: Vec<(u16, u32)> = Vec::new();
    let n = if kind == BurstKind::Notes {
        n + n % 2
    } else {
        n
    };
    for e in 0..n {
        let frame = burst_block * b + (e as u64 * b) / n as u64;
        let note_on = |index: u32, rng: &mut Lcg| {
            let key = 24 + (rng.next() % 80) as u16;
            (
                key,
                BURST_ID + index,
                f64::from(1 + (rng.next() % 8) as u32) / 8.0,
            )
        };
        let param = |rng: &mut Lcg, events: &mut Vec<Scheduled>| {
            let p = params[(rng.next() as usize) % params.len()];
            let u = f64::from((rng.next() % 1025) as u32) / 1024.0;
            events.push(Scheduled {
                frame,
                kind: Kind::Param {
                    id: p.id,
                    value: p.min + (p.max - p.min) * u,
                },
            });
        };
        match kind {
            BurstKind::Notes => {
                if e % 2 == 0 {
                    let (key, id, velocity) = note_on(note_index, &mut rng);
                    note_index += 1;
                    note_ons += 1;
                    open.push((key, id));
                    events.push(Scheduled {
                        frame,
                        kind: Kind::NoteOn {
                            key,
                            note_id: id,
                            velocity,
                        },
                    });
                } else if let Some((key, id)) = open.pop() {
                    events.push(Scheduled {
                        frame,
                        kind: Kind::NoteOff { key, note_id: id },
                    });
                }
            }
            BurstKind::Params => {
                param(&mut rng, &mut events);
                param_events += 1;
            }
            BurstKind::Mixed => match e % 3 {
                0 => {
                    let (key, id, velocity) = note_on(note_index, &mut rng);
                    note_index += 1;
                    note_ons += 1;
                    open.push((key, id));
                    events.push(Scheduled {
                        frame,
                        kind: Kind::NoteOn {
                            key,
                            note_id: id,
                            velocity,
                        },
                    });
                }
                1 => {
                    param(&mut rng, &mut events);
                    param_events += 1;
                }
                _ => {
                    if let Some((key, id)) = open.pop() {
                        events.push(Scheduled {
                            frame,
                            kind: Kind::NoteOff { key, note_id: id },
                        });
                    }
                }
            },
        }
    }
    // Notes still open are released at the start of the next block.
    late_offs.append(&mut open);
    for (key, id) in late_offs {
        events.push(Scheduled {
            frame: (burst_block + 1) * b,
            kind: Kind::NoteOff { key, note_id: id },
        });
    }
    // The probe.
    events.push(Scheduled {
        frame: probe_block * b + 5,
        kind: Kind::NoteOn {
            key: 69,
            note_id: PROBE_ID,
            velocity: 0.75,
        },
    });
    events.push(Scheduled {
        frame: (probe_block + 4) * b + 5,
        kind: Kind::NoteOff {
            key: 69,
            note_id: PROBE_ID,
        },
    });
    events.retain(|e| e.frame < blocks * b);
    let params_first = order == SameFrame::ParamsFirst;
    events.sort_by_key(|e| {
        (
            e.frame,
            matches!(e.kind, Kind::Param { .. }) != params_first,
        )
    });
    let mut per_frame: BTreeMap<u64, usize> = BTreeMap::new();
    for e in &events {
        *per_frame.entry(e.frame).or_insert(0) += 1;
    }
    let schedule = Schedule { events };
    let burst_events = schedule
        .events
        .iter()
        .filter(|e| {
            e.frame >= burst_block * b
                && e.frame < (burst_block + 1) * b
                && !matches!(
                    e.kind,
                    Kind::NoteOn {
                        note_id: CHORD_ID..=2999,
                        ..
                    }
                )
        })
        .count();
    Burst {
        sent: sent_notes(&schedule),
        schedule,
        blocks,
        burst_block,
        probe_block,
        events: burst_events,
        note_ons,
        param_events,
        max_per_frame: per_frame.values().copied().max().unwrap_or(0),
    }
}

/// What one run of a schedule through a fresh instance produced.
pub struct Trial {
    pub hash: String,
    pub peak: f32,
    pub non_finite: usize,
    /// Peak of the probe note's first four blocks.
    pub probe_peak: f32,
    pub records: Vec<OutRecord>,
    pub process_ns: Vec<u64>,
    pub channels: usize,
}

/// Runs `schedule` for `blocks` blocks of `block` frames through a new instance.
#[allow(
    clippy::too_many_arguments,
    reason = "one trial is one render of one schedule; the arguments are its inputs"
)]
pub fn run_trial(
    entry: &PluginEntry,
    id: &CStr,
    state: Option<&[u8]>,
    schedule: &Schedule,
    block: u32,
    blocks: u64,
    sample_rate: f64,
    inject_event_cap: Option<usize>,
    probe: Option<(u64, u64)>,
) -> Result<Trial, String> {
    let config = RenderConfig {
        sample_rate,
        block,
        frames: blocks * u64::from(block),
    };
    let mut session = Session::open_bare(entry, id, state, config)?;
    session.schedule = schedule.clone();
    let processor = session.activate()?;
    let mut setup = session.setup(processor);
    setup.capture_events = true;
    setup.record_timing = true;
    setup.inject_event_cap = inject_event_cap;
    let out = run_job_ext(setup, ProcessThread::Dedicated)?;
    session.close(out.processor);
    let s = stats(&out.samples, out.channels);
    let probe_peak = probe.map_or(0.0, |(from, to)| {
        let lo = (from as usize * out.channels).min(out.samples.len());
        let hi = (to as usize * out.channels).min(out.samples.len());
        out.samples[lo..hi]
            .iter()
            .filter(|x| x.is_finite())
            .fold(0.0f32, |m, x| m.max(x.abs()))
    });
    Ok(Trial {
        hash: hash_samples(&out.samples),
        peak: s.peak,
        non_finite: s.non_finite,
        probe_peak,
        records: out.output_events,
        process_ns: out.process_ns,
        channels: out.channels,
    })
}

/// A fault the host makes on purpose.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Inject {
    /// `bursts`: deliver only this many events of each block.
    pub drop_after: Option<usize>,
    /// `note-end`: hide every K-th `NOTE_END`.
    pub drop_end: Option<usize>,
    /// `note-end`: move every `NOTE_END` this many frames later.
    pub late_end: Option<u32>,
}

impl Inject {
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut inject = Self::default();
        for part in text.split(',').map(str::trim).filter(|p| !p.is_empty()) {
            let (name, value) = part
                .split_once('=')
                .ok_or_else(|| format!("--inject {part:?}: expected NAME=NUMBER"))?;
            let n: u32 = value
                .parse()
                .map_err(|_| format!("--inject {part:?}: {value:?} is not a number"))?;
            match name {
                "drop-after" => inject.drop_after = Some(n as usize),
                "drop-end" => inject.drop_end = Some((n as usize).max(1)),
                "late-end" => inject.late_end = Some(n),
                other => {
                    return Err(format!(
                        "--inject {other:?}: use drop-after=K (bursts), drop-end=K or late-end=F (note-end)"
                    ));
                }
            }
        }
        Ok(inject)
    }

    /// Applies the `NOTE_END` faults to what the plugin sent.
    pub fn alter_records(&self, records: &[OutRecord]) -> Vec<OutRecord> {
        let mut seen = 0usize;
        let mut out = Vec::with_capacity(records.len());
        for r in records {
            let mut r = r.clone();
            if matches!(r.event, OutEvent::NoteEnd { .. }) {
                seen += 1;
                if self.drop_end.is_some_and(|k| seen.is_multiple_of(k)) {
                    continue;
                }
                if let Some(late) = self.late_end {
                    r.time += late;
                }
            }
            out.push(r);
        }
        out
    }
}

#[derive(Clone, Debug)]
pub struct BurstOptions {
    pub counts: Vec<usize>,
    pub kind: BurstKind,
    pub block: u32,
    pub sample_rate: f64,
    pub capacity: Option<usize>,
    pub state: Option<Vec<u8>>,
    pub warm_blocks: u64,
    pub after_blocks: u64,
    pub tail_s: f64,
    pub allow_unended: bool,
    pub inject: Inject,
    /// Same-frame orders to try for every count.
    pub orders: Vec<SameFrame>,
}

fn median(values: &[u64]) -> Option<u64> {
    let mut v = values.to_vec();
    v.sort_unstable();
    v.get(v.len() / 2).copied()
}

fn records_key(records: &[OutRecord]) -> Vec<(u64, u32, Option<u32>)> {
    records
        .iter()
        .map(|r| {
            (
                r.block,
                r.time,
                match r.event {
                    OutEvent::NoteEnd { note_id, .. } => note_id,
                    OutEvent::Other { type_id } => Some(u32::from(type_id) | 0x8000_0000),
                },
            )
        })
        .collect()
}

/// Runs the bursts of `opts.counts`, one report per count.
pub fn run_bursts(entry: &PluginEntry, id: &CStr, opts: &BurstOptions) -> Result<Value, String> {
    let params = {
        let config = RenderConfig::new(opts.sample_rate, opts.block, 0.1)?;
        Session::open_bare(entry, id, opts.state.as_deref(), config)?.params
    };
    let usable = burst_params(&params);
    let tail_blocks = (opts.tail_s * opts.sample_rate / f64::from(opts.block)).ceil() as u64;
    let mut trials = Vec::new();
    let mut all_ok = true;
    for &n in &opts.counts {
        for &order in &opts.orders {
            let burst = build_burst(
                n,
                opts.kind,
                opts.block,
                &usable,
                opts.warm_blocks,
                opts.after_blocks,
                tail_blocks,
                order,
            );
            let b = u64::from(opts.block);
            let probe = Some((burst.probe_block * b, (burst.probe_block + 4) * b));
            let mut failures: Vec<String> = Vec::new();

            let first = run_trial(
                entry,
                id,
                opts.state.as_deref(),
                &burst.schedule,
                opts.block,
                burst.blocks,
                opts.sample_rate,
                opts.inject.drop_after,
                probe,
            );
            let first = match first {
                Ok(t) => t,
                Err(e) => {
                    failures.push(format!("process: {e}"));
                    trials.push(json!({ "events": n, "order": order.name(), "ok": false, "failures": failures }));
                    all_ok = false;
                    continue;
                }
            };
            let second = run_trial(
                entry,
                id,
                opts.state.as_deref(),
                &burst.schedule,
                opts.block,
                burst.blocks,
                opts.sample_rate,
                opts.inject.drop_after,
                probe,
            )?;
            // The same events one frame at a time: nothing to overflow unless many share a frame.
            let reference = run_trial(
                entry,
                id,
                opts.state.as_deref(),
                &burst.schedule,
                1,
                burst.blocks * b,
                opts.sample_rate,
                None,
                probe,
            )?;

            let finite = first.non_finite == 0;
            let deterministic = first.hash == second.hash
                && records_key(&first.records) == records_key(&second.records);
            let note_ends = check_note_ends(&burst.sent, opts.block, &first.records);
            let probe_ended = !note_ends.unended.contains(&PROBE_ID);
            let recovered = first.probe_peak > 1e-5 && probe_ended;
            let applied_all = first.hash == reference.hash;
            let reference_valid = opts.capacity.is_none_or(|c| burst.max_per_frame <= c);
            let must_apply = opts.capacity.is_some_and(|c| burst.events <= c) && reference_valid;
            if !finite {
                failures.push(format!("{} non-finite samples", first.non_finite));
            }
            if !deterministic {
                failures.push("two fresh instances gave different samples or NOTE_ENDs".to_owned());
            }
            if !recovered {
                failures.push(format!(
                "after the burst the probe note did not sound and end (probe peak {}, ended: {probe_ended})",
                first.probe_peak
            ));
            }
            failures.extend(note_ends.failures(opts.allow_unended));
            if must_apply && !applied_all {
                failures.push(format!(
                "{} events are within the declared capacity {} but were not all applied (the render differs from the same events fed one frame at a time)",
                burst.events,
                opts.capacity.unwrap_or(0)
            ));
            }
            let burst_us = first
                .process_ns
                .get(burst.burst_block as usize)
                .map(|ns| *ns as f64 / 1000.0);
            let typical_us = median(&first.process_ns).map(|ns| ns as f64 / 1000.0);
            let ok = failures.is_empty();
            all_ok &= ok;
            trials.push(json!({
                "events": burst.events,
                "requested": n,
                "order": order.name(),
                "kind": opts.kind.name(),
                "note_ons": burst.note_ons,
                "param_events": burst.param_events,
                "max_events_per_frame": burst.max_per_frame,
                "blocks": burst.blocks,
                "finite": finite,
                "peak": first.peak,
                "deterministic": deterministic,
                "recovered": recovered,
                "probe_peak": first.probe_peak,
                "applied_all": applied_all,
                "applied_all_required": must_apply,
                "reference_valid": reference_valid,
                "note_end": note_ends.to_json(),
                "burst_block_us": burst_us,
                "typical_block_us": typical_us,
                "ok": ok,
                "failures": failures,
            }));
        }
    }
    // The largest count whose events were all applied, among the counts tried, for each same-frame order: the
    // plugin's observable capacity.
    let applied_up_to: serde_json::Map<String, Value> = opts
        .orders
        .iter()
        .map(|order| {
            let max = trials
                .iter()
                .filter(|t| t["order"] == order.name())
                .filter(|t| t["applied_all"] == true && t["reference_valid"] != false)
                .filter_map(|t| t["events"].as_u64())
                .max();
            (order.name().to_owned(), json!(max))
        })
        .collect();
    Ok(json!({
        "block": opts.block,
        "sample_rate": opts.sample_rate,
        "kind": opts.kind.name(),
        "declared_capacity": opts.capacity,
        "params_available": usable.len(),
        "inject": opts.inject.drop_after.map(|k| format!("drop-after={k}")),
        "applied_all_up_to_events": applied_up_to,
        "orders": opts.orders.iter().map(|o| o.name()).collect::<Vec<_>>(),
        "trials": trials,
        "ok": all_ok,
    }))
}

// ---------------------------------------------------------------------------------------------------------------
// note-end

#[derive(Clone, Debug)]
pub struct NoteEndOptions {
    pub script: String,
    pub blocks: Vec<u32>,
    pub sample_rate: f64,
    pub tail_s: f64,
    pub state: Option<Vec<u8>>,
    pub allow_unended: bool,
    pub inject: Inject,
}

pub fn run_note_end(
    entry: &PluginEntry,
    id: &CStr,
    opts: &NoteEndOptions,
) -> Result<Value, String> {
    let params = {
        let config = RenderConfig::new(opts.sample_rate, 64, 0.1)?;
        Session::open_bare(entry, id, opts.state.as_deref(), config)?.params
    };
    let resolve = |name: &str| query::resolve_param(&params, name);
    // Parse once at a huge length to learn where the script ends, then size the render to that plus the tail.
    let probe = notes::parse(&opts.script, opts.sample_rate, u64::MAX, &resolve)?;
    let last = probe.events.iter().map(|e| e.frame).max().unwrap_or(0);
    let wanted = last + (opts.tail_s * opts.sample_rate).ceil() as u64;
    let mut per_block = Vec::new();
    let mut ok = true;
    let mut ended_sets: Vec<BTreeSet<u32>> = Vec::new();
    for &block in &opts.blocks {
        let b = u64::from(block);
        let blocks = wanted.div_ceil(b);
        let schedule = Schedule {
            events: probe
                .events
                .iter()
                .filter(|e| e.frame < blocks * b)
                .cloned()
                .collect(),
        };
        let sent = sent_notes(&schedule);
        let trial = run_trial(
            entry,
            id,
            opts.state.as_deref(),
            &schedule,
            block,
            blocks,
            opts.sample_rate,
            None,
            None,
        )?;
        let records = opts.inject.alter_records(&trial.records);
        let report = check_note_ends(&sent, block, &records);
        let failures = report.failures(opts.allow_unended);
        let block_ok = failures.is_empty() && trial.non_finite == 0;
        ok &= block_ok;
        ended_sets.push(
            sent.iter()
                .map(|n| n.id)
                .filter(|id| !report.unended.contains(id))
                .collect(),
        );
        per_block.push(json!({
            "block": block,
            "blocks": blocks,
            "note_end": report.to_json(),
            "non_finite_samples": trial.non_finite,
            "ok": block_ok,
            "failures": failures,
        }));
    }
    let same_set = ended_sets.windows(2).all(|w| w[0] == w[1]);
    if !same_set {
        ok = false;
    }
    Ok(json!({
        "sample_rate": opts.sample_rate,
        "tail_s": opts.tail_s,
        "notes": probe.note_count(),
        "inject": {
            "drop_end": opts.inject.drop_end,
            "late_end": opts.inject.late_end,
        },
        "same_notes_end_at_every_block_size": same_set,
        "blocks": per_block,
        "ok": ok,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn end(block: u64, time: u32, id: u32, key: u16) -> OutRecord {
        OutRecord {
            block,
            time,
            event: OutEvent::NoteEnd {
                port: Some(0),
                channel: Some(0),
                key: Some(key),
                note_id: Some(id),
            },
        }
    }

    fn sent() -> Vec<SentNote> {
        vec![
            SentNote {
                id: 0,
                key: 60,
                on_frame: 0,
            },
            SentNote {
                id: 1,
                key: 64,
                on_frame: 70,
            },
        ]
    }

    #[test]
    fn a_complete_report_passes() {
        let report = check_note_ends(&sent(), 64, &[end(1, 5, 0, 60), end(2, 0, 1, 64)]);
        assert!(report.ok(false), "{:?}", report.failures(false));
        assert_eq!(report.ended_once, 2);
    }

    #[test]
    fn every_way_to_be_wrong_is_found() {
        // Missing, duplicate, unknown, past the block, premature, wrong key, out of order.
        let records = [
            end(2, 9, 1, 64),
            end(2, 3, 1, 64),  // duplicate of note 1, and out of order after time 9
            end(2, 64, 7, 60), // unknown id, and past the block
            end(0, 0, 0, 99),  // wrong key
        ];
        let r = check_note_ends(&sent(), 64, &records);
        assert_eq!(r.duplicated, [1]);
        assert_eq!(r.unknown, 1);
        assert_eq!(r.outside_block, 1);
        assert_eq!(r.key_mismatch, 1);
        assert_eq!(r.unsorted_blocks, 1);
        assert!(!r.ok(false) && !r.ok(true));
        let premature = check_note_ends(&sent(), 64, &[end(0, 3, 1, 64), end(0, 4, 0, 60)]);
        assert_eq!(premature.premature, 1, "note 1 started at frame 70");
        let missing = check_note_ends(&sent(), 64, &[end(1, 5, 0, 60)]);
        assert_eq!(missing.unended, [1]);
        assert!(!missing.ok(false));
        assert!(missing.ok(true), "allow_unended accepts a missing end");
        assert_eq!(missing.failures(false).len(), 1);
    }

    #[test]
    fn a_wildcard_note_id_cannot_be_matched() {
        let wild = OutRecord {
            block: 1,
            time: 0,
            event: OutEvent::NoteEnd {
                port: Some(0),
                channel: None,
                key: None,
                note_id: None,
            },
        };
        let r = check_note_ends(&sent(), 64, &[wild]);
        assert_eq!(r.unknown, 1);
        assert_eq!(r.unended.len(), 2);
    }

    #[test]
    fn injected_faults_change_what_the_check_sees() {
        let records = vec![end(1, 5, 0, 60), end(2, 0, 1, 64)];
        let drop = Inject::parse("drop-end=2").unwrap().alter_records(&records);
        assert_eq!(drop.len(), 1);
        let late = Inject::parse("late-end=60")
            .unwrap()
            .alter_records(&records);
        assert_eq!(check_note_ends(&sent(), 64, &late).outside_block, 1);
        assert!(Inject::parse("nope=1").is_err());
        assert!(Inject::parse("drop-after=x").is_err());
        assert_eq!(Inject::parse("drop-after=5").unwrap().drop_after, Some(5));
    }

    fn param(id: u32) -> ParamDesc {
        ParamDesc {
            index: id,
            id,
            name: format!("P{id}"),
            module: String::new(),
            min: 0.0,
            max: 1.0,
            default: 0.5,
            flags: vec!["IS_AUTOMATABLE"],
            value: Some(0.5),
            initial_name: None,
        }
    }

    #[test]
    fn a_burst_puts_all_its_events_in_one_block_in_time_order() {
        let p = [param(1), param(2)];
        let refs: Vec<&ParamDesc> = p.iter().collect();
        for kind in [BurstKind::Mixed, BurstKind::Notes, BurstKind::Params] {
            let b = build_burst(1000, kind, 64, &refs, 8, 4, 10, SameFrame::NotesFirst);
            assert!(
                b.schedule
                    .events
                    .windows(2)
                    .all(|w| w[0].frame <= w[1].frame),
                "{kind:?} is sorted"
            );
            assert!(
                b.events >= 1000,
                "{kind:?}: {} events in the burst block",
                b.events
            );
            assert!(b.max_per_frame >= 1000 / 64, "many events share a frame");
            // Every note-on has its note-off.
            let ons = b
                .schedule
                .events
                .iter()
                .filter(|e| matches!(e.kind, Kind::NoteOn { .. }))
                .count();
            let offs = b
                .schedule
                .events
                .iter()
                .filter(|e| matches!(e.kind, Kind::NoteOff { .. }))
                .count();
            assert_eq!(ons, offs, "{kind:?}");
            assert_eq!(b.sent.len(), ons);
        }
        // Without parameters a params burst becomes a notes burst.
        let none = build_burst(
            100,
            BurstKind::Params,
            64,
            &[],
            8,
            4,
            10,
            SameFrame::NotesFirst,
        );
        assert!(none.note_ons > 0 && none.param_events == 0);
        // The same count gives the same burst.
        let a = build_burst(
            500,
            BurstKind::Mixed,
            64,
            &refs,
            8,
            4,
            10,
            SameFrame::NotesFirst,
        );
        let b = build_burst(
            500,
            BurstKind::Mixed,
            64,
            &refs,
            8,
            4,
            10,
            SameFrame::NotesFirst,
        );
        assert_eq!(a.schedule, b.schedule);
    }
}

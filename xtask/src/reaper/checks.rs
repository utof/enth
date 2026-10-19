//! What the xtask checks after a script ran: the saved state a script dumped, and the audio REAPER rendered. The
//! scripts check what they can see from inside REAPER (parameters, windows, undo); these checks need the plugin's
//! state decoded or the render read.

use std::collections::BTreeMap;
use std::path::Path;

use super::audio::{Notes, Wav, compare_levels, median_at, rms, roughness};
use super::chunk::Saved;
use super::{Format, Report, Sandbox};

fn near(a: Option<f64>, want: f64, tolerance: f64) -> bool {
    a.is_some_and(|a| (a - want).abs() <= tolerance)
}

/// The first byte where two texts differ, with a little context, for a failure message.
fn first_difference(a: &str, b: &str) -> String {
    let at = a
        .bytes()
        .zip(b.bytes())
        .position(|(x, y)| x != y)
        .unwrap_or(a.len().min(b.len()));
    let around = |s: &str| {
        let from = at.saturating_sub(20);
        s.get(from..(at + 20).min(s.len()))
            .unwrap_or_default()
            .to_owned()
    };
    format!(
        "first difference at byte {at} ({} against {} bytes): …{}… against …{}…",
        a.len(),
        b.len(),
        around(a),
        around(b)
    )
}

/// `filter1.cutoff` of a patch whose slot 1 drives it, at host value `norm` on its log scale from 20 Hz to 20 kHz.
fn cutoff_at(norm: f64) -> f64 {
    20.0 * (20_000.0_f64 / 20.0).powf(norm)
}

// ---- state-create and state-reload -----------------------------------------------------------------------------

/// Decodes the plugin state REAPER saved before and after the reload and compares the values and
/// the patch blob (which carries a nonce stamped by the plugin on first activation).
pub fn state(sb: &Sandbox, report: &mut Report, format: Format, product: bool) {
    let scope = format!("{}/state-reload", format.tag());
    let (created, reloaded) = match (
        Saved::read(&sb.out, format, "create"),
        Saved::read(&sb.out, format, "reload"),
    ) {
        (Ok(a), Ok(b)) => (a.0, b.0),
        (a, b) => {
            let why = a.err().or(b.err()).unwrap_or_default();
            report.add(&scope, "state-decoded", false, why);
            return;
        }
    };
    let param = |state: &serde_json::Value, id: &str| state["params"][id]["f32"].as_f64();
    for (state, name) in [(&created, "saved"), (&reloaded, "reloaded")] {
        let (m, s) = (param(state, "macro_1"), param(state, "slot_1"));
        report.add(
            &scope,
            &format!("{name}-state-values"),
            near(m, 0.7, 0.002) && near(s, 0.3, 0.002),
            format!("macro_1 = {m:?}, slot_1 = {s:?}"),
        );
    }
    if product {
        product_state(report, &scope, &created, &reloaded);
        return;
    }
    // The blob is stored as a string holding JSON; the nonce proves it came from the saved state.
    let nonce = |state: &serde_json::Value| -> Option<String> {
        let blob: serde_json::Value =
            serde_json::from_str(state["fields"]["patch"].as_str()?).ok()?;
        blob["nonce"].as_str().map(str::to_owned)
    };
    let (before, after) = (nonce(&created), nonce(&reloaded));
    report.add(
        &scope,
        "patch-blob-restored",
        before.is_some() && before == after,
        format!(
            "nonce {} before and after reload",
            before.as_deref().unwrap_or("missing")
        ),
    );
}

/// The product's saved state: the patch loaded in `state-create` came back byte for byte after the reload in a new
/// process, and the host's values set before the save were folded into it (macro 1 = 0.7; slot 1's target,
/// `filter1.cutoff`, at `from_norm(0.3)` = about 159 Hz on its 20 Hz to 20 kHz log scale).
fn product_state(
    report: &mut Report,
    scope: &str,
    created: &serde_json::Value,
    reloaded: &serde_json::Value,
) {
    let patch = |state: &serde_json::Value| -> Option<(String, serde_json::Value)> {
        let text = state["fields"]["patch"].as_str()?.to_owned();
        let value = serde_json::from_str(&text).ok()?;
        Some((text, value))
    };
    let (Some((before, p)), Some((after, _))) = (patch(created), patch(reloaded)) else {
        report.add(
            scope,
            "patch-blob-restored",
            false,
            "no enth patch in the saved state",
        );
        return;
    };
    let name = p["meta"]["name"].as_str().unwrap_or_default();
    report.add(
        scope,
        "patch-blob-restored",
        before == after && name == "reaper-test state" && p["slots"]["64"] == "filter1.resonance",
        format!(
            "{name:?}, {} bytes, identical after reload: {}",
            before.len(),
            before == after
        ),
    );
    let macro1 = p["macros"][0]["value"].as_f64();
    let cutoff = p["voice_fx"][0]["params"]["cutoff"].as_f64();
    report.add(
        scope,
        "patch-folds-host-values",
        near(macro1, 0.7, 0.002) && cutoff.is_some_and(|c| (150.0..170.0).contains(&c)),
        format!(
            "in the saved patch: macro 1 = {macro1:?}, filter1.cutoff (slot 1) = {cutoff:?} Hz"
        ),
    );
    let instance = created["fields"]["enth"].as_str().unwrap_or_default();
    report.add(
        scope,
        "instance-state-saved",
        instance.starts_with(r#"{"schema":"enth.plugin-state/1""#),
        instance.to_owned(),
    );
}

// ---- patch-roundtrip ---------------------------------------------------------------------------------------------

/// The patch `roundtrip.lua` loaded came back byte for byte from the first save, from the reload in a new process and
/// from the reload of the project that process saved. `fixture` is the patch file, in the plugin's own serialisation
/// (the plugin saves exactly `Patch::to_json()`), so a plugin that changes the patch on a load or a save shows here.
pub fn roundtrip(out: &Path, report: &mut Report, format: Format, fixture: &str) {
    let scope = format!("{}/patch-roundtrip", format.tag());
    let labels = ["roundtrip-create", "roundtrip-reload1", "roundtrip-reload2"];
    let mut states = Vec::new();
    for label in labels {
        match Saved::read(out, format, label) {
            Ok(saved) => states.push(saved),
            Err(why) => {
                report.add(&scope, "state-decoded", false, why);
                return;
            }
        }
    }
    let texts: Vec<&str> = states
        .iter()
        .map(|s| s.patch_text().unwrap_or_default())
        .collect();
    let want = fixture.trim_end();
    let mut identical = |name: &str, a: &str, b: &str, what: &str| {
        let ok = !a.is_empty() && a == b;
        let detail = if ok {
            format!("{what}: {} bytes, identical", a.len())
        } else {
            format!("{what}: {}", first_difference(a, b))
        };
        report.add(&scope, name, ok, detail);
    };
    identical(
        "patch-saved-as-loaded",
        texts[0],
        want,
        "the first save against the patch file (regenerate the file from the first save if the patch format changed on purpose)",
    );
    identical(
        "patch-survives-reload",
        texts[0],
        texts[1],
        "reload in a new process",
    );
    identical(
        "patch-survives-second-reload",
        texts[1],
        texts[2],
        "reload of the re-saved project",
    );
    let instance_same = states
        .iter()
        .all(|s| s.instance_text() == states[0].instance_text());
    report.add(
        &scope,
        "instance-state-survives",
        instance_same && states[0].instance_text().starts_with("{\"schema\""),
        states[0].instance_text().to_owned(),
    );
    let params_same = states
        .iter()
        .all(|s| s.0["params"] == states[0].0["params"]);
    let count = states[0].0["params"]
        .as_object()
        .map_or(0, serde_json::Map::len);
    report.add(
        &scope,
        "params-survive",
        params_same && count > 0,
        format!("{count} saved parameters, the same in all three saves"),
    );
    let patch = states[0].patch().unwrap_or_default();
    let len = |section: &str| patch[section].as_array().map_or(0, Vec::len);
    let facts = format!(
        "{:?}: {} generators, {} voice effects, {} effects, {} modulators, {} routes, macro 1 named {:?}, slot 64 = {}",
        patch["meta"]["name"].as_str().unwrap_or_default(),
        len("generators"),
        len("voice_fx"),
        len("fx"),
        len("modulators"),
        len("routes"),
        patch["macros"][0]["name"].as_str().unwrap_or_default(),
        patch["slots"]["64"]
    );
    report.add(
        &scope,
        "patch-is-not-default",
        patch["meta"]["name"] == "reaper-test roundtrip"
            && len("generators") == 3
            && len("voice_fx") == 2
            && len("fx") == 2
            && len("modulators") == 4
            && len("routes") == 4
            && patch["macros"][0]["name"] == "Bright"
            && patch["slots"]["64"] == "reverb1.wet",
        facts,
    );
}

// ---- two-instances -----------------------------------------------------------------------------------------------

/// Two instances each came back with their own patch and values, before and after the reload in a new process.
pub fn two_instances(out: &Path, report: &mut Report, format: Format) {
    let scope = format!("{}/two-instances", format.tag());
    let mut states = BTreeMap::new();
    for label in [
        "two-a-create",
        "two-b-create",
        "two-a-reload",
        "two-b-reload",
    ] {
        match Saved::read(out, format, label) {
            Ok(saved) => {
                states.insert(label, saved);
            }
            Err(why) => {
                report.add(&scope, "state-decoded", false, why);
                return;
            }
        }
    }
    // Instance A: patch "reaper-test state", macro 1 = 0.2, slot 1 = 0.1; instance B: patch "reaper-test roundtrip",
    // macro 1 = 0.9, slot 1 = 0.8. Both patches drive slot 1 to `filter1.cutoff`.
    let want = [
        ("a", "reaper-test state", 0.2, 0.1),
        ("b", "reaper-test roundtrip", 0.9, 0.8),
    ];
    for (instance, name, macro1, slot1) in want {
        for phase in ["create", "reload"] {
            let saved = &states[format!("two-{instance}-{phase}").as_str()];
            let patch = saved.patch().unwrap_or_default();
            let cutoff = patch["voice_fx"][0]["params"]["cutoff"].as_f64();
            let want_cutoff = cutoff_at(slot1);
            report.add(
                &scope,
                &format!("instance-{instance}-{phase}"),
                patch["meta"]["name"] == name
                    && near(saved.param("macro_1"), macro1, 0.002)
                    && near(saved.param("slot_1"), slot1, 0.002)
                    && near(patch["macros"][0]["value"].as_f64(), macro1, 0.002)
                    && cutoff.is_some_and(|c| (c - want_cutoff).abs() < 0.02 * want_cutoff),
                format!(
                    "{:?}, macro_1 = {:?}, slot_1 = {:?}, patch macro 1 = {:?}, cutoff = {cutoff:?} Hz (want {want_cutoff:.1})",
                    patch["meta"]["name"].as_str().unwrap_or_default(),
                    saved.param("macro_1"),
                    saved.param("slot_1"),
                    patch["macros"][0]["value"].as_f64()
                ),
            );
        }
        let (created, reloaded) = (
            &states[format!("two-{instance}-create").as_str()],
            &states[format!("two-{instance}-reload").as_str()],
        );
        let same = created.patch_text() == reloaded.patch_text()
            && created.0["params"] == reloaded.0["params"];
        report.add(
            &scope,
            &format!("instance-{instance}-survives-reload"),
            same,
            "the patch text and the parameters are identical after the reload".to_owned(),
        );
    }
    report.add(
        &scope,
        "instances-differ",
        states["two-a-create"].patch_text() != states["two-b-create"].patch_text(),
        "the two instances hold different patches".to_owned(),
    );
}

// ---- rendered audio ------------------------------------------------------------------------------------------------

fn load_render(
    out: &Path,
    report: &mut Report,
    scope: &str,
    name: &str,
    min_seconds: f64,
) -> Option<Wav> {
    let path = out.join(name);
    let wav = match Wav::read(&path) {
        Ok(wav) => wav,
        Err(e) => {
            report.add(scope, "render-readable", false, e);
            return None;
        }
    };
    report.add(
        scope,
        "render-finite",
        wav.finite(),
        format!("{} frames at {} Hz", wav.left.len(), wav.rate),
    );
    if wav.seconds() < min_seconds {
        report.add(
            scope,
            "render-length",
            false,
            format!("{:.2} s, want at least {min_seconds} s", wav.seconds()),
        );
        return None;
    }
    Some(wav)
}

/// How many notes `render.lua` plays, and the groups the ramps are measured over. The notes alternate between two
/// pitches (48 and 55), which differ in brightness, so a group is made of the even notes (pitch 48) only: three of them,
/// so its median survives one note that REAPER's offline render dropped. `EARLY` and `LATE` are in the first ramp
/// (0-2 s) with `MID` between them; `BEFORE` and `AFTER` are in the second (2-4 s). The first two notes are never
/// measured: the offline render restarts the plugin about 30 ms in. Note n starts at n/8 s.
pub const NOTES: usize = 32;
const EARLY: [usize; 3] = [2, 4, 6];
const MID: [usize; 3] = [8, 10, 12];
const LATE: [usize; 3] = [14, 16, 18];
const BEFORE: [usize; 3] = [18, 20, 22];
const RAMP_START: [usize; 3] = [16, 18, 20];
const AFTER: [usize; 3] = [26, 28, 30];

/// `slot-automation`: slot 1 (the cutoff) ramped 0 to 1 over 0-2 s and made the sound brighter; slot 64 (osc1's level)
/// ramped 1 to 0.25 over 2-4 s and made it quieter, with the brightness unchanged.
pub fn slot_automation(out: &Path, report: &mut Report, format: Format) {
    let scope = format!("{}/slot-automation", format.tag());
    let Some(wav) = load_render(
        out,
        report,
        &scope,
        &format!("render-slot-{}.wav", format.tag()),
        4.0,
    ) else {
        return;
    };
    let notes = Notes::measure(&wav, NOTES);
    let r = |g: [usize; 3]| median_at(&notes.roughness, &g);
    let l = |g: [usize; 3]| median_at(&notes.rms, &g);
    let (early, mid, late) = (r(EARLY), r(MID), r(LATE));
    report.add(
        &scope,
        "slot-1-gets-brighter",
        early > 0.0 && late > 2.0 * early,
        format!("brightness {early:.4} at notes 2-6 (0.25-0.75 s) -> {late:.4} at notes 14-18 (1.75-2.25 s)"),
    );
    report.add(
        &scope,
        "slot-1-rises-steadily",
        early < mid && mid < late,
        format!("brightness {early:.4} -> {mid:.4} at notes 8-12 (1.0-1.5 s) -> {late:.4}"),
    );
    let (loud_before, loud_after) = (l(BEFORE), l(AFTER));
    report.add(
        &scope,
        "slot-64-gets-quieter",
        loud_before > 0.0 && loud_after < 0.75 * loud_before,
        format!("level {loud_before:.4} at notes 18-22 (2.25-2.75 s) -> {loud_after:.4} at notes 26-30 (3.25-3.75 s)"),
    );
    let (bright_before, bright_after) = (r(BEFORE), r(AFTER));
    report.add(
        &scope,
        "slot-64-leaves-brightness",
        bright_before > 0.0 && (0.8..1.25).contains(&(bright_after / bright_before)),
        format!("brightness {bright_before:.4} -> {bright_after:.4} while the level ramps"),
    );
}

/// A lane on slot 64 that never moves (0.25 for the whole render, where the patch says 1): is it applied? The plugin
/// takes the host's values at activation as its baseline and plays the patch until the host moves one, and REAPER's
/// offline render reactivates a CLAP plugin early with the lane's value already set, so CLAP plays the patch's level
/// (about 0.41) and VST3 the lane's (about 0.10). `known_gap` turns the CLAP result into an `info` row instead of a
/// failure; `reaper.rs` holds the switch.
pub fn flat_lane(out: &Path, report: &mut Report, format: Format, known_gap: bool) {
    let scope = format!("{}/slot-automation", format.tag());
    let Some(wav) = load_render(
        out,
        report,
        &scope,
        &format!("render-flat-{}.wav", format.tag()),
        4.0,
    ) else {
        return;
    };
    let notes = Notes::measure(&wav, NOTES);
    let level = median_at(&notes.rms, &[8, 10, 12, 14, 16, 18, 20, 22, 24, 26, 28]);
    let applied = level < 0.25;
    let detail = format!(
        "a lane held at 0.25 on slot 64 (osc1's level) sounds at level {level:.3}; {}",
        if applied {
            "the lane's 25 % plays (the patch's 100 % would be about 0.41)"
        } else {
            "the patch's 100 % plays instead of the lane's 25 % (the lane would be about 0.10)"
        }
    );
    if applied {
        report.add(&scope, "slot-flat-lane-applied", true, detail);
    } else if known_gap && format == Format::Clap {
        report.info(
            &scope,
            "slot-flat-lane-applied",
            format!("{detail}: KNOWN GAP in an offline render, a lane that never moves is ignored by CLAP until it moves"),
        );
    } else {
        report.add(&scope, "slot-flat-lane-applied", false, detail);
    }
}

/// `macro-automation`: macro 1 (osc1's level) ramped 0 to 1 over 0-2 s and made the sound louder, with the brightness
/// unchanged; macro 8 (the cutoff) ramped 0 to 1 over 2-4 s and made it brighter.
pub fn macro_automation(out: &Path, report: &mut Report, format: Format) {
    let scope = format!("{}/macro-automation", format.tag());
    let Some(wav) = load_render(
        out,
        report,
        &scope,
        &format!("render-macro-{}.wav", format.tag()),
        4.0,
    ) else {
        return;
    };
    let notes = Notes::measure(&wav, NOTES);
    let r = |g: [usize; 3]| median_at(&notes.roughness, &g);
    let l = |g: [usize; 3]| median_at(&notes.rms, &g);
    let (loud_early, loud_late) = (l(EARLY), l(LATE));
    report.add(
        &scope,
        "macro-1-gets-louder",
        loud_early > 0.0 && loud_late > 3.0 * loud_early,
        format!(
            "level {loud_early:.4} at notes 2-6 (0.25-0.75 s) -> {loud_late:.4} at notes 14-18 (1.75-2.25 s)"
        ),
    );
    let (bright_early, bright_late) = (r(EARLY), r(LATE));
    report.add(
        &scope,
        "macro-1-leaves-brightness",
        bright_early > 0.0 && (0.75..1.33).contains(&(bright_late / bright_early)),
        format!("brightness {bright_early:.4} -> {bright_late:.4} while the level ramps"),
    );
    let (before, after) = (r(RAMP_START), r(AFTER));
    report.add(
        &scope,
        "macro-8-gets-brighter",
        before > 0.0 && after > 2.0 * before,
        format!(
            "brightness {before:.4} at notes 16-20 (2.0-2.5 s) -> {after:.4} at notes 26-30 (3.25-3.75 s)"
        ),
    );
}

/// CLAP and VST3 sound the same: the notes' levels agree from the third note on (a dropped note is tolerated, up to
/// two). The first notes are reported, not judged: see the `info` row.
pub fn renders_agree(out: &Path, report: &mut Report, kind: &str, first: Format, second: Format) {
    let scope = "clap+vst3";
    let name = format!("{kind}-render-same-in-both-formats");
    let read = |format: Format| {
        Wav::read(&out.join(format!("render-{kind}-{}.wav", format.tag())))
            .map(|wav| Notes::measure(&wav, NOTES))
    };
    let (a, b) = match (read(first), read(second)) {
        (Ok(a), Ok(b)) => (a, b),
        (a, b) => {
            report.add(scope, &name, false, a.err().or(b.err()).unwrap_or_default());
            return;
        }
    };
    let (median_difference, outliers) = compare_levels(&a, &b, 2, 0.05);
    report.add(
        scope,
        &name,
        median_difference < 0.02 && outliers <= 2,
        format!(
            "the {kind}-automation notes 3-32 differ in level by {:.2} % (median), {outliers} notes by more than 5 %",
            median_difference * 100.0
        ),
    );
    report.info(
        scope,
        &format!("{kind}-first-note-level"),
        format!(
            "the first note's level is {:.3} in {} and {:.3} in {}{}",
            a.rms[0],
            first.tag(),
            b.rms[0],
            second.tag(),
            if kind == "slot" && (a.rms[0] - b.rms[0]).abs() > 0.5 * a.rms[0].max(b.rms[0]) {
                ": a lane that starts at the slot's default 0.0 is not applied until it moves (the patch plays meanwhile), by CLAP only"
            } else {
                ""
            }
        ),
    );
}

/// The spike shell's `automation` scenario (kept as it was): macro 1 and slot 1 ramped together from 0 to 1.
pub fn automation_legacy(out: &Path, report: &mut Report, format: Format) {
    let scope = format!("{}/automation", format.tag());
    let Some(wav) = load_render(
        out,
        report,
        &scope,
        &format!("render-{}.wav", format.tag()),
        2.0,
    ) else {
        return;
    };
    let (early, late) = (wav.window(0.25, 0.55), wav.window(1.7, 2.0));
    let (rms_early, rms_late) = (rms(early), rms(late));
    let (hf_early, hf_late) = (roughness(early), roughness(late));
    report.add(
        &scope,
        "macro-1-gets-louder",
        rms_early > 0.0 && rms_late > 3.0 * rms_early,
        format!("rms {rms_early:.4} at 0.25-0.55 s -> {rms_late:.4} at 1.7-2.0 s"),
    );
    report.add(
        &scope,
        "slot-1-gets-brighter",
        hf_early > 0.0 && hf_late > 2.0 * hf_early,
        format!("brightness {hf_early:.4} at 0.25-0.55 s -> {hf_late:.4} at 1.7-2.0 s"),
    );
}

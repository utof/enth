//! **Owner: lane DG.** The preset anchors of `ds1` and their projection onto layout 1 (docs/r3.md, "`ds1`").
//!
//! 25 % of `ds1`'s samples are perturbed preset candidates **from the anchor half only**: the 11 files below are byte
//! copies of the anchor half of `content/presets-candidates/` (lane DS's split: `content/gr3/synth.toml`'s
//! `[preset_split]`, the even and odd positions of the SHA-256 order), taken at the freeze, so that a later retune of a
//! candidate never moves `ds1` v1's samples (the way `content/gr3/presets/` freezes the test half). The test half is
//! never here: a test (`the_anchors_are_the_anchor_half_and_none_is_excluded`) compares the stems with `synth.toml`'s
//! `anchor` list and every stem, name and file hash with the exclusion lists.
//!
//! **Projection.** Layout 1 is generator v1's draw set on template 0, so a preset reaches `ds1` through
//! [`project`]: start from the template and copy the values of the layout's entries (the 33 continuous values, the 7
//! categoricals, the 4 structure switches and the routes on the 4 x 4 grid). Everything else goes back to the
//! template: `osc1.sync`, the reverb's `early`, `detune` and damping, the compressor's `knee`, `makeup` and `mix`,
//! the macro routes, the glide mode and polyphony, a module's seed and phase. [`measure`] reports how far that moves
//! each preset, as the parameters it changes and, in sound, the pooled log-mel distance between the preset and its
//! projection rendered at the same note (the bank's embedding, `nearest::embed`; one just-noticeable step is about
//! 0.125 nepers, `NEAR_RENDER_NEPERS` in `bank.rs`).
#![allow(
    clippy::disallowed_methods,
    reason = "tooling: renders and measures; the sampler's arithmetic goes through libm"
)]

use enth_analysis::Describer;
use enth_analysis::descriptors::Want;
use enth_analysis::features::{FeatureCfg, FeatureExtractor};
use enth_challenge::priors::jnd;
use enth_patch::{Op, ParamAddr, Patch, Route, apply, file};
use enth_render::{Performance, RenderSpec, render};
use enth_solver::nearest::{Embedding, REF_NOTE, embed, reference_hz};
use enth_solver::propose::decode::OutputLayout;
use sha2::{Digest, Sha256};

/// The anchors, as `(stem, bytes)`: byte copies of the anchor half of `content/presets-candidates/`.
pub const FILES: [(&str, &str); 11] = [
    (
        "wobble-machine",
        include_str!("anchors/wobble-machine.patch.json"),
    ),
    ("warm-reese", include_str!("anchors/warm-reese.patch.json")),
    (
        "noise-riser",
        include_str!("anchors/noise-riser.patch.json"),
    ),
    ("soft-bell", include_str!("anchors/soft-bell.patch.json")),
    ("solar-lead", include_str!("anchors/solar-lead.patch.json")),
    (
        "pixel-square",
        include_str!("anchors/pixel-square.patch.json"),
    ),
    ("velvet-pad", include_str!("anchors/velvet-pad.patch.json")),
    (
        "night-drift",
        include_str!("anchors/night-drift.patch.json"),
    ),
    ("thump-kick", include_str!("anchors/thump-kick.patch.json")),
    (
        "glass-pluck",
        include_str!("anchors/glass-pluck.patch.json"),
    ),
    (
        "wind-tunnel",
        include_str!("anchors/wind-tunnel.patch.json"),
    ),
];

/// One anchor: the preset and its projection onto the layout.
#[derive(Clone, Debug)]
pub struct Anchor {
    /// The file stem (`warm-reese`).
    pub stem: &'static str,
    /// The preset's `meta.name`.
    pub name: String,
    /// SHA-256 of the file's bytes, lowercase hex.
    pub sha256: String,
    /// The preset as saved.
    pub original: Patch,
    /// The preset projected onto layout 1: the template with the layout's entries copied in.
    pub projected: Patch,
}

/// The hex of a SHA-256.
pub fn hex(bytes: &[u8]) -> String {
    enth_solver::create::record::hex(bytes)
}

/// SHA-256 of `bytes` as lowercase hex.
pub fn sha256_hex(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

/// The anchors, projected onto `layout`.
///
/// # Errors
/// A file that does not load, or an op that fails on the template (a catalogue this layout was not built for).
pub fn load(layout: &OutputLayout) -> Result<Vec<Anchor>, String> {
    FILES
        .iter()
        .map(|(stem, text)| {
            let loaded =
                file::load(text).map_err(|e| format!("anchor {stem} does not load: {e}"))?;
            let original = loaded.patch;
            let projected =
                project(layout, &original).map_err(|e| format!("anchor {stem}: {e}"))?;
            Ok(Anchor {
                stem,
                name: original.meta.name.clone(),
                sha256: sha256_hex(text.as_bytes()),
                original,
                projected,
            })
        })
        .collect()
}

/// `patch` projected onto `layout` (see the module docs): the template with every layout entry `patch` has copied in,
/// the structure switches that are on, and the routes that sit on the grid.
///
/// # Errors
/// An op that fails (a value the template's catalogue refuses).
pub fn project(layout: &OutputLayout, patch: &Patch) -> Result<Patch, String> {
    let mut p = layout.template().clone();
    let set = |p: &mut Patch, addr: &ParamAddr, value: f32| {
        apply(
            p,
            Op::SetParam {
                addr: addr.clone(),
                value,
            },
        )
        .map(drop)
        .map_err(|e| format!("{addr}: {e}"))
    };
    for e in layout.continuous().iter().chain(layout.categorical()) {
        if let Some(v) = patch.value(&e.addr) {
            set(&mut p, &e.addr, e.spec.clamp(v))?;
        }
    }
    for e in layout.enables() {
        if patch.find(&e.id).is_some_and(|(_, m)| m.enabled) {
            apply(
                &mut p,
                Op::SetEnabled {
                    id: e.id.clone(),
                    enabled: true,
                },
            )
            .map_err(|e| e.to_string())?;
        }
    }
    for r in &patch.routes {
        if r.depth != 0.0
            && layout
                .routes()
                .iter()
                .any(|c| c.src == r.src && c.dst == r.dst)
        {
            let index = p.routes.len();
            apply(
                &mut p,
                Op::InsertRoute {
                    index,
                    route: Route {
                        src: r.src.clone(),
                        dst: r.dst.clone(),
                        depth: r.depth,
                    },
                },
            )
            .map_err(|e| e.to_string())?;
        }
    }
    // The context entry: the preset's own tuning, within the entry's range (`ds1` draws its own note noise over it).
    let fine: ParamAddr = "osc1.fine"
        .parse()
        .map_err(|e| format!("osc1.fine: {e:?}"))?;
    if let Some(v) = patch.value(&fine) {
        set(&mut p, &fine, v.clamp(-50.0, 50.0))?;
    }
    Ok(p)
}

/// What projecting one anchor changes.
#[derive(Clone, Debug)]
pub struct Moved {
    /// The anchor's stem.
    pub stem: &'static str,
    /// Parameters outside the layout whose value differs from the template's (so projection resets them), with the
    /// preset's value and the template's, as `addr: from -> to`.
    pub reset: Vec<String>,
    /// Of those, how many exceed generator v1's just-noticeable step (a discrete change always does) and so may be
    /// audible.
    pub reset_audible: usize,
    /// Routes the grid has no cell for (macro routes, other destinations), as `src->dst depth`.
    pub dropped_routes: Vec<String>,
    /// Layout entries projection changed (clamped into range): normally none.
    pub clamped: usize,
    /// Modules the preset has and the template does not (projection leaves them out), and modules whose switch the
    /// projection changes without a structure head to carry it, as `id (kind, on)` or `id: on -> off`.
    pub modules: Vec<String>,
    /// The pooled log-mel distance between the preset and its projection at the reference note, in nepers.
    pub nepers: Option<f64>,
}

/// The note, velocity and lengths of the measurement: the bank's reference performance (C3, velocity 0.8, held 1 s, in
/// a 2 s render at 48 kHz).
fn reference() -> Performance {
    Performance::note(REF_NOTE, 0.8, 1.0, 2.0)
}

fn embedding_of(patch: &Patch) -> Option<Embedding> {
    embedding(patch, &reference(), 48_000)
}

/// The bank's embedding of `patch` playing `perf` at `rate`: the pooled log-mel that `nearest` reads. `None` for a
/// silent render. Distances between two of them are in nepers (about 0.125 per just-noticeable step).
pub fn embedding(patch: &Patch, perf: &Performance, rate: u32) -> Option<Embedding> {
    let out = render(
        patch,
        perf,
        &RenderSpec::new(rate, enth_render::DEFAULT_SEED),
    )
    .ok()?;
    let mid = out.mid();
    let cfg = FeatureCfg {
        sample_rate: f64::from(rate),
        ..FeatureCfg::default()
    };
    let features = FeatureExtractor::new(cfg).extract(&mid);
    if features.silent {
        return None;
    }
    let desc = Describer::new(f64::from(rate))
        .describe_some(&mid, enth_solver::note_timing(perf), Want::CORE)
        .ok();
    Some(embed(
        &features,
        &cfg,
        0.0,
        reference_hz(desc.as_ref()),
        f64::from(rate) / 2.0,
    ))
}

/// How far projecting `a` moves it: in parameters and, rendered at the reference note, in sound.
pub fn measure(layout: &OutputLayout, a: &Anchor) -> Moved {
    let in_layout = |addr: &ParamAddr| {
        layout
            .continuous()
            .iter()
            .chain(layout.categorical())
            .any(|e| &e.addr == addr)
    };
    let fine: Option<ParamAddr> = "osc1.fine".parse().ok();
    let mut reset = Vec::new();
    let mut reset_audible = 0;
    let mut addrs: Vec<ParamAddr> = Vec::new();
    for (_, m) in a.original.modules() {
        for spec in m.kind.params() {
            if let Ok(addr) = ParamAddr::module(m.id.clone(), spec.key) {
                addrs.push(addr);
            }
        }
    }
    for addr in addrs {
        if in_layout(&addr) || Some(&addr) == fine.as_ref() {
            continue;
        }
        let (Some(from), Some(to), Some(spec)) = (
            a.original.value(&addr),
            a.projected.value(&addr),
            a.original.spec(&addr),
        ) else {
            continue;
        };
        if from != to {
            reset.push(format!("{addr}: {from} -> {to}"));
            if jnd(spec).exceeded(from, to) {
                reset_audible += 1;
            }
        }
    }
    let dropped_routes: Vec<String> = a
        .original
        .routes
        .iter()
        .filter(|r| {
            r.depth != 0.0
                && !layout
                    .routes()
                    .iter()
                    .any(|c| c.src == r.src && c.dst == r.dst)
        })
        .map(|r| format!("{}->{} {}", r.src, r.dst, r.depth))
        .collect();
    let clamped = layout
        .continuous()
        .iter()
        .chain(layout.categorical())
        .filter(|e| {
            a.original
                .value(&e.addr)
                .is_some_and(|v| e.spec.clamp(v) != v)
        })
        .count();
    let mut modules = Vec::new();
    for (_, m) in a.original.modules() {
        match a.projected.find(&m.id) {
            None => modules.push(format!(
                "{} ({:?}, {})",
                m.id,
                m.kind,
                if m.enabled { "on" } else { "off" }
            )),
            Some((_, p)) if p.enabled != m.enabled => modules.push(format!(
                "{}: {} -> {}",
                m.id,
                if m.enabled { "on" } else { "off" },
                if p.enabled { "on" } else { "off" }
            )),
            Some(_) => {}
        }
    }
    let nepers = match (embedding_of(&a.original), embedding_of(&a.projected)) {
        (Some(x), Some(y)) => x.distance(&y),
        _ => None,
    };
    Moved {
        stem: a.stem,
        reset,
        reset_audible,
        dropped_routes,
        clamped,
        modules,
        nepers,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout() -> OutputLayout {
        OutputLayout::v1()
    }

    #[test]
    fn the_anchors_load_and_project_onto_the_layout() {
        let l = layout();
        let anchors = load(&l).unwrap();
        assert_eq!(anchors.len(), 11);
        for a in &anchors {
            // The module set is the template's, so `encode` accepts the projection (checked in `pipeline`'s tests).
            let mods = |p: &Patch| {
                p.modules()
                    .map(|(s, m)| (s, m.id.clone(), m.kind))
                    .collect::<Vec<_>>()
            };
            assert_eq!(mods(&a.projected), mods(l.template()), "{}", a.stem);
            // Every layout entry equals the preset's.
            for e in l.continuous().iter().chain(l.categorical()) {
                assert_eq!(
                    a.projected.value(&e.addr),
                    a.original.value(&e.addr),
                    "{}: {}",
                    a.stem,
                    e.addr
                );
            }
            // Only grid routes survive.
            for r in &a.projected.routes {
                assert!(
                    l.routes().iter().any(|c| c.src == r.src && c.dst == r.dst),
                    "{}: {}->{}",
                    a.stem,
                    r.src,
                    r.dst
                );
            }
        }
    }

    #[test]
    fn projecting_twice_changes_nothing() {
        let l = layout();
        for a in load(&l).unwrap() {
            assert_eq!(
                project(&l, &a.projected).unwrap(),
                a.projected,
                "{}",
                a.stem
            );
        }
    }

    #[test]
    fn projection_resets_what_the_layout_does_not_hold() {
        let l = layout();
        let anchors = load(&l).unwrap();
        // Warm Reese saves macro routes: all of them leave.
        let reese = anchors.iter().find(|a| a.stem == "warm-reese").unwrap();
        let m = measure(&l, reese);
        assert!(
            m.dropped_routes.iter().any(|r| r.starts_with("macro")),
            "{m:?}"
        );
        assert!(
            reese
                .projected
                .routes
                .iter()
                .all(|r| !r.src.to_string().starts_with("macro"))
        );
        assert_eq!(m.clamped, 0);
    }
}

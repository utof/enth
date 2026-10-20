//! The observability mask (lane RG; design.md §9): what each phrase reveals of the starter patch and of edits to it.
//! Pure parameter-space cases; ST and the overlay rely on these classifications.

use enth_challenge::phrases::PhraseId;
use enth_challenge::random::{self, Dead, Mask, Visibility, observable};
use enth_patch::{ModSource, ModuleId, Op, ParamAddr, Patch, Route, RouteAddr, apply_all};

fn starter() -> Patch {
    random::start(random::TEMPLATE_STARTER).unwrap()
}

fn a(s: &str) -> ParamAddr {
    s.parse().unwrap()
}

fn with(ops: &[Op]) -> Patch {
    let mut p = starter();
    apply_all(&mut p, ops).unwrap();
    p
}

fn set(addr: &str, value: f32) -> Op {
    Op::SetParam {
        addr: a(addr),
        value,
    }
}

fn route(src: &str, dst: &str, depth: f32) -> Op {
    Op::InsertRoute {
        index: 0,
        route: Route {
            src: src.parse().unwrap(),
            dst: a(dst),
            depth,
        },
    }
}

fn enable(id: &str, on: bool) -> Op {
    Op::SetEnabled {
        id: ModuleId::new(id).unwrap(),
        enabled: on,
    }
}

fn mask(p: &Patch, phrase: PhraseId) -> Mask {
    observable(p, phrase.phrase())
}

fn vis(m: &Mask, addr: &str) -> Visibility {
    m.param(&a(addr))
        .unwrap_or_else(|| panic!("{addr} missing"))
}

fn ra(src: &str, dst: &str) -> RouteAddr {
    RouteAddr {
        src: src.parse::<ModSource>().unwrap(),
        dst: a(dst),
    }
}

#[test]
fn the_starter_under_one_held_note() {
    let m = mask(&starter(), PhraseId::Held);
    use Visibility::{Disabled, Gauge, Nuisance, Observable};
    let cases = [
        // gauges
        ("osc1.level", Gauge),
        ("master.level", Gauge),
        ("voice.pan", Gauge),
        // nuisances
        ("osc1.phase", Nuisance),
        ("osc1.phase_mode", Nuisance),
        ("osc1.seed", Nuisance),
        // dead by structure
        ("osc1.pw", Visibility::Dead(Dead::NotPulse)),
        ("osc1.position", Visibility::Dead(Dead::NotTable)),
        ("osc1.table", Visibility::Dead(Dead::NotTable)),
        ("osc1.sync", Visibility::Dead(Dead::SyncMaster)),
        ("env2.decay", Visibility::Dead(Dead::Unrouted)),
        ("lfo1.rate", Visibility::Dead(Dead::Unrouted)),
        // dead by the phrase
        ("filter1.keytrack", Visibility::Dead(Dead::NeedsOctave)),
        ("env1.velocity", Visibility::Dead(Dead::OneVelocity)),
        ("voice.glide", Visibility::Dead(Dead::NoGlide)),
        ("voice.glide_mode", Visibility::Dead(Dead::NoGlide)),
        ("voice.mode", Visibility::Dead(Dead::OneNote)),
        ("voice.polyphony", Visibility::Dead(Dead::OneNote)),
        // disabled
        ("osc2.wave", Disabled),
        ("noise1.level", Disabled),
        ("comp1.threshold", Disabled),
        ("reverb1.wet", Disabled),
        // observable
        ("osc1.wave", Observable),
        ("osc1.semi", Observable),
        ("osc1.fine", Observable),
        ("filter1.cutoff", Observable),
        ("filter1.resonance", Observable),
        ("filter1.drive", Observable),
        ("filter1.mode", Observable),
        ("env1.attack", Observable),
        ("env1.release", Observable),
    ];
    for (addr, want) in cases {
        assert_eq!(vis(&m, addr), want, "{addr}");
    }
    // Enable flags: a generator or an effect is heard when switched on; an unrouted modulator is not.
    let id = |s: &str| ModuleId::new(s).unwrap();
    assert_eq!(m.module(&id("osc2")), Some(Observable));
    assert_eq!(m.module(&id("reverb1")), Some(Observable));
    assert_eq!(m.module(&id("env1")), Some(Observable));
    assert_eq!(
        m.module(&id("lfo1")),
        Some(Visibility::Dead(Dead::Unrouted))
    );
    // Every parameter of the patch is classified, and nothing else.
    let p = starter();
    let count: usize = p.modules().map(|(_, m)| m.values.len()).sum::<usize>() + 5 + 1;
    assert_eq!(m.params().count(), count);
    assert_eq!(m.routes().count(), 0);
}

#[test]
fn the_octave_pair_reveals_keytrack_and_velocity() {
    let m = mask(&starter(), PhraseId::OctavePair);
    assert_eq!(vis(&m, "filter1.keytrack"), Visibility::Observable);
    assert_eq!(vis(&m, "env1.velocity"), Visibility::Observable);
    // glide_mode = always glides a detached note too, so glide is heard; its mode only matters once glide is on.
    assert_eq!(vis(&m, "voice.glide"), Visibility::Observable);
    assert_eq!(vis(&m, "voice.glide_mode"), Visibility::Dead(Dead::NoGlide));
    let m = mask(&with(&[set("voice.glide", 200.0)]), PhraseId::OctavePair);
    assert_eq!(vis(&m, "voice.glide_mode"), Visibility::Observable);
    assert_eq!(vis(&m, "voice.mode"), Visibility::Observable);
    assert_eq!(vis(&m, "lfo1.scope"), Visibility::Dead(Dead::Unrouted));
}

#[test]
fn the_legato_pair_reveals_glide() {
    let p = with(&[set("voice.mode", 2.0), set("voice.glide", 100.0)]);
    let m = mask(&p, PhraseId::LegatoPair);
    assert_eq!(vis(&m, "voice.glide"), Visibility::Observable);
    // Only overlapping notes: always and legato glide the same.
    assert_eq!(vis(&m, "voice.glide_mode"), Visibility::Dead(Dead::NoGlide));
    assert_eq!(
        vis(&m, "filter1.keytrack"),
        Visibility::Dead(Dead::NeedsOctave)
    );
    assert_eq!(
        vis(&m, "env1.velocity"),
        Visibility::Dead(Dead::OneVelocity)
    );
    // In legato glide mode, a held phrase never glides.
    let p = with(&[set("voice.glide_mode", 1.0), set("voice.glide", 100.0)]);
    assert_eq!(
        vis(&mask(&p, PhraseId::OctavePair), "voice.glide"),
        Visibility::Dead(Dead::NoGlide)
    );
    assert_eq!(
        vis(&mask(&p, PhraseId::LegatoPair), "voice.glide"),
        Visibility::Observable
    );
}

#[test]
fn levels_vary_only_as_ratios() {
    let m = mask(&with(&[enable("osc2", true)]), PhraseId::Held);
    assert_eq!(vis(&m, "osc1.level"), Visibility::Gauge);
    assert_eq!(vis(&m, "osc2.level"), Visibility::Observable);
    assert_eq!(vis(&m, "osc2.wave"), Visibility::Observable);
    // osc 2 is not the sync master, so its sync is heard.
    assert_eq!(vis(&m, "osc2.sync"), Visibility::Observable);
    // With osc 1 off, osc 2 is the lone source: its level is the gauge, and it is the sync master.
    let m = mask(
        &with(&[enable("osc2", true), enable("osc1", false)]),
        PhraseId::Held,
    );
    assert_eq!(vis(&m, "osc2.level"), Visibility::Gauge);
    assert_eq!(vis(&m, "osc2.sync"), Visibility::Dead(Dead::SyncMaster));
    assert_eq!(vis(&m, "osc1.wave"), Visibility::Disabled);
    // The noise next to osc 1.
    let m = mask(&with(&[enable("noise1", true)]), PhraseId::Held);
    assert_eq!(vis(&m, "noise1.level"), Visibility::Observable);
    // A silent oscillator hides its sound, unless a route moves its level.
    let m = mask(
        &with(&[enable("osc2", true), set("osc2.level", 0.0)]),
        PhraseId::Held,
    );
    assert_eq!(vis(&m, "osc2.wave"), Visibility::Dead(Dead::Silent));
    assert_eq!(vis(&m, "osc2.level"), Visibility::Observable);
    let m = mask(
        &with(&[
            enable("osc2", true),
            set("osc2.level", 0.0),
            route("env2", "osc2.level", 50.0),
        ]),
        PhraseId::Held,
    );
    assert_eq!(vis(&m, "osc2.wave"), Visibility::Observable);
}

#[test]
fn effects_keep_their_gauges() {
    let m = mask(
        &with(&[enable("comp1", true), enable("reverb1", true)]),
        PhraseId::Held,
    );
    assert_eq!(vis(&m, "comp1.makeup"), Visibility::Gauge);
    assert_eq!(vis(&m, "comp1.threshold"), Visibility::Observable);
    assert_eq!(vis(&m, "reverb1.dry"), Visibility::Gauge);
    assert_eq!(vis(&m, "reverb1.wet"), Visibility::Observable);
    assert_eq!(vis(&m, "reverb1.decay"), Visibility::Observable);
}

#[test]
fn a_waveform_wakes_its_own_parameters() {
    let m = mask(&with(&[set("osc1.wave", 3.0)]), PhraseId::Held);
    assert_eq!(vis(&m, "osc1.pw"), Visibility::Observable);
    assert_eq!(vis(&m, "osc1.position"), Visibility::Dead(Dead::NotTable));
    let m = mask(&with(&[set("osc1.wave", 4.0)]), PhraseId::Held);
    assert_eq!(vis(&m, "osc1.position"), Visibility::Observable);
    assert_eq!(vis(&m, "osc1.table"), Visibility::Observable);
    assert_eq!(vis(&m, "osc1.pw"), Visibility::Dead(Dead::NotPulse));
}

#[test]
fn routes_wake_modulators() {
    let p = with(&[route("env2", "filter1.cutoff", 30.0)]);
    let m = mask(&p, PhraseId::Held);
    assert_eq!(
        m.route(&ra("env2", "filter1.cutoff")),
        Some(Visibility::Observable)
    );
    assert_eq!(vis(&m, "env2.decay"), Visibility::Observable);
    assert_eq!(
        vis(&m, "env2.velocity"),
        Visibility::Dead(Dead::OneVelocity)
    );
    // A route onto a dead parameter (pw on a saw) wakes nothing.
    let p = with(&[route("lfo1", "osc1.pw", 30.0)]);
    let m = mask(&p, PhraseId::Held);
    assert_eq!(
        m.route(&ra("lfo1", "osc1.pw")),
        Some(Visibility::Dead(Dead::HiddenEnd))
    );
    assert_eq!(vis(&m, "lfo1.rate"), Visibility::Dead(Dead::Unrouted));
    // A route onto a gauge is heard (tremolo on a lone oscillator's level).
    let p = with(&[route("lfo1", "osc1.level", 30.0)]);
    let m = mask(&p, PhraseId::Held);
    assert_eq!(
        m.route(&ra("lfo1", "osc1.level")),
        Some(Visibility::Observable)
    );
    assert_eq!(vis(&m, "lfo1.rate"), Visibility::Observable);
    assert_eq!(vis(&m, "lfo1.shape"), Visibility::Observable);
    assert_eq!(vis(&m, "lfo1.phase"), Visibility::Nuisance);
    assert_eq!(vis(&m, "lfo1.scope"), Visibility::Dead(Dead::OneNote));
    // A chain: lfo2 drives lfo1's rate, lfo1 drives the cutoff; both are live.
    let p = with(&[
        route("lfo1", "filter1.cutoff", 20.0),
        route("lfo2", "lfo1.rate", 20.0),
    ]);
    let m = mask(&p, PhraseId::Held);
    assert_eq!(vis(&m, "lfo2.rate"), Visibility::Observable);
    assert_eq!(
        m.route(&ra("lfo2", "lfo1.rate")),
        Some(Visibility::Observable)
    );
    // ... and without the end of the chain, neither is.
    let p = with(&[route("lfo2", "lfo1.rate", 20.0)]);
    let m = mask(&p, PhraseId::Held);
    assert_eq!(vis(&m, "lfo2.rate"), Visibility::Dead(Dead::Unrouted));
    assert_eq!(
        m.route(&ra("lfo2", "lfo1.rate")),
        Some(Visibility::Dead(Dead::HiddenEnd))
    );
}

#[test]
fn routes_the_phrase_cannot_reveal() {
    let p = with(&[
        route("velocity", "filter1.cutoff", 20.0),
        route("key", "filter1.resonance", 20.0),
        route("macro1", "osc1.fine", 20.0),
        route("env2", "osc1.semi", 0.0),
    ]);
    let m = mask(&p, PhraseId::Held);
    assert_eq!(
        m.route(&ra("velocity", "filter1.cutoff")),
        Some(Visibility::Dead(Dead::OneVelocity))
    );
    assert_eq!(
        m.route(&ra("key", "filter1.resonance")),
        Some(Visibility::Dead(Dead::OneKey))
    );
    assert_eq!(
        m.route(&ra("macro1", "osc1.fine")),
        Some(Visibility::Dead(Dead::NoControls))
    );
    assert_eq!(
        m.route(&ra("env2", "osc1.semi")),
        Some(Visibility::Dead(Dead::ZeroDepth))
    );
    assert_eq!(vis(&m, "env2.decay"), Visibility::Dead(Dead::Unrouted));
    let m = mask(&p, PhraseId::OctavePair);
    assert_eq!(
        m.route(&ra("velocity", "filter1.cutoff")),
        Some(Visibility::Observable)
    );
    assert_eq!(
        m.route(&ra("key", "filter1.resonance")),
        Some(Visibility::Observable)
    );
    // A disabled source or destination.
    let p = with(&[route("env2", "filter1.cutoff", 30.0), enable("env2", false)]);
    assert_eq!(
        mask(&p, PhraseId::Held).route(&ra("env2", "filter1.cutoff")),
        Some(Visibility::Disabled)
    );
    let p = with(&[
        enable("osc2", true),
        route("env2", "osc2.semi", 10.0),
        enable("osc2", false),
    ]);
    let m = mask(&p, PhraseId::Held);
    assert_eq!(
        m.route(&ra("env2", "osc2.semi")),
        Some(Visibility::Disabled)
    );
    assert_eq!(vis(&m, "env2.decay"), Visibility::Dead(Dead::Unrouted));
}

#[test]
fn times_beyond_the_phrase_are_dead() {
    let p = with(&[set("env1.attack", 2_000.0), set("env1.release", 1_600.0)]);
    let m = mask(&p, PhraseId::Held);
    assert_eq!(vis(&m, "env1.attack"), Visibility::Dead(Dead::BeyondHold));
    assert_eq!(vis(&m, "env1.release"), Visibility::Dead(Dead::BeyondTail));
    let p = with(&[set("env1.attack", 1_500.0), set("env1.release", 1_500.0)]);
    let m = mask(&p, PhraseId::Held);
    assert_eq!(vis(&m, "env1.attack"), Visibility::Observable);
    assert_eq!(vis(&m, "env1.release"), Visibility::Observable);
}

#[test]
fn names_are_stable() {
    assert_eq!(Visibility::Observable.name(), "observable");
    assert_eq!(Visibility::Gauge.name(), "gauge");
    assert_eq!(Visibility::Dead(Dead::NotPulse).name(), "dead:not_pulse");
    assert!(Visibility::Observable.is_observable() && !Visibility::Gauge.is_observable());
}

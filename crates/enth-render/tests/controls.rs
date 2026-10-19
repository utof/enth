//! Macro and mod-wheel moves in a `Performance` (P1b lane E+): what the plugin does with a host's macro automation
//! or a MIDI mod wheel, expressible in a CLI render.

use enth_engine::{Engine, Event, EventKind, NoteKey, PrepareCfg, compile};
use enth_patch::{MacroIndex, Patch, Route};
use enth_render::{
    Control, Controller, Note, Performance, RenderOutput, RenderSpec, render, render_with_block,
};

fn bits(x: &[f32]) -> Vec<u32> {
    x.iter().map(|v| v.to_bits()).collect()
}

fn out_bits(o: &RenderOutput) -> Vec<u32> {
    bits(&o.left).into_iter().chain(bits(&o.right)).collect()
}

/// The init saw with macro1 on its level, macro2 on its fine tune and the mod wheel on the pan.
fn patch() -> Patch {
    let mut p = Patch::init();
    p.macros[1].value = 0.25;
    for (src, dst, depth) in [
        ("macro1", "osc1.level", -60.0),
        ("macro2", "osc1.fine", 20.0),
        ("modwheel", "voice.pan", 50.0),
    ] {
        p.routes.push(Route {
            src: src.parse().unwrap(),
            dst: dst.parse().unwrap(),
            depth,
        });
    }
    p
}

fn macro_n(n: usize) -> Controller {
    Controller::Macro(MacroIndex::from_one_based(n).unwrap())
}

fn perf() -> Performance {
    Performance {
        notes: vec![Note::new(57, 0.8, 0.0, 0.7), Note::new(64, 0.6, 0.11, 0.7)],
        controls: vec![
            Control {
                time_s: 0.2,
                controller: macro_n(1),
                value: 1.0,
            },
            Control {
                time_s: 0.33333,
                controller: Controller::ModWheel,
                value: 0.8,
            },
            Control {
                time_s: 0.5,
                controller: macro_n(2),
                value: 0.0,
            },
        ],
        duration_s: 1.0,
        ..Performance::default()
    }
}

#[test]
fn controls_are_the_engines_macro_and_mod_wheel_events() {
    let spec = RenderSpec::default();
    let out = render(&patch(), &perf(), &spec).unwrap();
    // The same performance driven by hand: frames = round(t · sample rate), controls after automation and before
    // notes at the same frame.
    let (plan, _) = compile(&patch(), &PrepareCfg::new(48_000.0)).unwrap();
    let mut e = Engine::new(plan, spec.seed);
    let mut ev: Vec<(u64, EventKind)> = vec![
        (
            9600,
            EventKind::Macro {
                index: 0,
                value: 1.0,
            },
        ),
        (16_000, EventKind::ModWheel { value: 0.8 }),
        (
            24_000,
            EventKind::Macro {
                index: 1,
                value: 0.0,
            },
        ),
    ];
    for (i, (key, vel, on, off)) in [(57u8, 0.8f32, 0u64, 33_600u64), (64, 0.6, 5280, 33_600)]
        .into_iter()
        .enumerate()
    {
        let k = NoteKey::new(key, i as i32);
        ev.push((
            on,
            EventKind::NoteOn {
                key: k,
                velocity: vel,
            },
        ));
        ev.push((
            off,
            EventKind::NoteOff {
                key: k,
                velocity: 0.0,
            },
        ));
    }
    ev.sort_by_key(|(f, _)| *f);
    let (mut l, mut r) = (vec![0.0f32; 48_000], vec![0.0f32; 48_000]);
    let events: Vec<Event> = ev
        .iter()
        .map(|&(f, kind)| Event {
            offset: f as u32,
            kind,
        })
        .collect();
    e.process(&mut l, &mut r, &events);
    assert_eq!(bits(&out.left), bits(&l));
    assert_eq!(bits(&out.right), bits(&r));
    // And they matter.
    let plain = render(
        &patch(),
        &Performance {
            controls: Vec::new(),
            ..perf()
        },
        &spec,
    )
    .unwrap();
    assert_ne!(out_bits(&out), out_bits(&plain));
}

#[test]
fn controls_do_not_depend_on_the_block_size() {
    let spec = RenderSpec::default();
    let reference = render(&patch(), &perf(), &spec).unwrap();
    for block in [1, 31, 64, 333, 4096] {
        let got = render_with_block(&patch(), &perf(), &spec, block).unwrap();
        assert_eq!(out_bits(&got), out_bits(&reference), "block {block}");
    }
}

#[test]
fn a_control_to_the_patch_value_changes_nothing() {
    let spec = RenderSpec::default();
    let plain = render(&patch(), &Performance::note(60, 0.8, 0.5, 0.8), &spec).unwrap();
    let same = Performance {
        controls: vec![
            Control {
                time_s: 0.3,
                controller: macro_n(2),
                value: 0.25,
            },
            Control {
                time_s: 0.4,
                controller: Controller::ModWheel,
                value: 0.0,
            },
        ],
        ..Performance::note(60, 0.8, 0.5, 0.8)
    };
    let got = render(&patch(), &same, &spec).unwrap();
    assert_eq!(out_bits(&got), out_bits(&plain));
}

#[test]
fn bad_controls_are_typed_errors_and_controllers_have_a_text_form() {
    let spec = RenderSpec::default();
    for (time_s, value) in [
        (0.1, 1.5),
        (0.1, -0.1),
        (0.1, f32::NAN),
        (f64::NAN, 0.5),
        (-1.0, 0.5),
    ] {
        let p = Performance {
            controls: vec![Control {
                time_s,
                controller: Controller::ModWheel,
                value,
            }],
            duration_s: 0.5,
            ..Performance::default()
        };
        assert_eq!(
            render(&patch(), &p, &spec).unwrap_err().code(),
            "bad_performance",
            "{time_s} {value}"
        );
    }
    for text in ["macro1", "macro8", "modwheel"] {
        let c: Controller = text.parse().unwrap();
        assert_eq!(c.to_string(), text);
    }
    for text in ["macro9", "velocity", "key", "lfo1", ""] {
        assert_eq!(
            text.parse::<Controller>().unwrap_err().code(),
            "bad_performance",
            "{text}"
        );
    }
}

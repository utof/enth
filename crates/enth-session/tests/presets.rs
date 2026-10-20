//! The preset core (lane PBC of R1/S1), through the public API: the file format and its fixtures, the index, search,
//! tags, favorites, saving (names, folders, overwrite protection), the in-memory store, and the bookmark in the saved
//! state. What a load does to the history, the engine and the host is in `src/tests/presets.rs` (it needs the crate's
//! internals).
//!
//! Fixtures live in `tests/fixtures/presets/<folder>/<name>.patch.json`, exactly the text a preset file holds
//! (`Patch::to_json()`). They are what `fixture_cases` builds; a change to the format fails
//! `the_fixtures_are_what_this_build_writes`. Regenerate on purpose with
//! `cargo test -p enth-session --test presets -- --ignored regenerate` and read the diff.

#![allow(
    clippy::disallowed_methods,
    reason = "a test reads its fixture files and the regenerate test writes them; the library does no I/O"
)]

mod common;

use std::fs;
use std::path::{Path, PathBuf};

use common::{addr, id, init_patch, macro_index, slot};
use enth_patch::canon::identical;
use enth_patch::model::{ModuleInstance, Route, Section};
use enth_patch::{ModSource, ModuleKind, Op, Patch, PatchMeta, Position, apply_all};
use enth_session::presets::{
    self, FAVORITES_FILE, Favorites, Library, MemStore, PresetError, PresetKey, PresetStore,
    Problem, Query, RawPreset, SaveRequest, Source, plan_save, tags,
};
use enth_session::{InstanceState, PresetState, Session};

fn dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/presets")
}

fn meta(name: &str, author: &str, description: &str, tags: &[&str]) -> Op {
    Op::SetMeta {
        meta: PatchMeta {
            name: name.to_owned(),
            author: author.to_owned(),
            description: description.to_owned(),
            tags: tags.iter().map(|t| (*t).to_owned()).collect(),
        },
    }
}

fn set(a: &str, value: f32) -> Op {
    Op::SetParam {
        addr: addr(a),
        value,
    }
}

fn add(section: Section, index: usize, name: &str, kind: ModuleKind) -> Op {
    Op::AddModule {
        at: Position { section, index },
        module: ModuleInstance::new(id(name), kind),
    }
}

/// The fixture presets: (path without the suffix, patch).
fn fixture_cases() -> Vec<(&'static str, Patch)> {
    let build = |ops: Vec<Op>| {
        let mut p = init_patch();
        apply_all(&mut p, &ops).unwrap();
        p
    };
    vec![
        (
            "Init",
            build(vec![meta(
                "Init",
                "enth",
                "One saw and one envelope",
                &["init"],
            )]),
        ),
        (
            "bass/Sub One",
            build(vec![
                add(Section::VoiceFx, 0, "filter1", ModuleKind::Svf),
                set("osc1.semi", -12.0),
                set("filter1.cutoff", 400.0),
                meta(
                    "Sub One",
                    "enth",
                    "A round sub bass with a slow envelope",
                    &["bass", "dark", "mono"],
                ),
            ]),
        ),
        (
            "bass/Wobble",
            build(vec![
                add(Section::VoiceFx, 0, "filter1", ModuleKind::Svf),
                add(Section::Modulators, 1, "lfo1", ModuleKind::Lfo),
                set("filter1.cutoff", 600.0),
                set("filter1.resonance", 40.0),
                Op::InsertRoute {
                    index: 0,
                    route: Route {
                        src: ModSource::Module(id("lfo1")),
                        dst: addr("filter1.cutoff"),
                        depth: 60.0,
                    },
                },
                Op::SetSlot {
                    slot: slot(1),
                    addr: Some(addr("filter1.cutoff")),
                },
                Op::SetMacroName {
                    index: macro_index(1),
                    name: Some("Wobble".to_owned()),
                },
                meta(
                    "Wobble",
                    "enth",
                    "An LFO sweeps the filter; macro 1 is the amount",
                    &["bass", "gritty", "rhythmic"],
                ),
            ]),
        ),
        (
            "pads/Warm Pad",
            build(vec![
                add(Section::VoiceFx, 0, "filter1", ModuleKind::Svf),
                add(Section::Generators, 1, "osc2", ModuleKind::Osc),
                set("osc1.fine", -8.0),
                set("osc2.fine", 8.0),
                set("filter1.cutoff", 1800.0),
                set("env1.attack", 600.0),
                set("env1.release", 1500.0),
                meta(
                    "Warm Pad",
                    "enth",
                    "Two detuned saws behind a soft filter",
                    &["pad", "warm", "wide", "soft"],
                ),
            ]),
        ),
        (
            "pluck/Glass Pluck",
            build(vec![
                add(Section::VoiceFx, 0, "filter1", ModuleKind::Svf),
                set("osc1.semi", 12.0),
                set("filter1.cutoff", 5000.0),
                set("env1.attack", 1.0),
                set("env1.decay", 180.0),
                set("env1.sustain", 0.0),
                meta(
                    "Glass Pluck",
                    "Jo",
                    "A short bright pluck",
                    &["pluck", "bright", "metallic"],
                ),
            ]),
        ),
        (
            "fx/Rain",
            build(vec![
                add(Section::Generators, 1, "noise1", ModuleKind::Noise),
                set("osc1.level", 0.0),
                meta(
                    "Rain",
                    "Jo",
                    "Noise with nothing else, for a wash",
                    &["texture", "noisy", "evolving"],
                ),
            ]),
        ),
    ]
}

fn fixture_path(path: &str) -> PathBuf {
    dir().join(format!("{path}{}", presets::FILE_SUFFIX))
}

fn read_fixtures() -> Vec<RawPreset> {
    fixture_cases()
        .into_iter()
        .map(|(path, _)| RawPreset {
            key: PresetKey::factory(path),
            text: fs::read_to_string(fixture_path(path)).unwrap_or_else(|e| panic!("{path}: {e}")),
        })
        .collect()
}

fn library() -> Library {
    Library::build(read_fixtures(), Favorites::default())
}

fn titles(hits: &[&presets::PresetEntry]) -> Vec<String> {
    hits.iter().map(|e| e.title().to_owned()).collect()
}

// ---------------------------------------------------------------------------------------------------------------
// Format
// ---------------------------------------------------------------------------------------------------------------

#[test]
#[ignore = "writes the fixtures: run on purpose and review the diff"]
fn regenerate() {
    for (path, patch) in fixture_cases() {
        let file = fixture_path(path);
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(file, patch.to_json()).unwrap();
    }
}

#[test]
fn the_fixtures_are_what_this_build_writes() {
    for (path, patch) in fixture_cases() {
        let text = fs::read_to_string(fixture_path(path)).unwrap();
        assert_eq!(text, patch.to_json(), "{path}: the format changed");
    }
}

/// A preset file is exactly `Patch::to_json()`: loading it as a state field and saving gives the same bytes, so the
/// plugin and the browser share one format, and the files are valid saved states.
#[test]
fn a_preset_file_is_a_saved_patch_byte_for_byte() {
    for file in read_fixtures() {
        let patch = presets::parse_preset(&file.key, &file.text).unwrap();
        assert_eq!(patch.to_json(), file.text, "{}", file.key);
        let mut session = Session::default();
        let report = session.load_patch(&file.text);
        assert_eq!(report.errors, [], "{}", file.key);
        assert_eq!(session.save_patch(None), file.text, "{}", file.key);
    }
}

#[test]
fn a_file_from_a_newer_schema_or_damage_is_a_problem_not_a_failure() {
    let mut files = read_fixtures();
    files.push(RawPreset {
        key: PresetKey::user("broken/Not JSON"),
        text: "not json".to_owned(),
    });
    files.push(RawPreset {
        key: PresetKey::user("broken/Newer"),
        text: r#"{"schema": 99}"#.to_owned(),
    });
    let lib = Library::build(files, Favorites::default());
    assert_eq!(lib.entries().len(), fixture_cases().len());
    let bad: Vec<&PresetKey> = lib.problems().iter().map(|p| &p.key).collect();
    assert_eq!(
        bad,
        [
            &PresetKey::user("broken/Newer"),
            &PresetKey::user("broken/Not JSON")
        ]
    );
    assert!(
        lib.problems()
            .iter()
            .all(|p: &Problem| !p.message.is_empty())
    );
    assert_eq!(
        presets::parse_preset(&PresetKey::user("x"), "{")
            .unwrap_err()
            .code(),
        "preset_invalid"
    );
}

#[test]
fn keys_have_one_text_form() {
    for text in ["factory:bass/Sub One", "user:Mine/Leads/Wobble", "user:a:b"] {
        let key = PresetKey::parse(text).unwrap();
        assert_eq!(key.to_text(), text);
        assert_eq!(key.to_string(), text);
    }
    let key = PresetKey::parse("user:Mine/Leads/Wobble").unwrap();
    assert_eq!(
        (key.source, key.name(), key.folder()),
        (Source::User, "Wobble", "Mine/Leads")
    );
    assert_eq!(key.file_name(), "Mine/Leads/Wobble.patch.json");
    assert_eq!(PresetKey::factory("Init").folder(), "");
    for bad in [
        "",
        "user:",
        "user:..",
        "user:a/../b",
        "user:/a",
        "user:a//b",
        "user:a\\b",
        "nobody:x",
        "user:a/.",
        "plain",
    ] {
        assert_eq!(PresetKey::parse(bad), None, "{bad:?}");
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Tags
// ---------------------------------------------------------------------------------------------------------------

fn tagged(name: &str, tag_list: &[&str]) -> PatchMeta {
    PatchMeta {
        name: name.to_owned(),
        tags: tag_list.iter().map(|t| (*t).to_owned()).collect(),
        ..PatchMeta::default()
    }
}

#[test]
fn the_vocabulary_is_small_lower_case_and_disjoint() {
    assert!(tags::TYPES.len() + tags::CHARACTER.len() <= 32, "small");
    for tag in tags::TYPES.iter().chain(&tags::CHARACTER) {
        assert_eq!(*tag, tag.to_lowercase());
        assert!(!tag.contains(char::is_whitespace));
    }
    assert!(tags::TYPES.iter().all(|t| !tags::is_character(t)));
    for ty in [
        "bass", "lead", "pad", "pluck", "keys", "arp", "fx", "texture",
    ] {
        assert!(tags::is_type(ty), "{ty}");
    }
}

#[test]
fn factory_tags_are_one_type_and_a_few_character_words() {
    assert_eq!(
        tags::check_factory(&tagged("Sub", &["bass", "dark"])),
        Vec::<String>::new()
    );
    assert_eq!(
        tags::check_factory(&tagged("Init", &["init"])),
        Vec::<String>::new()
    );
    let problems = |meta: &PatchMeta| tags::check_factory(meta).join("; ");
    assert!(problems(&tagged("", &["bass"])).contains("no name"));
    assert!(problems(&tagged("x", &["dark"])).contains("exactly one type"));
    assert!(problems(&tagged("x", &["bass", "lead"])).contains("exactly one type"));
    assert!(problems(&tagged("x", &["bass", "spicy"])).contains("not in the vocabulary"));
    assert!(problems(&tagged("x", &["bass", "dark", "dark"])).contains("twice"));
    assert!(
        problems(&tagged(
            "x",
            &["bass", "dark", "warm", "soft", "wide", "hard"]
        ))
        .contains("most is 4")
    );
    assert!(problems(&tagged("x", &["Bass"])).contains("not in the vocabulary"));
    assert!(problems(&tagged(&"n".repeat(65), &["bass"])).contains("longer than 64"));
}

#[test]
fn every_fixture_uses_the_vocabulary() {
    for file in read_fixtures() {
        let patch = presets::parse_preset(&file.key, &file.text).unwrap();
        assert_eq!(
            tags::check_factory(&patch.meta),
            Vec::<String>::new(),
            "{}",
            file.key
        );
    }
}

#[test]
fn the_tag_cloud_counts_each_tag_once_per_preset() {
    let lib = library();
    let cloud = lib.tags();
    let count = |t: &str| cloud.iter().find(|(name, _)| name == t).map(|(_, n)| *n);
    assert_eq!(count("bass"), Some(2));
    assert_eq!(count("dark"), Some(1));
    assert_eq!(count("nothing"), None);
    assert!(
        cloud
            .windows(2)
            .all(|w| w[0].1 > w[1].1 || (w[0].1 == w[1].1 && w[0].0 < w[1].0))
    );
    // Case and doubles do not inflate it.
    let mut files = read_fixtures();
    files.push(RawPreset {
        key: PresetKey::user("Odd"),
        text: Patch::init().to_json(),
    });
    let mut odd = Patch::init();
    apply_all(
        &mut odd,
        &[meta("Odd", "", "", &["Bass", "bass", " BASS "])],
    )
    .unwrap();
    files.last_mut().unwrap().text = odd.to_json();
    let cloud = Library::build(files, Favorites::default()).tags();
    assert_eq!(
        cloud.iter().find(|(t, _)| t == "bass").map(|(_, n)| *n),
        Some(3)
    );
}

// ---------------------------------------------------------------------------------------------------------------
// The index and search
// ---------------------------------------------------------------------------------------------------------------

#[test]
fn the_index_is_sorted_folder_then_name_ignoring_case() {
    let lib = library();
    let order: Vec<String> = lib.entries().iter().map(|e| e.key.to_text()).collect();
    assert_eq!(
        order,
        [
            "factory:Init",
            "factory:bass/Sub One",
            "factory:bass/Wobble",
            "factory:fx/Rain",
            "factory:pads/Warm Pad",
            "factory:pluck/Glass Pluck",
        ]
    );
    assert_eq!(
        lib.folders(Source::Factory),
        ["bass", "fx", "pads", "pluck"]
    );
    assert_eq!(lib.folders(Source::User), Vec::<String>::new());
    let entry = lib.get(&PresetKey::factory("bass/Wobble")).unwrap();
    assert_eq!(entry.meta.author, "enth");
    assert_eq!(entry.meta.tags, ["bass", "gritty", "rhythmic"]);
    assert!(lib.get(&PresetKey::user("bass/Wobble")).is_none());
}

#[test]
fn user_presets_follow_factory_ones_and_nested_folders_list_their_parents() {
    let mut files = read_fixtures();
    for path in ["zed", "Mine/Leads/Edge", "mine/Alpha", "Mine/beta"] {
        let mut p = Patch::init();
        apply_all(
            &mut p,
            &[meta(path.rsplit('/').next().unwrap(), "", "", &[])],
        )
        .unwrap();
        files.push(RawPreset {
            key: PresetKey::user(path),
            text: p.to_json(),
        });
    }
    let lib = Library::build(files, Favorites::default());
    let user: Vec<String> = lib
        .entries()
        .iter()
        .filter(|e| e.key.source == Source::User)
        .map(|e| e.key.path.clone())
        .collect();
    // Top level first, then by folder ignoring case (a parent before its children), then by name ignoring case.
    assert_eq!(user, ["zed", "mine/Alpha", "Mine/beta", "Mine/Leads/Edge"]);
    assert_eq!(lib.folders(Source::User), ["Mine", "mine", "Mine/Leads"]);
}

#[test]
fn search_matches_word_beginnings_in_any_order_and_any_field() {
    let lib = library();
    let q = |text: &str| Query {
        text: text.to_owned(),
        ..Query::default()
    };
    // name word beginnings
    assert_eq!(titles(&lib.search(&q("wob"))), ["Wobble"]);
    assert_eq!(titles(&lib.search(&q("pad warm"))), ["Warm Pad"]);
    assert_eq!(titles(&lib.search(&q("warm pad"))), ["Warm Pad"]);
    // the middle of a word does not match
    assert_eq!(titles(&lib.search(&q("obble"))), Vec::<String>::new());
    // tags, case ignored
    assert_eq!(titles(&lib.search(&q("GRITTY"))), ["Wobble"]);
    // folder names
    assert_eq!(titles(&lib.search(&q("fx"))), ["Rain"]);
    // description words
    assert_eq!(titles(&lib.search(&q("lfo sweeps"))), ["Wobble"]);
    // every word has to match something
    assert_eq!(
        titles(&lib.search(&q("wobble pluck"))),
        Vec::<String>::new()
    );
    // punctuation separates words
    assert_eq!(titles(&lib.search(&q("sub-one"))), ["Sub One"]);
    assert_eq!(titles(&lib.search(&q("   "))).len(), 6);
}

#[test]
fn search_ranks_the_name_above_the_tags_above_the_description() {
    let mut files = Vec::new();
    let make = |path: &str, ops: Vec<Op>| {
        let mut p = Patch::init();
        apply_all(&mut p, &ops).unwrap();
        RawPreset {
            key: PresetKey::user(path),
            text: p.to_json(),
        }
    };
    // Browsing order is by name: "Alpha", "Beta", "Gamma", "Zeta".
    files.push(make("a", vec![meta("Alpha", "", "a deep sound", &[])]));
    files.push(make("b", vec![meta("Beta", "", "", &["deep"])]));
    files.push(make("g", vec![meta("Gamma", "", "", &[])]));
    files.push(make("z", vec![meta("Zeta Deep", "", "", &[])]));
    let lib = Library::build(files, Favorites::default());
    let hits = lib.search(&Query {
        text: "deep".to_owned(),
        ..Query::default()
    });
    assert_eq!(titles(&hits), ["Zeta Deep", "Beta", "Alpha"]);
    // An exact word beats a longer word it begins.
    let files = vec![
        make("p", vec![meta("Pads", "", "", &[])]),
        make("q", vec![meta("Pad", "", "", &[])]),
    ];
    let lib = Library::build(files, Favorites::default());
    let hits = lib.search(&Query {
        text: "pad".to_owned(),
        ..Query::default()
    });
    assert_eq!(titles(&hits), ["Pad", "Pads"]);
}

#[test]
fn hash_tags_and_by_filter_and_chips_narrow_the_list() {
    let lib = library();
    let q = |text: &str| Query {
        text: text.to_owned(),
        ..Query::default()
    };
    assert_eq!(titles(&lib.search(&q("#bass"))), ["Sub One", "Wobble"]);
    assert_eq!(titles(&lib.search(&q("#bass #dark"))), ["Sub One"]);
    assert_eq!(titles(&lib.search(&q("#bass sub"))), ["Sub One"]);
    assert_eq!(titles(&lib.search(&q("#nope"))), Vec::<String>::new());
    assert_eq!(titles(&lib.search(&q("by:jo"))), ["Rain", "Glass Pluck"]);
    assert_eq!(titles(&lib.search(&q("by:jo glass"))), ["Glass Pluck"]);
    assert_eq!(titles(&lib.search(&q("BY:ENTH #pad"))), ["Warm Pad"]);
    // The chips are exact, all required, case ignored.
    let chips = Query {
        tags: vec!["BASS".to_owned(), "mono".to_owned()],
        ..Query::default()
    };
    assert_eq!(titles(&lib.search(&chips)), ["Sub One"]);
    let sub = Query {
        tags: vec!["ba".to_owned()],
        ..Query::default()
    };
    assert!(lib.search(&sub).is_empty(), "a chip is a whole tag");
}

#[test]
fn source_folder_and_favorites_filters() {
    let mut store = MemStore::with_factory(read_fixtures());
    let mut mine = Patch::init();
    apply_all(&mut mine, &[meta("Mine Bass", "", "", &["bass"])]).unwrap();
    store.put_user("Mine/Mine Bass", &mine.to_json());
    store
        .set_favorite(&PresetKey::factory("bass/Wobble"), true)
        .unwrap();
    store
        .set_favorite(&PresetKey::user("Mine/Mine Bass"), true)
        .unwrap();
    let lib = store.scan();
    assert_eq!(lib.favorites().len(), 2);
    let with = |f: &dyn Fn(&mut Query)| {
        let mut q = Query::default();
        f(&mut q);
        titles(&lib.search(&q))
    };
    assert_eq!(with(&|q| q.source = Some(Source::User)), ["Mine Bass"]);
    assert_eq!(with(&|q| q.favorites_only = true), ["Wobble", "Mine Bass"]);
    assert_eq!(
        with(&|q| q.folder = Some("bass".to_owned())),
        ["Sub One", "Wobble"]
    );
    assert_eq!(with(&|q| q.folder = Some("Mine".to_owned())), ["Mine Bass"]);
    assert_eq!(
        with(&|q| q.folder = Some("ba".to_owned())),
        Vec::<String>::new()
    );
    assert_eq!(
        with(&|q| {
            q.favorites_only = true;
            q.tags = vec!["bass".to_owned()];
            q.source = Some(Source::Factory);
        }),
        ["Wobble"]
    );
}

#[test]
fn stepping_through_a_list_wraps() {
    let lib = library();
    let list: Vec<&presets::PresetEntry> = lib.entries().iter().collect();
    let key = |i: usize| list[i].key.clone();
    let at = |from: Option<&PresetKey>, delta: isize| {
        presets::step(&list, from, delta).map(|e| e.key.clone())
    };
    assert_eq!(at(Some(&key(0)), 1), Some(key(1)));
    assert_eq!(at(Some(&key(5)), 1), Some(key(0)), "wraps forward");
    assert_eq!(at(Some(&key(0)), -1), Some(key(5)), "wraps back");
    assert_eq!(at(None, 1), Some(key(0)));
    assert_eq!(at(None, -1), Some(key(5)));
    assert_eq!(at(Some(&PresetKey::user("gone")), 1), Some(key(0)));
    assert_eq!(presets::step(&[], None, 1), None);
    // Stepping through a filtered list.
    let bass = lib.search(&Query {
        text: "#bass".to_owned(),
        ..Query::default()
    });
    assert_eq!(
        presets::step(&bass, Some(&PresetKey::factory("bass/Wobble")), 1)
            .map(enth_session::PresetEntry::title),
        Some("Sub One")
    );
}

// ---------------------------------------------------------------------------------------------------------------
// Favorites
// ---------------------------------------------------------------------------------------------------------------

#[test]
fn favorites_are_a_sorted_list_of_keys_with_one_set_of_bytes() {
    let mut fav = Favorites::default();
    assert!(fav.is_empty());
    assert!(fav.set(&PresetKey::user("Mine/Wobble"), true));
    assert!(fav.set(&PresetKey::factory("bass/Sub One"), true));
    assert!(
        !fav.set(&PresetKey::factory("bass/Sub One"), true),
        "already"
    );
    let text = fav.to_json();
    assert_eq!(
        text,
        r#"{"schema":"enth.favorites/1","favorites":["factory:bass/Sub One","user:Mine/Wobble"]}"#
    );
    assert_eq!(Favorites::from_json(&text).unwrap(), fav);
    assert!(fav.set(&PresetKey::factory("bass/Sub One"), false));
    assert!(!fav.set(&PresetKey::factory("bass/Sub One"), false));
    assert_eq!(fav.len(), 1);
    assert!(!fav.contains(&PresetKey::factory("bass/Sub One")));
    assert_eq!(FAVORITES_FILE, "favorites.json");
    assert!(!FAVORITES_FILE.ends_with(presets::FILE_SUFFIX));
}

#[test]
fn a_favorites_file_with_junk_in_it_keeps_what_makes_sense() {
    let text = r#"{"schema":"enth.favorites/9","future":1,"favorites":["user:ok","nonsense","user:../x",7,"factory:a/b"]}"#;
    // The 7 is not a string: the whole file is refused, and the store then starts with none.
    assert!(Favorites::from_json(text).is_err());
    let text = r#"{"schema":"enth.favorites/9","future":1,"favorites":["user:ok","nonsense","user:../x","factory:a/b"]}"#;
    let fav = Favorites::from_json(text).unwrap();
    let keys: Vec<String> = fav.iter().map(PresetKey::to_text).collect();
    assert_eq!(keys, ["factory:a/b", "user:ok"]);
    assert!(Favorites::from_json("{}").unwrap().is_empty());
    assert_eq!(
        Favorites::from_json("nope").unwrap_err().code(),
        "preset_io"
    );
}

// ---------------------------------------------------------------------------------------------------------------
// Saving
// ---------------------------------------------------------------------------------------------------------------

#[test]
fn file_names_are_safe_on_every_platform() {
    let stem = |n: &str| presets::file_stem(n);
    assert_eq!(stem("Sub One").as_deref(), Some("Sub One"));
    assert_eq!(stem("  padded   name  ").as_deref(), Some("padded name"));
    assert_eq!(
        stem("a/b\\c:d*e?f\"g<h>i|j").as_deref(),
        Some("a_b_c_d_e_f_g_h_i_j")
    );
    assert_eq!(stem("tab\there").as_deref(), Some("tab_here"));
    assert_eq!(stem("dots...").as_deref(), Some("dots"));
    assert_eq!(stem("...hidden").as_deref(), Some("hidden"));
    assert_eq!(stem("日本語 🎹").as_deref(), Some("日本語 🎹"));
    assert_eq!(stem("CON").as_deref(), Some("CON_"));
    assert_eq!(stem("nul").as_deref(), Some("nul_"));
    assert_eq!(stem("Com1").as_deref(), Some("Com1_"));
    assert_eq!(stem("lpt9.txt").as_deref(), Some("lpt9.txt_"));
    assert_eq!(stem("COM0").as_deref(), Some("COM0"));
    assert_eq!(stem("Console").as_deref(), Some("Console"));
    assert_eq!(stem(""), None);
    assert_eq!(stem("   "), None);
    assert_eq!(stem("..."), None);
    assert_eq!(stem("???"), None);
    assert_eq!(stem(&"x".repeat(200)).unwrap().chars().count(), 80);
}

#[test]
fn folders_are_cleaned_and_cannot_climb_out() {
    assert_eq!(presets::clean_folder("").unwrap(), "");
    assert_eq!(presets::clean_folder("Leads").unwrap(), "Leads");
    assert_eq!(presets::clean_folder("Mine\\Leads/").unwrap(), "Mine/Leads");
    assert_eq!(presets::clean_folder("/a//b/./c").unwrap(), "a/b/c");
    assert_eq!(presets::clean_folder("a:b/c?").unwrap(), "a_b/c_");
    for bad in ["..", "a/../b", "../x", "a\\..\\b"] {
        assert_eq!(
            presets::clean_folder(bad).unwrap_err().code(),
            "bad_preset_folder",
            "{bad}"
        );
    }
    assert_eq!(
        presets::clean_folder("???").unwrap_err().code(),
        "bad_preset_folder"
    );
}

#[test]
fn user_tags_are_free_but_tidy() {
    let tags =
        |t: &[&str]| presets::clean_tags(&t.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>());
    assert_eq!(tags(&["  Dark ", "dark", "DARK", "", "  "]), ["Dark"]);
    assert_eq!(tags(&["two   words", "x"]), ["two words", "x"]);
    assert_eq!(tags(&["ü", "Ü"]), ["ü"], "case-insensitive for Unicode too");
    let long = "y".repeat(40);
    assert_eq!(tags(&[&long])[0].chars().count(), presets::MAX_TAG_CHARS);
    let many: Vec<String> = (0..30).map(|i| format!("t{i}")).collect();
    assert_eq!(presets::clean_tags(&many).len(), presets::MAX_TAGS);
}

#[test]
fn a_save_plan_names_the_file_and_sets_the_metadata() {
    let current = PatchMeta {
        name: "old".to_owned(),
        author: "me".to_owned(),
        description: "kept".to_owned(),
        tags: vec!["old".to_owned()],
    };
    let plan = plan_save(
        &current,
        &SaveRequest {
            name: "  Big: Pad? ".to_owned(),
            folder: "Mine\\Pads".to_owned(),
            tags: vec!["Pad".to_owned(), "pad".to_owned(), "airy".to_owned()],
            overwrite: false,
        },
    )
    .unwrap();
    assert_eq!(plan.key, PresetKey::user("Mine/Pads/Big_ Pad_"));
    assert_eq!(
        plan.meta.name, "Big: Pad?",
        "the name keeps what the file cannot"
    );
    assert_eq!(plan.meta.tags, ["Pad", "airy"]);
    assert_eq!(
        (plan.meta.author.as_str(), plan.meta.description.as_str()),
        ("me", "kept")
    );
    for (name, code) in [
        ("", "bad_preset_name"),
        ("   ", "bad_preset_name"),
        ("***", "bad_preset_name"),
    ] {
        assert_eq!(
            plan_save(&current, &SaveRequest::new(name))
                .unwrap_err()
                .code(),
            code,
            "{name:?}"
        );
    }
    let long = "n".repeat(65);
    assert_eq!(
        plan_save(&current, &SaveRequest::new(long))
            .unwrap_err()
            .code(),
        "bad_preset_name"
    );
}

#[test]
fn errors_have_codes_and_hints() {
    let key = PresetKey::user("x");
    let all = [
        PresetError::NoUserFolder,
        PresetError::BadName { why: "w" },
        PresetError::BadFolder { why: "w" },
        PresetError::Exists { key: key.clone() },
        PresetError::NotFound { key: key.clone() },
        PresetError::ReadOnly { key: key.clone() },
        PresetError::Invalid {
            key,
            message: "m".to_owned(),
        },
        PresetError::Io {
            what: "w",
            message: "m".to_owned(),
        },
        PresetError::Unsupported { what: "w" },
    ];
    let mut codes = std::collections::BTreeSet::new();
    for e in &all {
        assert!(!e.hint().is_empty() && !e.to_string().is_empty(), "{e:?}");
        assert!(codes.insert(e.code()), "{} is used twice", e.code());
    }
}

// ---------------------------------------------------------------------------------------------------------------
// The in-memory store and Session::save_preset
// ---------------------------------------------------------------------------------------------------------------

fn session_with_sound() -> Session {
    let mut session = Session::new(init_patch());
    session
        .edit(
            vec![
                set("osc1.semi", 7.0),
                meta("Draft", "me", "to save", &["wip"]),
            ],
            None,
        )
        .unwrap();
    session
}

#[test]
fn saving_writes_to_json_and_the_patch_takes_the_name() {
    let mut store = MemStore::with_factory(Vec::new());
    let mut session = session_with_sound();
    let request = SaveRequest {
        name: "Seven Up".to_owned(),
        folder: "Mine".to_owned(),
        tags: vec!["lead".to_owned(), "bright".to_owned()],
        overwrite: false,
    };
    let key = session.save_preset(&mut store, &request).unwrap();
    assert_eq!(key, PresetKey::user("Mine/Seven Up"));
    // The file is exactly the patch's text with the new metadata.
    let text = store.read_text(&key).unwrap();
    assert_eq!(text, session.patch().to_json());
    let saved = presets::parse_preset(&key, &text).unwrap();
    assert_eq!(saved.meta.name, "Seven Up");
    assert_eq!(saved.meta.tags, ["lead", "bright"]);
    assert_eq!(saved.meta.author, "me", "the author is the patch's own");
    assert_eq!(saved.meta.description, "to save");
    assert!(identical(&saved, session.patch()));
    // The session counts as saved, and the bookmark follows.
    assert!(!session.history().is_dirty());
    assert_eq!(
        session.instance().preset,
        Some(PresetState {
            last: Some("user:Mine/Seven Up".to_owned())
        })
    );
    // The rename was an undoable edit.
    assert_eq!(session.undo(), Ok(true));
    assert_eq!(session.patch().meta.name, "Draft");
    // And the library sees it.
    let lib = store.scan();
    assert_eq!(lib.entries().len(), 1);
    assert_eq!(lib.entries()[0].key, key);
}

#[test]
fn saving_never_overwrites_unless_told_to() {
    let mut store = MemStore::with_factory(Vec::new());
    let mut session = session_with_sound();
    let key = session
        .save_preset(&mut store, &SaveRequest::new("Keep"))
        .unwrap();
    let first = store.read_text(&key).unwrap();
    session.edit(vec![set("osc1.semi", -5.0)], None).unwrap();
    let again = session.save_preset(&mut store, &SaveRequest::new("Keep"));
    assert_eq!(again, Err(PresetError::Exists { key: key.clone() }));
    assert_eq!(
        store.read_text(&key).unwrap(),
        first,
        "the file is untouched"
    );
    assert_eq!(session.patch().value(&addr("osc1.semi")), Some(-5.0));
    // A different name is fine; a name that cleans to the same file is the same file.
    session
        .save_preset(&mut store, &SaveRequest::new("Keep 2"))
        .unwrap();
    assert!(
        session
            .save_preset(&mut store, &SaveRequest::new("Keep?"))
            .is_ok()
    );
    assert_eq!(
        session.save_preset(&mut store, &SaveRequest::new("Keep:")),
        Err(PresetError::Exists {
            key: PresetKey::user("Keep_")
        })
    );
    // On purpose it replaces.
    let key = session
        .save_preset(
            &mut store,
            &SaveRequest {
                overwrite: true,
                ..SaveRequest::new("Keep")
            },
        )
        .unwrap();
    assert_ne!(store.read_text(&key).unwrap(), first);
    assert_eq!(store.user_paths(), ["Keep", "Keep 2", "Keep_"]);
}

#[test]
fn a_failed_save_changes_nothing() {
    let mut store = MemStore::with_factory(Vec::new());
    let mut session = session_with_sound();
    let before = session.patch().clone();
    let undo_before = session.history().can_undo();
    store.fail_writes = true;
    let error = session
        .save_preset(&mut store, &SaveRequest::new("Nope"))
        .unwrap_err();
    assert_eq!(error.code(), "preset_io");
    assert!(
        identical(session.patch(), &before),
        "the name was not taken"
    );
    assert_eq!(session.history().can_undo(), undo_before);
    assert!(session.history().is_dirty());
    assert_eq!(session.instance().preset, None);
    // Bad names are refused before the store is asked.
    store.fail_writes = false;
    assert_eq!(
        session
            .save_preset(&mut store, &SaveRequest::new(""))
            .unwrap_err()
            .code(),
        "bad_preset_name"
    );
    assert_eq!(store.user_paths(), Vec::<&str>::new());
}

#[test]
fn the_factory_presets_are_read_only() {
    let mut store = MemStore::with_factory(read_fixtures());
    let key = PresetKey::factory("bass/Wobble");
    let text = store.read_text(&key).unwrap();
    assert_eq!(
        store.write_text(&key, &text, true),
        Err(PresetError::ReadOnly { key: key.clone() })
    );
    assert!(Source::Factory.read_only() && !Source::User.read_only());
    assert_eq!(
        store.read_text(&PresetKey::user("bass/Wobble")),
        Err(PresetError::NotFound {
            key: PresetKey::user("bass/Wobble")
        })
    );
    // A user key cannot climb out of the store either.
    assert_eq!(
        store
            .write_text(&PresetKey::user("../x"), "{}", true)
            .unwrap_err()
            .code(),
        "bad_preset_name"
    );
}

#[test]
fn a_preset_is_loaded_from_a_store_by_key() {
    let mut store = MemStore::with_factory(read_fixtures());
    let mut session = Session::new(init_patch());
    let key = PresetKey::factory("bass/Wobble");
    let loaded = presets::load_from(&mut session, &store, &key).unwrap();
    assert!(loaded.changed && loaded.undoable);
    assert_eq!(session.patch().meta.name, "Wobble");
    assert_eq!(
        session.instance().preset.as_ref().unwrap().last_key(),
        Some(key)
    );
    let missing = presets::load_from(&mut session, &store, &PresetKey::user("gone"));
    assert_eq!(missing.unwrap_err().code(), "preset_not_found");
    store.put_user("bad", "{ not json");
    let bad = presets::load_from(&mut session, &store, &PresetKey::user("bad"));
    assert_eq!(bad.unwrap_err().code(), "preset_invalid");
    assert_eq!(
        session.patch().meta.name,
        "Wobble",
        "a failed load changes nothing"
    );
}

// ---------------------------------------------------------------------------------------------------------------
// The bookmark in the saved state
// ---------------------------------------------------------------------------------------------------------------

#[test]
fn the_bookmark_is_part_of_the_enth_field_and_absent_until_used() {
    let mut state = InstanceState::default();
    assert!(!state.to_json().contains("preset"));
    state.preset = Some(PresetState {
        last: Some("factory:bass/Wobble".to_owned()),
    });
    let text = state.to_json();
    assert!(
        text.ends_with(r#","preset":{"last":"factory:bass/Wobble"}}"#),
        "{text}"
    );
    let (back, version) = InstanceState::from_json(&text).unwrap();
    assert_eq!((back, version), (state, 1), "still version 1");
    // An empty bookmark and a missing one load alike; an old build's field with extra keys loads too.
    let (empty, _) =
        InstanceState::from_json(r#"{"schema":"enth.plugin-state/1","preset":{}}"#).unwrap();
    assert_eq!(empty.preset, Some(PresetState::default()));
    let (extra, _) = InstanceState::from_json(
        r#"{"schema":"enth.plugin-state/1","preset":{"last":"user:x","future":[1]}}"#,
    )
    .unwrap();
    assert_eq!(extra.preset.unwrap().last_key(), Some(PresetKey::user("x")));
    assert_eq!(
        PresetState {
            last: Some("junk".to_owned())
        }
        .last_key(),
        None
    );
}

#[test]
fn loading_a_preset_moves_the_bookmark_and_a_session_keeps_it_across_a_save() {
    let mut session = Session::new(init_patch());
    let wobble = presets::parse_preset(
        &PresetKey::factory("bass/Wobble"),
        &fs::read_to_string(fixture_path("bass/Wobble")).unwrap(),
    )
    .unwrap();
    session
        .load_preset(&wobble, Some(&PresetKey::factory("bass/Wobble")))
        .unwrap();
    let saved = session.save_instance();
    let mut other = Session::new(init_patch());
    let report = other.load_instance(&saved);
    assert_eq!(report.errors, []);
    assert_eq!(
        other
            .instance()
            .preset
            .as_ref()
            .and_then(PresetState::last_key),
        Some(PresetKey::factory("bass/Wobble"))
    );
    // A load without a key forgets the bookmark.
    let init = init_patch();
    session.load_preset(&init, None).unwrap();
    assert_eq!(session.instance().preset, Some(PresetState { last: None }));
    assert!(!session.save_instance().contains("\"last\""));
}

// ---------------------------------------------------------------------------------------------------------------
// The embedded factory presets
// ---------------------------------------------------------------------------------------------------------------

/// Whatever `content/presets` holds when this is built is in the build, loads, uses the vocabulary and has a unique
/// name; nothing is missing. (The directory may be empty: then nothing is embedded.)
#[test]
fn the_embedded_factory_presets_are_the_files_of_content_presets() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../content/presets");
    let mut on_disk = Vec::new();
    collect(&root, &root, &mut on_disk);
    on_disk.sort();
    let embedded: Vec<String> = presets::factory_files()
        .iter()
        .map(|f| f.key.file_name())
        .collect();
    assert_eq!(
        embedded, on_disk,
        "build.rs embeds every .patch.json under content/presets"
    );
    let mut names = std::collections::BTreeSet::new();
    for file in presets::factory_files() {
        assert_eq!(file.key.source, Source::Factory);
        let disk = fs::read_to_string(root.join(file.key.file_name())).unwrap();
        assert_eq!(file.text, disk, "{}", file.key);
        let patch = presets::parse_preset(&file.key, &file.text).unwrap();
        assert_eq!(
            patch.to_json(),
            file.text,
            "{}: not exactly to_json",
            file.key
        );
        assert_eq!(
            tags::check_factory(&patch.meta),
            Vec::<String>::new(),
            "{}",
            file.key
        );
        assert!(
            names.insert(patch.meta.name.to_lowercase()),
            "{}: the name {:?} is taken",
            file.key,
            patch.meta.name
        );
        assert_eq!(presets::factory_text(&file.key), Some(file.text.as_str()));
    }
    assert_eq!(presets::factory_text(&PresetKey::user("Init")), None);
    // The default store serves exactly these.
    let lib = MemStore::default().scan();
    assert_eq!(lib.entries().len() + lib.problems().len(), embedded.len());
    assert_eq!(lib.problems(), []);
}

fn collect(root: &Path, dir: &Path, out: &mut Vec<String>) {
    let Ok(read) = fs::read_dir(dir) else {
        return;
    };
    for entry in read.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        if path.is_dir() {
            collect(root, &path, out);
        } else if name.ends_with(presets::FILE_SUFFIX) {
            let rel = path.strip_prefix(root).unwrap();
            out.push(
                rel.components()
                    .map(|c| c.as_os_str().to_string_lossy().into_owned())
                    .collect::<Vec<_>>()
                    .join("/"),
            );
        }
    }
}

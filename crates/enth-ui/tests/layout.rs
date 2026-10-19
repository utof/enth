//! The layout seam's geometry (lane ED1): layout 1 ("lanes") at the default window, and a small window. The checks
//! of the rules and the built-in files are unit tests in `src/editor/layout.rs`.

use egui::{Rect, pos2, vec2};
use enth_ui::Theme;
use enth_ui::editor::layout::{Layout, LayoutSpec, Part, Region};

fn lanes() -> Layout {
    Layout::new(LayoutSpec::lanes(), &Theme::dark()).expect("lanes is valid")
}

#[test]
fn lanes_at_the_default_window() {
    let theme = Theme::dark();
    let window = Rect::from_min_size(pos2(0.0, 0.0), theme.size.window.default);
    let regions = lanes().regions(window);
    let rect = |r: Region| regions.iter().find(|p| p.region == r).unwrap().rect;
    let gap = theme.space.gap;
    assert_eq!(rect(Region::Header).height(), theme.size.bar.header);
    assert_eq!(rect(Region::Header).min, pos2(gap, gap));
    assert_eq!(rect(Region::Status).bottom(), window.bottom() - gap);
    // Three generator strips with two gaps between them.
    assert_eq!(
        rect(Region::Sources).height(),
        3.0 * theme.size.card.generator_height + 2.0 * gap
    );
    assert_eq!(rect(Region::VoiceFx).width(), theme.size.card.lane_width);
    assert_eq!(rect(Region::Effects).width(), theme.size.card.lane_width);
    assert_eq!(
        rect(Region::Modulators).height(),
        theme.size.editor.band_height
    );
    // Signal order reads left to right along the top of the body: sources, then the filter lane, then effects.
    assert!(rect(Region::Sources).right() < rect(Region::VoiceFx).left());
    assert!(rect(Region::VoiceFx).right() < rect(Region::Effects).left());
    // The modulators sit together, under the signal path; the overview takes what is left.
    assert!(rect(Region::Modulators).top() > rect(Region::Sources).bottom());
    assert!(rect(Region::Overview).top() > rect(Region::Modulators).bottom());
    assert!(rect(Region::Overview).height() > 0.0);
    // Nothing overlaps.
    for (i, a) in regions.iter().enumerate() {
        for b in &regions[i + 1..] {
            assert!(
                !a.rect.intersects(b.rect) || a.rect.intersect(b.rect).area() == 0.0,
                "{:?} overlaps {:?}",
                a.region,
                b.region
            );
        }
    }
    assert_eq!(
        lanes().parts_in(Region::Effects).collect::<Vec<_>>(),
        vec![Part::Voice, Part::Fx]
    );
    assert_eq!(
        lanes().parts_in(Region::Header).collect::<Vec<_>>(),
        vec![Part::Master]
    );
}

#[test]
fn a_small_window_cuts_the_last_fixed_children_first() {
    let theme = Theme::dark();
    let small = Rect::from_min_size(pos2(0.0, 0.0), vec2(1000.0, 400.0));
    let regions = lanes().regions(small);
    for p in &regions {
        assert!(small.contains_rect(p.rect), "{:?} stays inside", p.region);
    }
    let header = regions.iter().find(|p| p.region == Region::Header).unwrap();
    assert_eq!(header.rect.height(), theme.size.bar.header);
}

/// Layout 1 with a lesson pane on the left of the body and the challenge bar and results drawer under the macros:
/// the shape of a Learn or Recreate mode layout.
fn with_reserved_regions() -> String {
    let mut spec: serde_json::Value = serde_json::from_str(&LayoutSpec::lanes().to_json()).unwrap();
    let rows = spec["root"]["rows"].as_array_mut().unwrap();
    // Layout 1's body, at three shares, between the challenge bar and the results drawer, right of the lesson pane.
    let mut body = rows[2].clone();
    body["size"] = serde_json::json!({"share": 3});
    rows[2] = serde_json::json!({"size": {"share": 1}, "cols": [
        {"size": {"token": "size.pane.lesson_width"}, "region": "lesson"},
        {"size": {"share": 1}, "rows": [
            {"size": {"token": "size.bar.macros"}, "region": "challenge_bar"},
            body,
            {"size": {"share": 1}, "region": "results"}
        ]}
    ]});
    spec["name"] = "learn-and-recreate".into();
    spec.to_string()
}

#[cfg(all(feature = "learn", feature = "recreate"))]
#[test]
fn a_learn_and_recreate_layout_places_the_reserved_regions() {
    let theme = Theme::dark();
    let spec = LayoutSpec::from_json(&with_reserved_regions()).unwrap();
    let layout = Layout::new(spec, &theme).expect("the reserved regions are regions");
    let window = Rect::from_min_size(pos2(0.0, 0.0), theme.size.window.default);
    let regions: Vec<Region> = layout.regions(window).iter().map(|p| p.region).collect();
    for r in [Region::Lesson, Region::ChallengeBar, Region::Results] {
        assert!(regions.contains(&r), "{r:?} is placed: {regions:?}");
    }
}

#[cfg(not(feature = "learn"))]
#[test]
fn without_the_learn_feature_a_lesson_region_is_refused() {
    let err = LayoutSpec::from_json(&with_reserved_regions()).unwrap_err();
    assert!(err.0.contains("unknown variant `lesson`"), "{err}");
}

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

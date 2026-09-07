use gt_map::test_util;
use gt_types::DataCategory;
use gt_ui_types::{DataPointRef, HighlightScope, MapHighlight};

fn tpv_point(pi: usize) -> DataPointRef {
    test_util::point_ref(DataCategory::Tpv, pi)
}

/// Every way an element under the pointer loses its own hover label, and the
/// plain hover that keeps it.
#[rstest::rstest]
#[case::plain_hover(None, false, false, true)]
#[case::another_point_pinned(Some(tpv_point(1)), false, false, true)]
#[case::hovered_point_pinned(Some(tpv_point(0)), false, false, false)]
#[case::popup_open(None, true, false, false)]
#[case::compound_label_took_over(None, false, true, false)]
fn the_map_stacks_an_elements_hover_label_unless_something_else_shows_it(
    #[case] sticky: Option<DataPointRef>,
    #[case] any_popup_open: bool,
    #[case] suppress_hover_labels: bool,
    #[case] expected: bool,
) {
    let hovered = tpv_point(0);
    let highlight = MapHighlight {
        hover: Some(HighlightScope::Point(hovered)),
        sticky,
        suppress_hover_labels,
        ..Default::default()
    };
    assert_eq!(
        highlight.shows_hover_label(hovered, any_popup_open),
        expected
    );
}

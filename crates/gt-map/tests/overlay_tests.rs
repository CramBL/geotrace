use egui::ComboBox;
use egui::accesskit;
use egui_kittest::kittest::Queryable as _;
use gt_test_utils::TestHarness;
use gt_types::{DataCategory, FileIdx, PointIdx, TrackIdx, TrackRef};
use gt_ui_types::{DataPointRef, HighlightScope, MapHighlight};

fn gen_marker_point(pi: usize) -> DataPointRef {
    DataPointRef {
        track: TrackRef::new(FileIdx::new(0), TrackIdx::new(0)),
        category: DataCategory::GeneratedMarker,
        point_index: PointIdx::new(pi),
    }
}

fn tpv_point(pi: usize) -> DataPointRef {
    DataPointRef {
        track: TrackRef::new(FileIdx::new(0), TrackIdx::new(0)),
        category: DataCategory::Tpv,
        point_index: PointIdx::new(pi),
    }
}

/// Every way a renderer loses its own hover label, and the plain hover that
/// keeps it.
#[rstest::rstest]
#[case::plain_hover(None, false, false, true)]
#[case::another_point_pinned(Some(tpv_point(1)), false, false, true)]
#[case::hovered_point_pinned(Some(tpv_point(0)), false, false, false)]
#[case::popup_open(None, true, false, false)]
#[case::compound_label_took_over(None, false, true, false)]
fn a_renderer_draws_its_hover_label_unless_something_else_shows_the_point(
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

/// A generated marker's tooltip yields to a TPV tooltip at the same position.
#[rstest::rstest]
#[case::tpv_hovered(Some(HighlightScope::Point(tpv_point(0))), true)]
#[case::marker_hovered(Some(HighlightScope::Point(gen_marker_point(0))), false)]
#[case::nothing_hovered(None, false)]
fn only_a_hovered_fix_is_the_primary_tpv_hover(
    #[case] hover: Option<HighlightScope>,
    #[case] expected: bool,
) {
    let highlight = MapHighlight {
        hover,
        ..Default::default()
    };
    assert_eq!(highlight.primary_hover_is_tpv(), expected);
}

/// `suppress_hover_labels` defaults to false. Renderers show tooltips normally
/// when nothing special is active.
#[test]
fn suppress_hover_labels_defaults_false() {
    let h = MapHighlight::default();
    assert!(!h.suppress_hover_labels);
}

/// Verifies that `any_popup_open()` returns false when no popup has been opened -
/// confirming the egui baseline the guard relies on.
#[test]
fn egui_any_popup_open_false_by_default() {
    let mut harness = TestHarness::builder().ui(|_ui| {});
    harness.run();
    assert!(
        !harness.inner.ctx.any_popup_open(),
        "no popup should be open in an idle UI"
    );
}

/// Opens a ComboBox (which creates an egui Popup) and asserts that
/// `any_popup_open()` returns true while the dropdown is visible.
/// This confirms that the context-menu case the guard suppresses would actually
/// set the flag the guard reads.
#[test]
fn egui_any_popup_open_true_when_combobox_expanded() {
    let items = ["A", "B", "C"];

    let mut harness = TestHarness::builder()
        .size(egui::Vec2::new(300.0, 200.0))
        .ui_state(
            |ui, selected: &mut usize| {
                ComboBox::from_label("Pick")
                    .selected_text(items[*selected])
                    .show_ui(ui, |ui| {
                        for (i, item) in items.iter().enumerate() {
                            ui.selectable_value(selected, i, *item);
                        }
                    });
            },
            0_usize,
        );

    harness.run();

    // Click the combo box to open its popup.
    harness
        .inner
        .get_by_role_and_label(accesskit::Role::ComboBox, "Pick")
        .click();
    harness.run();

    assert!(
        harness.inner.ctx.any_popup_open(),
        "any_popup_open() should be true while the combo box dropdown is open"
    );
}

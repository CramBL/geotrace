use egui::accesskit;
use egui_kittest::{Harness, kittest::Queryable as _};
use gt_types::{
    DataCategory, DataPointRef, FileIdx, HighlightScope, MapHighlight, PointIdx, TrackIdx,
};

fn tpv_point(pi: usize) -> DataPointRef {
    DataPointRef {
        file_index: FileIdx::new(0),
        track_index: TrackIdx::new(0),
        category: DataCategory::Tpv,
        point_index: PointIdx::new(pi),
    }
}

/// Mirrors the guard used in every renderer's hover-tooltip block:
/// ```ignore
/// if highlight.sticky != Some(r) && !ui.ctx().any_popup_open() { … }
/// ```
/// Extracted here so pure-logic tests can assert on the decision without egui.
fn tooltip_guard_passes(highlight: &MapHighlight, r: DataPointRef, any_popup_open: bool) -> bool {
    highlight.sticky != Some(r) && !any_popup_open
}

#[test]
fn tooltip_suppressed_when_sticky_eq_hover() {
    let p = tpv_point(0);
    let h = MapHighlight {
        sticky: Some(p),
        hover: Some(HighlightScope::Point(p)),
        ..Default::default()
    };
    let Some(HighlightScope::Point(r)) = h.hover else {
        panic!("hover not set");
    };
    assert!(
        !tooltip_guard_passes(&h, r, false),
        "tooltip should be suppressed when sticky == hover"
    );
}

#[test]
fn tooltip_shown_when_sticky_differs_from_hover() {
    let p0 = tpv_point(0);
    let p1 = tpv_point(1);
    let h = MapHighlight {
        sticky: Some(p1),
        hover: Some(HighlightScope::Point(p0)),
        ..Default::default()
    };
    let Some(HighlightScope::Point(r)) = h.hover else {
        panic!("hover not set");
    };
    assert!(
        tooltip_guard_passes(&h, r, false),
        "tooltip should show when sticky != hover"
    );
}

#[test]
fn tooltip_shown_when_no_sticky() {
    let p = tpv_point(0);
    let h = MapHighlight {
        sticky: None,
        hover: Some(HighlightScope::Point(p)),
        ..Default::default()
    };
    let Some(HighlightScope::Point(r)) = h.hover else {
        panic!("hover not set");
    };
    assert!(
        tooltip_guard_passes(&h, r, false),
        "tooltip should show when there is no sticky element"
    );
}

#[test]
fn tooltip_suppressed_when_popup_open() {
    let p = tpv_point(5);
    let h = MapHighlight {
        sticky: None,
        hover: Some(HighlightScope::Point(p)),
        ..Default::default()
    };
    let Some(HighlightScope::Point(r)) = h.hover else {
        panic!("hover not set");
    };
    assert!(
        !tooltip_guard_passes(&h, r, true),
        "tooltip should be suppressed when a popup is open"
    );
}

/// Verifies that `any_popup_open()` returns false when no popup has been opened —
/// confirming the egui baseline the guard relies on.
#[test]
fn egui_any_popup_open_false_by_default() {
    let mut harness = Harness::new_ui(|_ui| {});
    harness.run();
    assert!(
        !harness.ctx.any_popup_open(),
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

    let mut harness = Harness::builder()
        .with_size(egui::Vec2::new(300.0, 200.0))
        .build_ui_state(
            |ui, selected: &mut usize| {
                egui::ComboBox::from_label("Pick")
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
        .get_by_role_and_label(accesskit::Role::ComboBox, "Pick")
        .click();
    harness.run();

    assert!(
        harness.ctx.any_popup_open(),
        "any_popup_open() should be true while the combo box dropdown is open"
    );
}

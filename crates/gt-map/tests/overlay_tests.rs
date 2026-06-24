use egui::accesskit;
use egui_kittest::{Harness, kittest::Queryable as _};
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

/// Mirrors the disambiguation-close guard introduced to fix the popup-flash bug:
/// ```ignore
/// if !just_opened_disambig && (area_resp.response.clicked_elsewhere() || esc) {
///     self.disambiguation_candidates = [None; 4];
/// }
/// ```
/// Before the fix, `just_opened_disambig` did not exist, so `clicked_elsewhere()`
/// (which fires on the same frame as the click that opened the popup) immediately
/// cleared the candidates and the popup disappeared in one frame.
#[expect(
    clippy::fn_params_excessive_bools,
    reason = "mirrors the three independent boolean inputs to the guard"
)]
fn disambig_should_close(just_opened: bool, clicked_elsewhere: bool, esc: bool) -> bool {
    !just_opened && (clicked_elsewhere || esc)
}

/// Regression: the click that opens the disambiguation popup also fires
/// `clicked_elsewhere()` on the popup area (the click was on the map, not inside
/// the popup), which used to close it immediately in the same frame.
#[test]
fn disambig_stays_open_on_same_frame_as_click() {
    assert!(
        !disambig_should_close(true, true, false),
        "popup must not close on the frame it was opened, even if clicked_elsewhere fires"
    );
}

#[test]
fn disambig_closes_on_subsequent_clicked_elsewhere() {
    assert!(
        disambig_should_close(false, true, false),
        "popup must close when user clicks outside it on a later frame"
    );
}

#[test]
fn disambig_closes_on_esc() {
    assert!(
        disambig_should_close(false, false, true),
        "popup must close when ESC is pressed"
    );
}

/// Mirrors the hover-suppression guard in `GeneratedMarkerRenderer` that prevents a
/// redundant tooltip from appearing during the one-frame transition into multi-hover
/// mode (when `suppress_hover_labels` hasn't yet caught up):
/// ```ignore
/// let primary_is_tpv = matches!(
///     highlight.hover,
///     Some(HighlightScope::Point(r)) if r.category == DataCategory::Tpv
/// );
/// if !primary_is_tpv && ... { show_tooltip(…) }
/// ```
/// From the second frame onward `suppress_hover_labels` is true and individual
/// tooltips are already suppressed. The compound label takes over.
fn generated_marker_tooltip_allowed(highlight: &MapHighlight) -> bool {
    !matches!(
        highlight.hover,
        Some(HighlightScope::Point(r)) if r.category == DataCategory::Tpv
    )
}

/// Regression: hovering a TPV point that coincides with a generated marker used to
/// show two overlapping tooltips at the same screen position.
#[test]
fn generated_marker_tooltip_suppressed_when_primary_hover_is_tpv() {
    let tpv = tpv_point(0);
    let h = MapHighlight {
        hover: Some(HighlightScope::Point(tpv)),
        ..Default::default()
    };
    assert!(
        !generated_marker_tooltip_allowed(&h),
        "generated marker tooltip must be suppressed when primary hover is a TPV point"
    );
}

#[test]
fn generated_marker_tooltip_shown_when_no_tpv_hovered() {
    let h = MapHighlight {
        hover: None,
        ..Default::default()
    };
    assert!(
        generated_marker_tooltip_allowed(&h),
        "generated marker tooltip must show when no TPV point is the primary hover"
    );
}

#[test]
fn generated_marker_tooltip_shown_when_primary_hover_is_non_tpv() {
    let gm = gen_marker_point(0);
    let h = MapHighlight {
        hover: Some(HighlightScope::Point(gm)),
        ..Default::default()
    };
    assert!(
        generated_marker_tooltip_allowed(&h),
        "generated marker tooltip must show when primary hover is not a TPV point"
    );
}

/// Mirrors the full renderer tooltip guard (items 14/15), which now also checks
/// `suppress_hover_labels` so that individual tooltips are suppressed both when
/// the disambiguation popup is open and when multiple candidates are hovered.
fn tooltip_guard_passes_full(
    highlight: &MapHighlight,
    r: DataPointRef,
    any_popup_open: bool,
) -> bool {
    highlight.sticky != Some(r) && !any_popup_open && !highlight.suppress_hover_labels
}

/// With no popup open and a single hover, the tooltip should show.
#[test]
fn tooltip_shows_when_single_hover_no_popup() {
    let p = tpv_point(0);
    let h = MapHighlight {
        hover: Some(HighlightScope::Point(p)),
        suppress_hover_labels: false,
        ..Default::default()
    };
    assert!(tooltip_guard_passes_full(&h, p, false));
}

/// When `suppress_hover_labels` is set (multiple candidates hovered simultaneously),
/// individual renderer tooltips must not appear so they don't pile on top of each other.
#[test]
fn tooltip_suppressed_when_suppress_hover_labels_set() {
    let p = tpv_point(0);
    let other = gen_marker_point(0);
    let h = MapHighlight {
        hover: Some(HighlightScope::Point(p)),
        hover_candidates: [Some(p), None, None, Some(other)],
        suppress_hover_labels: true,
        ..Default::default()
    };
    assert!(
        !tooltip_guard_passes_full(&h, p, false),
        "tooltip must be suppressed when suppress_hover_labels is true"
    );
}

/// When the disambiguation popup is open, individual renderer tooltips must not
/// appear so they don't overlap the popup (item 14).
#[test]
fn tooltip_suppressed_when_disambig_popup_open() {
    let p = tpv_point(0);
    let h = MapHighlight {
        hover: Some(HighlightScope::Point(p)),
        suppress_hover_labels: true,
        ..Default::default()
    };
    assert!(
        !tooltip_guard_passes_full(&h, p, false),
        "tooltip must be suppressed when disambiguation popup is open"
    );
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

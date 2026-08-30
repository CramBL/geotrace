use egui::{Button, Grid, Label, RichText, WidgetText};
use egui_phosphor::regular::CARET_DOWN as ICON_CARET_DOWN;
use egui_phosphor::regular::CARET_RIGHT as ICON_CARET_RIGHT;
use egui_phosphor::regular::CHECK_SQUARE as ICON_CHECK_SQUARE;
use egui_phosphor::regular::MINUS_SQUARE as ICON_MINUS_SQUARE;
use egui_phosphor::regular::SQUARE as ICON_SQUARE;
use gt_types::{FileMetadata, FixStats, TravelMode};
use gt_ui_types::{DataPointRef, HighlightScope, MapHighlight, MapScope};

use crate::tree::CheckState;

/// A borrowed view of the metadata fields shown in the recording-details UI.
///
/// Lets the side panel (from a [`FileMetadata`]) and the History window (from a
/// `RecordingEntry`) share one presence check and one renderer. A caller sets
/// `identity` to `None` when the identity is shown elsewhere (e.g. the History
/// row already displays it).
#[derive(Debug, Clone, Copy, Default)]
pub struct MetadataView<'a> {
    pub title: Option<&'a str>,
    pub device: Option<&'a str>,
    /// Display form of the declared travel mode (see [`TravelMode::display_name`]).
    pub travel_mode: Option<&'a str>,
    pub identity: Option<&'a str>,
    pub notes: Option<&'a str>,
}

impl<'a> MetadataView<'a> {
    /// View of a loaded file's SDK metadata, with the recording `identity`
    /// supplied separately (it lives outside [`FileMetadata`]).
    pub fn from_file_metadata(metadata: &'a FileMetadata, identity: Option<&'a str>) -> Self {
        Self {
            title: metadata.title.as_deref(),
            device: metadata.device.as_deref(),
            travel_mode: metadata.travel_mode.as_ref().map(TravelMode::display_name),
            identity,
            notes: metadata.notes.as_deref(),
        }
    }
}

/// Kept in step with the fields rendered by [`metadata_detail_rows`].
pub fn has_metadata_details(view: &MetadataView<'_>) -> bool {
    view.title.is_some()
        || view.device.is_some()
        || view.travel_mode.is_some()
        || view.identity.is_some()
        || view.notes.is_some()
}

/// Column and row spacing shared by every recording-details grid.
const DETAIL_GRID_SPACING: [f32; 2] = [12.0, 6.0];

/// A recording-details row: a weak caption and its value, which wraps to the
/// available width. No colon after the caption, per DESIGN.md.
fn detail_row(ui: &mut egui::Ui, caption: &str, value: &str) {
    ui.label(RichText::new(caption).weak());
    // Values select: a reader copies a recording's times, its identity, its
    // device name or a note out of the details dialog.
    ui.add(Label::new(value).wrap().selectable(true));
    ui.end_row();
}

/// Render the present metadata fields as a two-column grid (weak label, value),
/// in a stable order: title, device, travel mode, identity, notes. Values wrap
/// to the available width, so the enclosing (resizable) container governs how
/// much is shown. Renders nothing when the view is empty.
pub fn metadata_detail_rows(ui: &mut egui::Ui, view: &MetadataView<'_>) {
    Grid::new("recording_metadata_grid")
        .num_columns(2)
        .spacing(DETAIL_GRID_SPACING)
        .show(ui, |ui| {
            if let Some(title) = view.title {
                detail_row(ui, "Title", title);
            }
            if let Some(device) = view.device {
                detail_row(ui, "Device", device);
            }
            if let Some(travel_mode) = view.travel_mode {
                detail_row(ui, "Travel mode", travel_mode);
            }
            if let Some(identity) = view.identity {
                // Strip the internal `auto:` marker.
                detail_row(
                    ui,
                    "Identity",
                    gt_loaded_files::display_identity(identity).0,
                );
            }
            if let Some(notes) = view.notes {
                detail_row(ui, "Notes", notes);
            }
        });
}

/// Render a recording's time range and its recorded time as a two-column grid
/// beside [`metadata_detail_rows`]. The recorded time is the sum of the track
/// durations: it is shorter than the time range whenever the recording idled
/// between tracks.
pub fn recording_time_detail_rows(ui: &mut egui::Ui, metadata: &FileMetadata) {
    Grid::new("recording_times_grid")
        .num_columns(2)
        .spacing(DETAIL_GRID_SPACING)
        .show(ui, |ui| {
            detail_row(
                ui,
                "Time range",
                &gt_fmt::format_time_range(metadata.time_range.start, metadata.time_range.end),
            );
            detail_row(
                ui,
                "Recorded time",
                &gt_fmt::format_human_terse_duration(metadata.total_duration),
            );
        });
}

/// Caret icon for an expand/collapse toggle.
pub fn expand_arrow(expanded: bool) -> &'static str {
    if expanded {
        ICON_CARET_DOWN
    } else {
        ICON_CARET_RIGHT
    }
}

/// Width of the tri-state checkbox column, for padding a checkbox-less row so it
/// aligns with the checkboxed sections. Single source of truth for [`tri_checkbox`].
pub fn checkbox_width(ui: &egui::Ui) -> f32 {
    ui.spacing().interact_size.y + 4.0
}

/// Frameless button showing On/Off/Mixed state with Phosphor icons.
///
/// Sized to match egui's standard checkbox: icon at `icon_width`, total hit area
/// `interact_size.y²`.
pub fn tri_checkbox(ui: &mut egui::Ui, state: CheckState) -> egui::Response {
    let icon = match state {
        CheckState::On => ICON_CHECK_SQUARE,
        CheckState::Off => ICON_SQUARE,
        CheckState::Mixed => ICON_MINUS_SQUARE,
    };
    let icon_size = ui.spacing().icon_width + 4.0;
    let side = checkbox_width(ui);
    ui.add(
        Button::new(RichText::new(icon).size(icon_size))
            .frame(false)
            .min_size(egui::vec2(side, side)),
    )
}

/// Paints a translucent highlight behind `rect` to mark map-hover correspondence.
pub fn paint_map_hover_bg(ui: &egui::Ui, rect: egui::Rect, color: egui::Color32) {
    let bg_layer = egui::LayerId::new(egui::Order::Background, egui::Id::new("map_hover_bg"));
    let painter = ui
        .ctx()
        .layer_painter(bg_layer)
        .with_clip_rect(ui.clip_rect());
    painter.rect_filled(rect.expand2(egui::vec2(2.0, 1.0)), 3.0, color);
}

/// Renders the colour-coded fix-percentage label followed by the remaining
/// tooltip details (time without fix, loss count, max gap).
///
/// Disables egui's automatic item spacing so the details' own `  ·  ` joiner
/// (or empty string at 100% fix) renders without an extra gap.
pub fn fix_stats_tooltip_row(ui: &mut egui::Ui, stats: FixStats) {
    let pct_color =
        gt_ui_theme::fix_quality_color(gt_fmt::fix_percentage(stats), ui.visuals().dark_mode);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        ui.colored_label(pct_color, gt_fmt::format_fix_percentage(stats));
        ui.label(gt_fmt::format_fix_tooltip_details(stats));
    });
}

/// A selectable, sticky, double-click-to-focus row for a single data point.
///
/// Handles hover highlight, click-to-pin, and double-click map centering
/// uniformly across all point-list categories (TPV, satellite, markers).
pub fn point_item_row(
    ui: &mut egui::Ui,
    point_ref: DataPointRef,
    label: impl Into<WidgetText>,
    lat_lon: Option<(f64, f64)>,
    scope: MapScope<'_>,
    highlight: &mut MapHighlight,
    requests: &mut PointClickRequests<'_>,
) {
    let is_sticky = highlight.sticky.is_some_and(|r| r == point_ref);
    let response = ui.selectable_label(is_sticky, label);
    if response.hovered() {
        highlight.hover = Some(HighlightScope::Point(point_ref));
    }
    apply_point_click(
        ui, &response, point_ref, lat_lon, scope, highlight, requests,
    );
}

/// What a clicked point row requests from the app, consumed on the same frame.
pub struct PointClickRequests<'a> {
    pub map_center: &'a mut Option<(f64, f64)>,
    pub popup_pos: &'a mut Option<egui::Pos2>,
}

/// The click reactions every point row shares (here and in the query window's
/// match tables): clicking pins the point's map popup right of the containing
/// panel at the row's height, clicking again unpins, double-clicking centers
/// the map on the point.
///
/// Pinning is gated by [`MapHighlight::toggle_sticky_if_drawn`], so only a point
/// the map draws can be pinned. Double-click centers the map either way.
///
/// `lat_lon` is `None` for a fix of a track with no geometry: it is drawn
/// nowhere, so there is nothing to centre on.
pub fn apply_point_click(
    ui: &egui::Ui,
    response: &egui::Response,
    point_ref: DataPointRef,
    lat_lon: Option<(f64, f64)>,
    scope: MapScope<'_>,
    highlight: &mut MapHighlight,
    requests: &mut PointClickRequests<'_>,
) {
    if response.clicked() && highlight.toggle_sticky_if_drawn(scope, point_ref) {
        *requests.popup_pos = Some(egui::pos2(ui.clip_rect().max.x + 8.0, response.rect.min.y));
    }
    if response.double_clicked() {
        *requests.map_center = lat_lon;
    }
}

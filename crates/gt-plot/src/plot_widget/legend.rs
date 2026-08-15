//! The draggable file-style legend overlay and its line-style swatches.

use egui::{Area, Button, Color32, Frame, Label, RichText};
use egui_phosphor::regular::ARROW_LINE_UP_LEFT as ICON_ARROW_LINE_UP_LEFT;
use egui_phosphor::regular::CARET_DOWN as ICON_CARET_DOWN;
use egui_phosphor::regular::CARET_RIGHT as ICON_CARET_RIGHT;
use egui_phosphor::regular::DOTS_SIX as ICON_DOTS_SIX;
use egui_plot::LineStyle;
use gt_loaded_files::RecordingNames;

use super::PlotState;
use super::recording_name;
use super::style::file_line_style;

/// Default legend overlay position, anchored just inside the plot's top-left
/// corner.
pub const LEGEND_DOCK_OFFSET: egui::Vec2 = egui::vec2(10.0, 10.0);
/// Sub-pixel tolerance for [`legend_is_docked`]'s "is this exactly the dock
/// position" check - not to be confused with [`LEGEND_DOCK_SNAP_RADIUS`],
/// the much larger radius used to *move* the legend onto the dock position.
const LEGEND_DOCK_POSITION_TOLERANCE: f32 = 1.0;
/// Background opacity of the file-style legend overlay, matching the default
/// `background_alpha` of egui_plot's built-in legend.
const LEGEND_BACKGROUND_ALPHA: f32 = 0.75;
/// Minimum distance the dragged legend keeps from the plot edges.
const LEGEND_EDGE_MARGIN: f32 = 6.0;
/// Distance from the docked top-left position within which a dragged legend
/// snaps back to docking, so dropping it near the corner re-docks it without
/// requiring a click on the re-dock button.
const LEGEND_DOCK_SNAP_RADIUS: f32 = 32.0;
const LEGEND_AREA_ID_SALT: &str = "plot_file_legend_overlay";
/// Dimensions of the line-style swatch painted next to each legend entry.
const SWATCH_SIZE: egui::Vec2 = egui::vec2(26.0, 10.0);
const SWATCH_STROKE_WIDTH: f32 = 2.0;
/// Gap between dashes as a fraction of the dash length.
const SWATCH_DASH_GAP_RATIO: f32 = 0.62;
const SWATCH_DOT_RADIUS: f32 = 1.7;
fn paint_line_style_swatch(ui: &mut egui::Ui, style: LineStyle, color: Color32) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(SWATCH_SIZE, egui::Sense::hover());
    let y = rect.center().y;
    let start = egui::pos2(rect.left(), y);
    let end = egui::pos2(rect.right(), y);
    let painter = ui.painter();
    match style {
        LineStyle::Solid => {
            painter.line_segment([start, end], egui::Stroke::new(SWATCH_STROKE_WIDTH, color));
        }
        LineStyle::Dashed { length } => {
            painter.extend(egui::Shape::dashed_line(
                &[start, end],
                egui::Stroke::new(SWATCH_STROKE_WIDTH, color),
                length,
                length * SWATCH_DASH_GAP_RATIO,
            ));
        }
        LineStyle::Dotted { spacing } => {
            painter.extend(egui::Shape::dotted_line(
                &[start, end],
                color,
                spacing,
                SWATCH_DOT_RADIUS,
            ));
        }
    }
    response
}

pub(super) fn show_file_legend_overlay(
    ui: &egui::Ui,
    names: &RecordingNames,
    visible_files: &[usize],
    plot_rect: egui::Rect,
    state: &mut PlotState,
) -> Option<usize> {
    if visible_files.len() < 2 {
        return None;
    }

    let mut redock_requested = false;
    let show_redock_icon = !legend_is_docked(state.file_legend_offset);
    let legend_id = ui.id().with(LEGEND_AREA_ID_SALT);
    let drag_bg_size = state.file_legend_size;
    let area = Area::new(legend_id)
        // Windows draw at this order too, so a focused window stacks above the
        // legend, which still draws above the panel-hosted plot.
        .order(egui::Order::Middle)
        .movable(false)
        .current_pos(plot_rect.min + state.file_legend_offset)
        .show(ui.ctx(), |ui| {
            // Selectable labels also sense drag (for text selection) and
            // would win the hit-test over `drag_response` below.
            ui.style_mut().interaction.selectable_labels = false;

            // Drag-sense the whole body first (bottom z-order) so buttons
            // on top still get their clicks.
            let drag_rect = egui::Rect::from_min_size(ui.cursor().min, drag_bg_size);
            let drag_response =
                ui.interact(drag_rect, legend_id.with("drag_bg"), egui::Sense::drag());

            let hovered_file = Frame::default()
                .inner_margin(egui::Margin::symmetric(8, 4))
                .corner_radius(ui.visuals().window_corner_radius)
                .fill(ui.visuals().extreme_bg_color)
                .stroke(ui.visuals().window_stroke())
                .multiply_with_opacity(LEGEND_BACKGROUND_ALPHA)
                .show(ui, |ui| {
                    let mut hovered_file = None;
                    ui.vertical(|ui| {
                        ui.horizontal(|ui| {
                            let dock_btn_size = egui::vec2(
                                ui.spacing().interact_size.y,
                                ui.spacing().interact_size.y,
                            );
                            if show_redock_icon
                                && ui
                                    .add_sized(dock_btn_size, Button::new(ICON_ARROW_LINE_UP_LEFT))
                                    .on_hover_text("Re-dock legend to top-left")
                                    .clicked()
                            {
                                redock_requested = true;
                            }
                            ui.add_sized(
                                dock_btn_size,
                                Label::new(RichText::new(ICON_DOTS_SIX).weak()),
                            )
                            .on_hover_cursor(egui::CursorIcon::Grab)
                            .on_hover_text("Drag to move legend");
                            let fold_icon = if state.file_legend_collapsed {
                                ICON_CARET_RIGHT
                            } else {
                                ICON_CARET_DOWN
                            };
                            if ui
                                .small_button(fold_icon)
                                .on_hover_text(if state.file_legend_collapsed {
                                    "Expand legend"
                                } else {
                                    "Collapse legend"
                                })
                                .clicked()
                            {
                                state.file_legend_collapsed = !state.file_legend_collapsed;
                            }
                        });
                        if !state.file_legend_collapsed {
                            for &fi in visible_files {
                                let row = ui.horizontal(|ui| {
                                    let style = file_line_style(fi);
                                    let file_name = recording_name(names, fi);
                                    let swatch = paint_line_style_swatch(
                                        ui,
                                        style,
                                        ui.visuals().text_color(),
                                    );
                                    let name = ui.label(RichText::new(file_name).small());
                                    swatch.hovered() || name.hovered()
                                });
                                if row.response.hovered() || row.inner {
                                    hovered_file = Some(fi);
                                }
                            }
                        }
                    });
                    hovered_file
                })
                .inner;

            (hovered_file, drag_response)
        });

    let (hovered_file, drag_response) = area.inner;
    state.file_legend_size = area.response.rect.size();

    if drag_response.dragged() {
        state.file_legend_offset += ui.ctx().input(|i| i.pointer.delta());
    }

    state.file_legend_offset = resolve_legend_offset(
        state.file_legend_offset,
        state.file_legend_size,
        plot_rect,
        redock_requested,
        drag_response.drag_stopped(),
    );
    hovered_file
}

/// Clamps `offset` to the plot's edges, then snaps it to [`LEGEND_DOCK_OFFSET`]
/// if redocking was requested or the drag just ended near that corner.
fn resolve_legend_offset(
    offset: egui::Vec2,
    legend_size: egui::Vec2,
    plot_rect: egui::Rect,
    redock_requested: bool,
    drag_released: bool,
) -> egui::Vec2 {
    let max_x = (plot_rect.width() - legend_size.x - LEGEND_EDGE_MARGIN).max(LEGEND_EDGE_MARGIN);
    let max_y = (plot_rect.height() - legend_size.y - LEGEND_EDGE_MARGIN).max(LEGEND_EDGE_MARGIN);
    let clamped = egui::vec2(
        offset.x.clamp(LEGEND_EDGE_MARGIN, max_x),
        offset.y.clamp(LEGEND_EDGE_MARGIN, max_y),
    );
    // Snap only on release, so dragging away from the dock isn't pulled
    // straight back mid-drag.
    let near_dock =
        drag_released && (clamped - LEGEND_DOCK_OFFSET).length() < LEGEND_DOCK_SNAP_RADIUS;
    if redock_requested || near_dock {
        LEGEND_DOCK_OFFSET
    } else {
        clamped
    }
}

/// Whether `offset` is close enough to [`LEGEND_DOCK_OFFSET`] to be considered docked.
pub fn legend_is_docked(offset: egui::Vec2) -> bool {
    (offset.x - LEGEND_DOCK_OFFSET.x).abs() < LEGEND_DOCK_POSITION_TOLERANCE
        && (offset.y - LEGEND_DOCK_OFFSET.y).abs() < LEGEND_DOCK_POSITION_TOLERANCE
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_plot_rect() -> egui::Rect {
        egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(400.0, 300.0))
    }

    #[test]
    fn file_legend_overlay_draws_below_floating_windows() {
        let ctx = egui::Context::default();
        let names = RecordingNames::default();
        let mut state = PlotState::default();
        let mut legend_id = None;
        let window_id = egui::Id::new("test window");

        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            show_file_legend_overlay(ui, &names, &[0, 1], test_plot_rect(), &mut state);
            legend_id = Some(ui.id().with(LEGEND_AREA_ID_SALT));
            egui::Window::new("Settings")
                .id(window_id)
                .show(ui.ctx(), |ui| {
                    ui.label("window body");
                });
        });
        // Dropping a `TexturesDelta` with unapplied deltas panics.
        output.textures_delta.clear();

        // Back-to-front, so a higher index draws on top.
        let layers: Vec<egui::LayerId> = ctx.memory(|memory| memory.layer_ids().collect());
        let legend_id = legend_id.expect("the legend renders with two visible files");
        let position = |wanted: egui::Id| {
            layers
                .iter()
                .position(|layer| layer.id == wanted)
                .expect("layer is registered")
        };
        let legend_position = position(legend_id);
        assert_eq!(
            layers.get(legend_position).map(|layer| layer.order),
            Some(egui::Order::Middle),
            "the legend draws above the panel-hosted plot"
        );
        assert!(
            legend_position < position(window_id),
            "the legend draws below the window, got {layers:?}"
        );
    }

    #[test]
    fn resolve_legend_offset_clamps_to_plot_edges() {
        let legend_size = egui::vec2(100.0, 50.0);
        let offset = resolve_legend_offset(
            egui::vec2(-50.0, 1000.0),
            legend_size,
            test_plot_rect(),
            false,
            false,
        );
        assert!((offset.x - LEGEND_EDGE_MARGIN).abs() < f32::EPSILON);
        assert!((offset.y - (300.0 - legend_size.y - LEGEND_EDGE_MARGIN)).abs() < f32::EPSILON);
    }

    #[test]
    fn resolve_legend_offset_redocks_on_explicit_request() {
        let offset = resolve_legend_offset(
            egui::vec2(200.0, 150.0),
            egui::vec2(100.0, 50.0),
            test_plot_rect(),
            true,
            false,
        );
        assert_eq!(offset, LEGEND_DOCK_OFFSET);
    }

    #[test]
    fn resolve_legend_offset_snaps_to_dock_on_release_near_corner_only() {
        let legend_size = egui::vec2(100.0, 50.0);
        let near_dock = LEGEND_DOCK_OFFSET + egui::vec2(LEGEND_DOCK_SNAP_RADIUS - 1.0, 0.0);

        let mid_drag =
            resolve_legend_offset(near_dock, legend_size, test_plot_rect(), false, false);
        assert_eq!(mid_drag, near_dock, "must not snap before drag release");

        let released = resolve_legend_offset(near_dock, legend_size, test_plot_rect(), false, true);
        assert_eq!(
            released, LEGEND_DOCK_OFFSET,
            "must snap once released near the dock"
        );
    }

    #[test]
    fn resolve_legend_offset_does_not_snap_when_far_from_dock() {
        let legend_size = egui::vec2(100.0, 50.0);
        let far = egui::vec2(200.0, 150.0);
        let offset = resolve_legend_offset(far, legend_size, test_plot_rect(), false, true);
        assert_eq!(offset, far);
    }
}

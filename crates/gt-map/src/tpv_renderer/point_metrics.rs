use egui::{Grid, RichText};
use gt_fmt::UTC_SECOND_FORMAT;
use gt_sky::SkyPlotSize;
use gt_types::coordinates::{Coordinate, RecordedCoordinate};
use gt_types::{LoadedTrack, NavPoint, PlacedPoint, PointIdx, ResolvedPosition};
use gt_ui_theme::{DEGREE_SIGN, EM_DASH};
use gt_ui_types::{DRAWN_AT_CAPTION, INTERPOLATED_POSITION_NOTE};
use uom::si::angle::degree;

use super::point_summary::PointSummary;
use super::satellite_tables;
use super::sky_context::{self, SkyHost, SkySection};

/// Where the map draws a fix, for the surfaces that state it beside the
/// coordinates the receiver recorded.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum FixPlacement {
    Placed(ResolvedPosition),
    /// The fix belongs to a track with no geometry, so the map draws it
    /// nowhere.
    TrackWithoutGeometry,
}

impl FixPlacement {
    pub(crate) fn resolve(track: &LoadedTrack, point_index: PointIdx) -> Self {
        track
            .placed_points()
            .and_then(|placed| placed.get(point_index.as_usize()))
            .map_or(Self::TrackWithoutGeometry, |point| {
                Self::Placed(point.resolved())
            })
    }
}

pub(super) struct PointMetrics<'a> {
    pub(super) point: &'a NavPoint,
    pub(super) recording_name: Option<&'a str>,
    pub(super) placement: FixPlacement,
}

impl PointMetrics<'_> {
    pub(super) fn pinned_ui(&self, ui: &mut egui::Ui) {
        Grid::new("sticky_tpv_basic")
            .num_columns(2)
            .max_col_width(
                (ui.available_width() - METRIC_LABEL_WIDTH_PX).max(METRIC_LABEL_WIDTH_PX),
            )
            .show(ui, |ui| {
                recording_row_ui(ui, self.recording_name);
                position_rows_ui(ui, self.point, self.placement);
                self.motion_rows_ui(ui);
            });
    }

    fn motion_rows_ui(&self, ui: &mut egui::Ui) {
        let fix = self.point;
        ui.label("Speed");
        match fix.tpv.velocity_kmh() {
            Some(v) => ui.label(format!("{:.1} km/h", v)),
            None => ui.label(EM_DASH),
        };
        ui.end_row();

        ui.label("Heading");
        match fix.tpv.heading() {
            Some(h) => ui.label(format!("{:.1}{DEGREE_SIGN}", h.get::<degree>())),
            None => ui.label(EM_DASH),
        };
        ui.end_row();

        if let Some(eph) = fix.tpv.eph_m() {
            ui.label("Accuracy");
            ui.label(format!("±{eph:.1}m"));
            ui.end_row();
        }

        PointSummary(fix).dop_row_ui(ui);
    }
}

/// `recording_name` fills the table's recording row. It is `None` while a
/// single file is loaded.
pub(crate) fn show_hover_table(
    ui: &mut egui::Ui,
    point: PlacedPoint<'_>,
    sky: &SkySection<'_>,
    recording_name: Option<&str>,
) {
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| hover_grid_ui(ui, point, recording_name));
        if !matches!(sky, SkySection::TrackWithoutReports) {
            ui.add_space(12.0);
            ui.vertical(|ui| {
                sky_context::sky_section_ui(
                    ui,
                    sky,
                    SkyPlotSize::Compact,
                    None,
                    SkyHost {
                        point: point.fix,
                        identities_in_table: false,
                    },
                )
            });
        }
    });
}

fn hover_grid_ui(ui: &mut egui::Ui, point: PlacedPoint<'_>, recording_name: Option<&str>) {
    let fix = point.fix;
    let metrics = PointMetrics {
        point: fix,
        recording_name,
        placement: FixPlacement::Placed(point.resolved()),
    };
    Grid::new("hover_grid")
        .striped(true)
        .num_columns(2)
        .show(ui, |ui| {
            recording_row_ui(ui, recording_name);

            ui.horizontal(|ui| {
                ui.label("Time");
                PointSummary(fix).summary_ui(ui);
            });
            ui.label(fix.tpv.time().utc().format(UTC_SECOND_FORMAT).to_string());
            ui.end_row();

            position_rows_ui(ui, fix, metrics.placement);

            metrics.motion_rows_ui(ui);
            satellite_tables::show_satellite_rows(ui, fix);
        });
}

fn position_rows_ui(ui: &mut egui::Ui, fix: &NavPoint, placement: FixPlacement) {
    if fix.has_unreported_position() {
        ui.label("Position");
        ui.label(EM_DASH);
        ui.end_row();
    } else {
        recorded_coordinate_row_ui(ui, "Lat", fix.tpv.lat());
        recorded_coordinate_row_ui(ui, "Lon", fix.tpv.lon());
    }
    match placement {
        FixPlacement::Placed(resolved) => drawn_at_row_ui(ui, fix, resolved),
        FixPlacement::TrackWithoutGeometry => {
            ui.label("Not drawn");
            ui.label("No valid coordinate in this track");
            ui.end_row();
        }
    }
}

fn recording_row_ui(ui: &mut egui::Ui, recording_name: Option<&str>) {
    if let Some(name) = recording_name {
        ui.label("Recording");
        ui.label(name);
        ui.end_row();
    }
}

fn recorded_coordinate_row_ui<C: Coordinate>(
    ui: &mut egui::Ui,
    caption: &str,
    recorded: RecordedCoordinate<C>,
) {
    ui.label(caption);
    let degrees = RichText::new(recorded.to_degrees_with_hemisphere());
    match recorded {
        RecordedCoordinate::Valid(_) => ui.label(degrees),
        RecordedCoordinate::Invalid(_) => {
            ui.label(degrees.color(gt_ui_theme::warning_amber(ui.visuals().dark_mode)))
        }
    };
    ui.end_row();
}

/// Derived display coordinates remain separate from recorded values.
fn drawn_at_row_ui(ui: &mut egui::Ui, fix: &NavPoint, resolved: ResolvedPosition) {
    let ResolvedPosition::Interpolated(projected) = resolved else {
        return;
    };
    let (latitude, longitude) = projected.coordinates();
    ui.label(DRAWN_AT_CAPTION);
    ui.vertical(|ui| {
        ui.label(format!(
            "{}  {}",
            latitude.to_degrees_with_hemisphere(),
            longitude.to_degrees_with_hemisphere()
        ));
        let note = if fix.has_unreported_position() {
            "Derived from recorded positions"
        } else {
            INTERPOLATED_POSITION_NOTE
        };
        ui.label(RichText::new(note).weak().small());
    });
    ui.end_row();
}

const METRIC_LABEL_WIDTH_PX: f32 = 76.0;

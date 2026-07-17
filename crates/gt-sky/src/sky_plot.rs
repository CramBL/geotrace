use egui::{Align2, FontId, Pos2, Sense, Shape, Stroke, Vec2};

use gt_types::satellites::{Satellite, Satellites};

use crate::projection;
use crate::style;

/// The two rendered sizes of the sky plot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkyPlotSize {
    /// Hover-badge size: no elevation labels, north label only.
    Compact,
    /// Sticky-popup size: full cardinal and elevation ring labels.
    Full,
}

impl SkyPlotSize {
    const fn diameter(self) -> f32 {
        match self {
            Self::Compact => style::COMPACT_DIAMETER_PX,
            Self::Full => style::FULL_DIAMETER_PX,
        }
    }

    const fn rim_margin(self) -> f32 {
        match self {
            Self::Compact => style::COMPACT_RIM_MARGIN_PX,
            Self::Full => style::FULL_RIM_MARGIN_PX,
        }
    }

    const fn mark_scale(self) -> f32 {
        match self {
            Self::Compact => style::COMPACT_MARK_SCALE,
            Self::Full => 1.0,
        }
    }
}

/// The polar satellite view for one satellite report: north up, azimuth
/// clockwise, horizon at the rim, zenith at the center.
///
/// Satellites in the fix render as filled dots, tracked-only satellites as
/// hollow outlines, both in their constellation's themed color with the dot
/// radius encoding the signal-quality tier. Satellites without azimuth or
/// elevation cannot be placed and are surfaced as a count line under the
/// plot instead.
pub struct SkyPlot<'a> {
    satellites: &'a Satellites,
    size: SkyPlotSize,
    elevation_mask_deg: Option<f32>,
}

impl<'a> SkyPlot<'a> {
    pub fn new(satellites: &'a Satellites, size: SkyPlotSize) -> Self {
        Self {
            satellites,
            size,
            elevation_mask_deg: None,
        }
    }

    /// Draws the elevation mask as a dashed ring. Satellites below the mask
    /// stay visible - the ring is context, not a filter.
    pub fn with_elevation_mask_deg(self, mask_deg: f32) -> Self {
        Self {
            elevation_mask_deg: Some(mask_deg),
            ..self
        }
    }

    pub fn ui(&self, ui: &mut egui::Ui) -> egui::Response {
        let Self {
            satellites,
            size,
            elevation_mask_deg,
        } = *self;

        ui.vertical(|ui| {
            status_line_ui(ui, satellites, size);

            let diameter = size.diameter();
            let (rect, response) = ui.allocate_exact_size(Vec2::splat(diameter), Sense::hover());
            if ui.is_rect_visible(rect) {
                let center = rect.center();
                let radius = diameter / 2.0 - size.rim_margin();
                paint_grid(ui, center, radius, size);
                if let Some(mask_deg) = elevation_mask_deg {
                    paint_mask_ring(ui, center, radius, mask_deg);
                }
                paint_marks(ui, center, radius, satellites, size);
            }

            unplaceable_line_ui(ui, satellites);
            response
        })
        .inner
    }
}

/// "9 of 14 in fix" above the plot.
fn status_line_ui(ui: &mut egui::Ui, satellites: &Satellites, size: SkyPlotSize) {
    let text = format!(
        "{} of {} in fix",
        satellites.fix_count(),
        satellites.satellite_count()
    );
    match size {
        SkyPlotSize::Compact => ui.label(egui::RichText::new(text).small()),
        SkyPlotSize::Full => ui.label(text),
    };
}

/// "2 satellites without sky position" under the plot; nothing when all
/// satellites are placeable.
fn unplaceable_line_ui(ui: &mut egui::Ui, satellites: &Satellites) {
    let unplaceable = satellites
        .satellites()
        .filter(|satellite| projection::mark_position(satellite).is_none())
        .count();
    let text = match unplaceable {
        0 => return,
        1 => "1 satellite without sky position".to_owned(),
        n => format!("{n} satellites without sky position"),
    };
    ui.label(egui::RichText::new(text).weak().small());
}

fn paint_grid(ui: &egui::Ui, center: Pos2, radius: f32, size: SkyPlotSize) {
    let painter = ui.painter();
    let visuals = ui.visuals();
    let grid_stroke = Stroke::new(style::GRID_STROKE_WIDTH_PX, visuals.window_stroke.color);
    let label_color = visuals.weak_text_color();

    // Horizon rim plus the inner elevation rings.
    painter.circle_stroke(center, radius, grid_stroke);
    for elevation_deg in style::GRID_RING_ELEVATIONS_DEG {
        let ring_radius = radius * projection::unit_disc_radius(elevation_deg);
        painter.circle_stroke(center, ring_radius, grid_stroke);
        if size == SkyPlotSize::Full {
            painter.text(
                center
                    + Vec2::new(
                        style::ELEVATION_LABEL_OFFSET_X_PX,
                        -ring_radius - style::ELEVATION_LABEL_OFFSET_Y_PX,
                    ),
                Align2::LEFT_BOTTOM,
                format!("{elevation_deg:.0}{}", gt_ui_theme::DEGREE_SIGN),
                FontId::proportional(style::ELEVATION_LABEL_FONT_SIZE),
                label_color,
            );
        }
    }

    // Cardinal spokes through the center.
    painter.line_segment(
        [
            center - Vec2::new(radius, 0.0),
            center + Vec2::new(radius, 0.0),
        ],
        grid_stroke,
    );
    painter.line_segment(
        [
            center - Vec2::new(0.0, radius),
            center + Vec2::new(0.0, radius),
        ],
        grid_stroke,
    );

    // North up. The compact size labels north only and marks the other
    // cardinals with rim ticks.
    let cardinals: &[(&str, Vec2)] = match size {
        SkyPlotSize::Full => &[
            ("N", Vec2::new(0.0, -1.0)),
            ("E", Vec2::new(1.0, 0.0)),
            ("S", Vec2::new(0.0, 1.0)),
            ("W", Vec2::new(-1.0, 0.0)),
        ],
        SkyPlotSize::Compact => &[("N", Vec2::new(0.0, -1.0))],
    };
    let (font_size, label_offset) = match size {
        SkyPlotSize::Full => (
            style::FULL_CARDINAL_FONT_SIZE,
            style::FULL_CARDINAL_LABEL_OFFSET_PX,
        ),
        SkyPlotSize::Compact => (
            style::COMPACT_CARDINAL_FONT_SIZE,
            style::COMPACT_CARDINAL_LABEL_OFFSET_PX,
        ),
    };
    for (label, direction) in cardinals {
        painter.text(
            center + *direction * (radius + label_offset),
            Align2::CENTER_CENTER,
            *label,
            FontId::proportional(font_size),
            label_color,
        );
    }
    if size == SkyPlotSize::Compact {
        for direction in [
            Vec2::new(1.0, 0.0),
            Vec2::new(0.0, 1.0),
            Vec2::new(-1.0, 0.0),
        ] {
            painter.line_segment(
                [
                    center + direction * radius,
                    center + direction * (radius + style::COMPACT_CARDINAL_TICK_PX),
                ],
                grid_stroke,
            );
        }
    }
}

/// The elevation mask as a dashed ring, approximated by a dashed polyline
/// over the circle.
fn paint_mask_ring(ui: &egui::Ui, center: Pos2, radius: f32, mask_deg: f32) {
    let ring_radius = radius * projection::unit_disc_radius(mask_deg);
    let points: Vec<Pos2> = (0..=style::MASK_RING_SEGMENTS)
        .map(|i| {
            let angle = i as f32 / style::MASK_RING_SEGMENTS as f32 * std::f32::consts::TAU;
            center + Vec2::new(angle.sin(), -angle.cos()) * ring_radius
        })
        .collect();
    let stroke = Stroke::new(style::GRID_STROKE_WIDTH_PX, ui.visuals().weak_text_color());
    ui.painter().add(Shape::dashed_line(
        &points,
        stroke,
        style::MASK_RING_DASH_PX,
        style::MASK_RING_GAP_PX,
    ));
}

fn paint_marks(
    ui: &egui::Ui,
    center: Pos2,
    radius: f32,
    satellites: &Satellites,
    size: SkyPlotSize,
) {
    let painter = ui.painter();
    let dark_mode = ui.visuals().dark_mode;
    // The panel fill as the mark outline keeps overlapping dots separable.
    let edge = Stroke::new(style::MARK_EDGE_STROKE_WIDTH_PX, ui.visuals().panel_fill);

    let mark = |satellite: &Satellite| {
        let position = projection::mark_position(satellite)?;
        Some((*satellite, center + position * radius))
    };
    // Hollow (tracked-only) marks first so fix satellites paint on top.
    let (fix, tracked): (Vec<_>, Vec<_>) = satellites
        .satellites()
        .filter_map(mark)
        .partition(|(satellite, _)| satellite.in_fix());
    for (satellite, position) in tracked.iter().chain(fix.iter()) {
        let color = gt_ui_theme::constellation_color(satellite.constellation(), dark_mode);
        let mark_radius =
            style::mark_radius(satellite.snr().map(|snr| snr.quality())) * size.mark_scale();
        if satellite.in_fix() {
            painter.circle(*position, mark_radius, color, edge);
        } else {
            painter.circle_stroke(
                *position,
                mark_radius,
                Stroke::new(style::HOLLOW_MARK_STROKE_WIDTH_PX, color),
            );
        }
    }
}

#[cfg(test)]
mod snapshot_tests {
    use rstest::rstest;

    use gt_test_utils::TestHarness;
    use gt_types::satellites::{Constellation, Satellite, Satellites};

    use super::{SkyPlot, SkyPlotSize};

    /// Several constellations, tracked-only satellites, the full
    /// signal-quality spread, and two unplaceable satellites.
    fn mixed_report() -> Satellites {
        let sat = |constellation, prn, elevation: f32, azimuth: f32, snr, in_fix| {
            Satellite::new(
                constellation,
                prn,
                Some(elevation),
                Some(azimuth),
                snr,
                in_fix,
            )
        };
        Satellites::new(
            None,
            None,
            vec![
                sat(Constellation::Gps, 5, 62.0, 45.0, Some(44.0), true),
                sat(Constellation::Gps, 12, 35.0, 110.0, Some(38.0), true),
                sat(Constellation::Gps, 18, 71.0, 200.0, Some(47.0), true),
                sat(Constellation::Gps, 23, 18.0, 305.0, Some(31.0), true),
                sat(Constellation::Gps, 29, 12.0, 155.0, Some(24.0), false),
                sat(Constellation::Gps, 2, 8.0, 250.0, None, false),
                sat(Constellation::Galileo, 3, 55.0, 80.0, Some(42.0), true),
                sat(Constellation::Galileo, 15, 40.0, 340.0, Some(39.0), true),
                sat(Constellation::Galileo, 27, 25.0, 220.0, Some(33.0), true),
                sat(Constellation::Glonass, 9, 48.0, 130.0, Some(40.0), true),
                sat(Constellation::Glonass, 22, 30.0, 20.0, Some(35.0), false),
                sat(Constellation::Beidou, 14, 65.0, 275.0, Some(41.0), true),
                sat(Constellation::Beidou, 31, 20.0, 185.0, Some(28.0), false),
                Satellite::new(Constellation::Qzss, 1, Some(50.0), None, Some(36.0), false),
                Satellite::new(Constellation::Navic, 4, None, None, None, false),
            ],
        )
    }

    /// A report where the receiver tracks satellites but uses none in a fix.
    fn zero_fix_report() -> Satellites {
        let satellites = mixed_report()
            .satellites()
            .map(|s| {
                Satellite::new(
                    s.constellation(),
                    s.prn().value(),
                    s.elevation(),
                    s.azimuth(),
                    s.snr().map(|snr| snr.value()),
                    false,
                )
            })
            .collect();
        Satellites::new(None, None, satellites)
    }

    fn snapshot(name: &str, size: SkyPlotSize, report: &Satellites, dark_mode: bool) {
        let harness_size = match size {
            SkyPlotSize::Compact => egui::vec2(160.0, 190.0),
            SkyPlotSize::Full => egui::vec2(290.0, 330.0),
        };
        let mut harness = TestHarness::builder()
            .size(harness_size)
            .theme(dark_mode)
            .ui(|ui| {
                SkyPlot::new(report, size)
                    .with_elevation_mask_deg(10.0)
                    .ui(ui);
            });
        harness.run();
        harness.snapshot(name);
    }

    #[rstest]
    #[case::full_dark("sky_plot_full_dark", SkyPlotSize::Full, true)]
    #[case::full_light("sky_plot_full_light", SkyPlotSize::Full, false)]
    #[case::compact_dark("sky_plot_compact_dark", SkyPlotSize::Compact, true)]
    #[case::compact_light("sky_plot_compact_light", SkyPlotSize::Compact, false)]
    fn sky_plot_sizes_and_themes(
        #[case] name: &str,
        #[case] size: SkyPlotSize,
        #[case] dark_mode: bool,
    ) {
        snapshot(name, size, &mixed_report(), dark_mode);
    }

    #[test]
    fn sky_plot_zero_fix_dark() {
        snapshot(
            "sky_plot_zero_fix_dark",
            SkyPlotSize::Full,
            &zero_fix_report(),
            true,
        );
    }
}

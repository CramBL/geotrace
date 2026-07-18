use egui::{Pos2, Sense, Stroke, Vec2};

use gt_types::satellites::{Constellation, ConstellationSet, Prn, Satellite, Satellites};

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

/// A subset of satellites to emphasize on the plot, driven by hovering the
/// satellite tables next to it. Matching marks stay at full strength; the
/// rest dim, so the highlighted subset stands out without the others
/// vanishing.
///
/// A predicate over three independent axes rather than an enum of cases: the
/// constellations to match (a [`ConstellationSet`], from one up to all), an
/// optional specific [`Prn`], and whether to require the satellite to be in
/// the fix. The constructors name the four ways the tables drive it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SkyHighlight {
    constellations: ConstellationSet,
    prn: Option<Prn>,
    in_fix_only: bool,
}

impl SkyHighlight {
    /// One satellite - hovering its row in the per-PRN table.
    pub fn satellite(constellation: Constellation, prn: Prn) -> Self {
        Self {
            constellations: ConstellationSet::single(constellation),
            prn: Some(prn),
            in_fix_only: false,
        }
    }

    /// Every satellite of a constellation - hovering its table header.
    pub fn constellation(constellation: Constellation) -> Self {
        Self {
            constellations: ConstellationSet::single(constellation),
            prn: None,
            in_fix_only: false,
        }
    }

    /// Every satellite in the fix - hovering the total fix count.
    pub fn in_fix() -> Self {
        Self {
            constellations: ConstellationSet::all(),
            prn: None,
            in_fix_only: true,
        }
    }

    /// A constellation's in-fix satellites - hovering its fix count.
    pub fn constellation_in_fix(constellation: Constellation) -> Self {
        Self {
            constellations: ConstellationSet::single(constellation),
            prn: None,
            in_fix_only: true,
        }
    }

    /// Whether `satellite` belongs to the highlighted subset.
    pub fn matches(self, satellite: &Satellite) -> bool {
        self.constellations.contains(satellite.constellation())
            && self.prn.is_none_or(|prn| prn == satellite.prn())
            && (!self.in_fix_only || satellite.in_fix())
    }
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
/// elevation cannot be placed and are surfaced beneath the plot instead, as
/// a count line at the compact size and as one row per satellite at the full
/// size.
pub struct SkyPlot<'a> {
    satellites: &'a Satellites,
    size: SkyPlotSize,
    elevation_mask_deg: Option<f32>,
    interactive: bool,
    highlight: Option<SkyHighlight>,
}

impl<'a> SkyPlot<'a> {
    pub fn new(satellites: &'a Satellites, size: SkyPlotSize) -> Self {
        Self {
            satellites,
            size,
            elevation_mask_deg: None,
            interactive: false,
            highlight: None,
        }
    }

    /// Emphasize a subset of satellites, dimming the rest. `None` draws every
    /// mark at full strength.
    pub fn with_highlight(self, highlight: Option<SkyHighlight>) -> Self {
        Self { highlight, ..self }
    }

    /// Draws the elevation mask as a dashed ring. Satellites below the mask
    /// stay visible - the ring is context, not a filter.
    pub fn with_elevation_mask_deg(self, mask_deg: f32) -> Self {
        Self {
            elevation_mask_deg: Some(mask_deg),
            ..self
        }
    }

    /// Enables the per-mark hover tooltip. Only for hosts that are not
    /// themselves hover-transient (the sticky popup window) - inside a
    /// tooltip the pointer can never reach a mark.
    pub fn interactive(self) -> Self {
        Self {
            interactive: true,
            ..self
        }
    }

    pub fn ui(&self, ui: &mut egui::Ui) -> egui::Response {
        let Self {
            satellites,
            size,
            elevation_mask_deg,
            interactive,
            highlight,
        } = *self;

        ui.vertical(|ui| {
            status_line_ui(ui, satellites, size);

            let diameter = size.diameter();
            let (rect, response) = ui.allocate_exact_size(Vec2::splat(diameter), Sense::hover());
            if ui.is_rect_visible(rect) {
                let center = rect.center();
                let radius = diameter / 2.0 - size.rim_margin();
                crate::grid::draw_grid(ui, center, radius, size == SkyPlotSize::Full);
                if let Some(mask_deg) = elevation_mask_deg {
                    // The point plot has no ring hover yet, so never highlighted.
                    crate::grid::draw_mask_ring(ui, center, radius, mask_deg, false);
                }
                let marks = paint_marks(ui, center, radius, satellites, size, highlight);
                if interactive {
                    mark_tooltip(ui, &response, &marks);
                }
            }

            unplaceable_ui(ui, satellites, size);
            response
        })
        .inner
    }
}

/// The instant tooltip for the mark nearest the pointer, within
/// [`style::MARK_HOVER_RADIUS_PX`].
fn mark_tooltip(ui: &egui::Ui, response: &egui::Response, marks: &[(Satellite, Pos2)]) {
    let Some(pointer) = response.hover_pos() else {
        return;
    };
    let Some(satellite) = nearest_mark(marks, pointer) else {
        return;
    };
    egui::Tooltip::always_open(
        ui.ctx().clone(),
        ui.layer_id(),
        response
            .id
            .with((satellite.constellation(), satellite.prn())),
        egui::PopupAnchor::Pointer,
    )
    .show(|ui| satellite_tooltip_ui(ui, satellite));
}

/// The mark nearest to `pointer`, within [`style::MARK_HOVER_RADIUS_PX`].
fn nearest_mark(marks: &[(Satellite, Pos2)], pointer: Pos2) -> Option<&Satellite> {
    let candidates = marks.iter().map(|(satellite, pos)| (satellite, *pos));
    crate::plot_common::nearest_within(candidates, pointer, style::MARK_HOVER_RADIUS_PX)
}

fn satellite_tooltip_ui(ui: &mut egui::Ui, satellite: &Satellite) {
    let label =
        crate::plot_common::satellite_designator(satellite.constellation(), satellite.prn());
    ui.label(egui::RichText::new(label).strong());
    let degree = |value: Option<f32>| {
        value.map_or_else(
            || gt_ui_theme::EM_DASH.to_owned(),
            |v| format!("{v:.0}{}", gt_ui_theme::DEGREE_SIGN),
        )
    };
    ui.label(format!("Elevation {}", degree(satellite.elevation())));
    ui.label(format!("Azimuth {}", degree(satellite.azimuth())));
    let snr = satellite.snr().map_or_else(
        || gt_ui_theme::EM_DASH.to_owned(),
        |snr| format!("{:.0} dB-Hz", snr.value()),
    );
    ui.label(format!("SNR {snr}"));
    ui.label(if satellite.in_fix() {
        "In fix"
    } else {
        "Tracked, not in fix"
    });
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

/// Satellites without azimuth or elevation, under the plot: the compact size
/// shows a count line, the full size one row per satellite. Nothing when all
/// satellites are placeable.
fn unplaceable_ui(ui: &mut egui::Ui, satellites: &Satellites, size: SkyPlotSize) {
    let unplaceable: Vec<&Satellite> = satellites
        .satellites()
        .filter(|satellite| projection::mark_position(satellite).is_none())
        .collect();
    match (size, unplaceable.as_slice()) {
        (_, []) => {}
        (SkyPlotSize::Compact, [_]) => {
            ui.label(
                egui::RichText::new("1 satellite without sky position")
                    .weak()
                    .small(),
            );
        }
        (SkyPlotSize::Compact, many) => {
            ui.label(
                egui::RichText::new(format!("{} satellites without sky position", many.len()))
                    .weak()
                    .small(),
            );
        }
        (SkyPlotSize::Full, many) => {
            let dark_mode = ui.visuals().dark_mode;
            for satellite in many {
                ui.horizontal(|ui| {
                    let label = crate::plot_common::satellite_designator(
                        satellite.constellation(),
                        satellite.prn(),
                    );
                    ui.label(
                        egui::RichText::new(label)
                            .color(gt_ui_theme::constellation_color(
                                satellite.constellation(),
                                dark_mode,
                            ))
                            .small(),
                    );
                    ui.label(egui::RichText::new("no sky position").weak().small());
                });
            }
        }
    }
}

/// Paints the satellite marks and returns them with their screen positions,
/// in paint order, for hit-testing.
fn paint_marks(
    ui: &egui::Ui,
    center: Pos2,
    radius: f32,
    satellites: &Satellites,
    size: SkyPlotSize,
    highlight: Option<SkyHighlight>,
) -> Vec<(Satellite, Pos2)> {
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
    let marks: Vec<(Satellite, Pos2)> = tracked.into_iter().chain(fix).collect();
    for (satellite, position) in &marks {
        // With a highlight active, marks outside the subset dim so the
        // highlighted ones stand out without the rest disappearing.
        let dimmed = highlight.is_some_and(|h| !h.matches(satellite));
        let dim = |color: egui::Color32| {
            if dimmed {
                color.gamma_multiply(style::DIMMED_MARK_ALPHA)
            } else {
                color
            }
        };
        let color = dim(gt_ui_theme::constellation_color(
            satellite.constellation(),
            dark_mode,
        ));
        let mark_radius =
            style::mark_radius(satellite.snr().map(|snr| snr.quality())) * size.mark_scale();
        if satellite.in_fix() {
            let edge = Stroke::new(edge.width, dim(edge.color));
            painter.circle(*position, mark_radius, color, edge);
        } else {
            painter.circle_stroke(
                *position,
                mark_radius,
                Stroke::new(style::HOLLOW_MARK_STROKE_WIDTH_PX, color),
            );
        }
    }
    marks
}

#[cfg(test)]
mod snapshot_tests {
    use rstest::rstest;

    use gt_test_utils::TestHarness;
    use gt_types::satellites::{Constellation, Satellite, Satellites};

    use super::{SkyHighlight, SkyPlot, SkyPlotSize};

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

    #[rstest]
    #[case::constellation(
        "sky_plot_highlight_constellation",
        SkyHighlight::constellation(Constellation::Gps)
    )]
    #[case::in_fix("sky_plot_highlight_in_fix", SkyHighlight::in_fix())]
    fn sky_plot_highlight_dims_the_rest(#[case] name: &str, #[case] highlight: SkyHighlight) {
        let report = mixed_report();
        let mut harness = TestHarness::builder()
            .size(egui::vec2(290.0, 330.0))
            .theme(true)
            .ui(move |ui| {
                SkyPlot::new(&report, SkyPlotSize::Full)
                    .with_elevation_mask_deg(10.0)
                    .with_highlight(Some(highlight))
                    .ui(ui);
            });
        harness.run();
        harness.snapshot(name);
    }

    #[test]
    fn highlight_matches_the_right_satellites() {
        let gps_fix = Satellite::new(Constellation::Gps, 5, Some(45.0), Some(90.0), None, true);
        let gps_idle = Satellite::new(Constellation::Gps, 9, Some(45.0), Some(90.0), None, false);
        let gal_fix = Satellite::new(
            Constellation::Galileo,
            3,
            Some(45.0),
            Some(90.0),
            None,
            true,
        );

        let one = SkyHighlight::satellite(Constellation::Gps, gps_fix.prn());
        assert!(one.matches(&gps_fix));
        assert!(!one.matches(&gps_idle));
        assert!(!one.matches(&gal_fix));

        let constellation = SkyHighlight::constellation(Constellation::Gps);
        assert!(constellation.matches(&gps_fix));
        assert!(constellation.matches(&gps_idle));
        assert!(!constellation.matches(&gal_fix));

        let in_fix = SkyHighlight::in_fix();
        assert!(in_fix.matches(&gps_fix));
        assert!(!in_fix.matches(&gps_idle));
        assert!(in_fix.matches(&gal_fix));

        let const_fix = SkyHighlight::constellation_in_fix(Constellation::Gps);
        assert!(const_fix.matches(&gps_fix));
        assert!(!const_fix.matches(&gps_idle));
        assert!(!const_fix.matches(&gal_fix));
    }

    #[test]
    fn sky_mark_tooltip() {
        let full = Satellite::new(
            Constellation::Gps,
            5,
            Some(62.0),
            Some(45.0),
            Some(44.0),
            true,
        );
        let snr_less = Satellite::new(
            Constellation::Glonass,
            22,
            Some(30.0),
            Some(20.0),
            None,
            false,
        );
        let mut harness = TestHarness::builder()
            .size(egui::vec2(220.0, 220.0))
            .theme(true)
            .ui(move |ui| {
                super::satellite_tooltip_ui(ui, &full);
                ui.separator();
                super::satellite_tooltip_ui(ui, &snr_less);
            });
        harness.run();
        harness.snapshot("sky_mark_tooltip");
    }

    #[rstest]
    #[case::hits_the_nearest(egui::pos2(101.0, 100.0), Some(5))]
    #[case::prefers_the_closer_of_two(egui::pos2(108.0, 100.0), Some(12))]
    #[case::beyond_hover_radius(egui::pos2(150.0, 150.0), None)]
    fn nearest_mark_respects_the_hover_radius(
        #[case] pointer: egui::Pos2,
        #[case] expected_prn: Option<u32>,
    ) {
        let sat = |prn| Satellite::new(Constellation::Gps, prn, Some(45.0), Some(90.0), None, true);
        let marks = vec![
            (sat(5), egui::pos2(100.0, 100.0)),
            (sat(12), egui::pos2(112.0, 100.0)),
        ];
        let nearest = super::nearest_mark(&marks, pointer).map(|s| s.prn().value());
        assert_eq!(nearest, expected_prn);
    }
}

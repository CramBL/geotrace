use egui::{Color32, Label, Pos2, RichText, Sense, Stroke, Vec2};

use gt_types::satellites::{
    Constellation, ConstellationSet, Prn, Satellite, Satellites, SignalQuality, Snr,
};
use gt_types::{SatelliteMembership, SkyCounts, SkyEvidence, SkySatellite};
use gt_ui_theme::{SatelliteCountBadge, SatelliteMembershipMark};

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
/// satellite tables next to it. Matching marks stay at full strength, the rest
/// dim.
///
/// A predicate over three independent axes: the constellations to match (a
/// [`ConstellationSet`]), an optional specific [`Prn`], and whether to require
/// the satellite to be in the fix. The constructors name the four ways the
/// tables drive it.
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

    pub fn matches(self, satellite: &Satellite) -> bool {
        self.matches_sky(&SkySatellite::from_legacy(satellite))
    }

    pub fn matches_sky(self, satellite: &SkySatellite) -> bool {
        self.constellations.contains(satellite.constellation())
            && self.prn.is_none_or(|prn| prn == satellite.prn())
            && (!self.in_fix_only || satellite.in_fix())
    }
}

impl SkyPlotSize {
    /// How wide this plot draws. Public so callers can budget layout space
    /// around it without re-deriving the size from [`style`]'s constants.
    pub const fn diameter(self) -> f32 {
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

pub struct SkyPlot {
    satellites: Vec<SkySatellite>,
    counts: SkyCounts,
    size: SkyPlotSize,
    elevation_mask_deg: Option<f32>,
    interactive: bool,
    highlight: Option<SkyHighlight>,
    show_counts: bool,
    show_identity_rows: bool,
}

impl SkyPlot {
    pub fn new(satellites: &Satellites, size: SkyPlotSize) -> Self {
        Self {
            satellites: satellites
                .satellites()
                .map(SkySatellite::from_legacy)
                .collect(),
            counts: SkyCounts {
                used: Some(satellites.fix_count()),
                seen: Some(satellites.satellite_count()),
            },
            size,
            elevation_mask_deg: None,
            interactive: false,
            highlight: None,
            show_counts: true,
            show_identity_rows: true,
        }
    }

    pub fn from_evidence(evidence: SkyEvidence<'_>, size: SkyPlotSize) -> Self {
        Self {
            satellites: evidence.satellites(),
            counts: evidence.counts(),
            size,
            elevation_mask_deg: None,
            interactive: false,
            highlight: None,
            show_counts: true,
            show_identity_rows: true,
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

    pub fn counts_in_host(self) -> Self {
        Self {
            show_counts: false,
            ..self
        }
    }

    pub fn identities_in_host(self) -> Self {
        Self {
            show_identity_rows: false,
            ..self
        }
    }

    pub fn ui(&self, ui: &mut egui::Ui) -> egui::Response {
        ui.vertical(|ui| {
            if self.show_counts {
                SatelliteCountBadge(self.counts).ui(ui);
            }
            let has_positions = self
                .satellites
                .iter()
                .any(|satellite| satellite.sky_position().is_some());
            if has_positions {
                let diameter = self.size.diameter();
                let (rect, response) =
                    ui.allocate_exact_size(Vec2::splat(diameter), Sense::hover());
                if ui.is_rect_visible(rect) {
                    let center = rect.center();
                    let radius = diameter / 2.0 - self.size.rim_margin();
                    crate::grid::draw_grid(ui, center, radius, self.size == SkyPlotSize::Full);
                    if let Some(mask_deg) = self.elevation_mask_deg {
                        crate::grid::draw_mask_ring(ui, center, radius, mask_deg, false);
                    }
                    let marks = paint_marks(
                        ui,
                        center,
                        radius,
                        &self.satellites,
                        self.size,
                        self.highlight,
                    );
                    if self.interactive {
                        mark_tooltip(ui, &response, &marks);
                    }
                }
            }
            let unplaceable: Vec<_> = self
                .satellites
                .iter()
                .filter(|satellite| satellite.sky_position().is_none())
                .collect();
            if !unplaceable.is_empty() {
                ui.label(RichText::new("No sky positions").weak().small());
                if self.show_identity_rows {
                    let preview_limit = match self.size {
                        SkyPlotSize::Compact => COMPACT_IDENTITY_PREVIEW_LIMIT,
                        SkyPlotSize::Full => unplaceable.len(),
                    };
                    for satellite in unplaceable.iter().take(preview_limit) {
                        ui.horizontal(|ui| {
                            let dimmed = self
                                .highlight
                                .is_some_and(|highlight| !highlight.matches_sky(satellite));
                            let color = gt_ui_theme::constellation_color(
                                satellite.constellation(),
                                ui.visuals().dark_mode,
                            );
                            let color = if dimmed {
                                color.gamma_multiply(style::DIMMED_MARK_ALPHA)
                            } else {
                                color
                            };
                            SatelliteMembershipMark {
                                membership: satellite.membership,
                                radius: IDENTITY_MARK_RADIUS_PX,
                                minimum_unknown_radius:
                                    gt_ui_theme::SKY_UNKNOWN_MEMBERSHIP_MIN_RADIUS_PX,
                                color,
                                edge: Stroke::new(style::HOLLOW_MARK_STROKE_WIDTH_PX, color),
                                hollow_fill: Color32::TRANSPARENT,
                            }
                            .ui(ui);
                            let label = crate::plot_common::satellite_designator(
                                satellite.constellation(),
                                satellite.prn(),
                            );
                            ui.add(
                                Label::new(RichText::new(label).color(color).small())
                                    .selectable(true),
                            );
                        });
                    }
                    if unplaceable.len() > preview_limit {
                        ui.label(
                            RichText::new(format!(
                                "{} more {} Pin to inspect",
                                unplaceable.len() - preview_limit,
                                gt_ui_theme::MIDDLE_DOT,
                            ))
                            .weak()
                            .small(),
                        );
                    }
                }
            }
        })
        .response
    }
}

/// The instant tooltip for the mark nearest the pointer, within
/// [`style::MARK_HOVER_RADIUS_PX`].
fn mark_tooltip(ui: &egui::Ui, response: &egui::Response, marks: &[(SkySatellite, Pos2)]) {
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
    .show(|ui| crate::plot_common::satellite_tooltip(ui, satellite, None));
}

/// The mark nearest to `pointer`, within [`style::MARK_HOVER_RADIUS_PX`].
fn nearest_mark(marks: &[(SkySatellite, Pos2)], pointer: Pos2) -> Option<&SkySatellite> {
    let candidates = marks.iter().map(|(satellite, pos)| (satellite, *pos));
    crate::plot_common::nearest_within(candidates, pointer, style::MARK_HOVER_RADIUS_PX)
}

const IDENTITY_MARK_RADIUS_PX: f32 = 3.0;
const COMPACT_IDENTITY_PREVIEW_LIMIT: usize = 3;

/// Paints the satellite marks and returns them with their screen positions,
/// in paint order, for hit-testing.
fn paint_marks(
    ui: &egui::Ui,
    center: Pos2,
    radius: f32,
    satellites: &[SkySatellite],
    size: SkyPlotSize,
    highlight: Option<SkyHighlight>,
) -> Vec<(SkySatellite, Pos2)> {
    let painter = ui.painter();
    let dark_mode = ui.visuals().dark_mode;
    // The panel fill as the mark outline keeps overlapping dots separable.
    let edge = Stroke::new(style::MARK_EDGE_STROKE_WIDTH_PX, ui.visuals().panel_fill);

    let mark = |satellite: &SkySatellite| {
        let position = satellite
            .sky_position()
            .map(|(azimuth, elevation)| projection::unit_disc_position(azimuth, elevation))?;
        Some((*satellite, center + position * radius))
    };
    // Hollow (tracked-only) marks first so fix satellites paint on top.
    let (fix, tracked): (Vec<_>, Vec<_>) = satellites
        .iter()
        .filter_map(mark)
        .partition(|(satellite, _)| satellite.in_fix());
    let marks: Vec<(SkySatellite, Pos2)> = tracked.into_iter().chain(fix).collect();
    for (satellite, position) in &marks {
        let dimmed = highlight.is_some_and(|h| !h.matches_sky(satellite));
        let dim = |color: egui::Color32| {
            if dimmed {
                color.gamma_multiply(style::DIMMED_MARK_ALPHA)
            } else {
                color
            }
        };
        // A satellite whose receiver reported the no-data SNR value stands
        // out: it takes that class's colour, not its constellation's.
        let color = dim(
            if satellite.membership != SatelliteMembership::Unknown
                && satellite.snr().is_some_and(Snr::is_no_data_sentinel)
            {
                gt_ui_theme::snr_color(SignalQuality::NoDataSentinel, dark_mode)
            } else {
                gt_ui_theme::constellation_color(satellite.constellation(), dark_mode)
            },
        );
        let mark_radius =
            style::mark_radius(satellite.snr().map(|snr| snr.quality())) * size.mark_scale();
        SatelliteMembershipMark {
            membership: satellite.membership,
            radius: mark_radius,
            minimum_unknown_radius: gt_ui_theme::SKY_UNKNOWN_MEMBERSHIP_MIN_RADIUS_PX,
            color,
            edge: if satellite.in_fix() {
                Stroke::new(edge.width, dim(edge.color))
            } else {
                Stroke::new(style::HOLLOW_MARK_STROKE_WIDTH_PX, color)
            },
            hollow_fill: Color32::TRANSPARENT,
        }
        .paint(painter, *position);
    }
    marks
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use gt_test_utils::TestHarness;
    use gt_types::SkySatellite;
    use gt_types::satellites::{Constellation, NO_DATA_SENTINEL_DB_HZ, Satellite, Satellites};

    use super::{SkyHighlight, SkyPlot, SkyPlotSize};
    use crate::test_util::{self, Azimuth, Elevation};

    /// One satellite of the report below, at a sky position both angles of
    /// which the receiver reported.
    fn sat(
        constellation: Constellation,
        prn: u32,
        azimuth_deg: f32,
        elevation_deg: f32,
        snr_db: Option<f32>,
        in_fix: bool,
    ) -> Satellite {
        test_util::satellite(
            constellation,
            prn,
            Some(Azimuth(azimuth_deg)),
            Some(Elevation(elevation_deg)),
            snr_db,
            in_fix,
        )
    }

    /// Several constellations, tracked-only satellites, the full
    /// signal-quality spread, a satellite whose receiver reported the no-data
    /// SNR value, and two unplaceable satellites.
    fn mixed_report() -> Satellites {
        Satellites::new(
            None,
            None,
            vec![
                sat(Constellation::Gps, 5, 45.0, 62.0, Some(44.0), true),
                sat(Constellation::Gps, 12, 110.0, 35.0, Some(38.0), true),
                sat(Constellation::Gps, 18, 200.0, 71.0, Some(47.0), true),
                sat(Constellation::Gps, 23, 305.0, 18.0, Some(31.0), true),
                sat(Constellation::Gps, 29, 155.0, 12.0, Some(24.0), false),
                sat(Constellation::Gps, 2, 250.0, 8.0, None, false),
                sat(Constellation::Galileo, 3, 80.0, 55.0, Some(42.0), true),
                sat(Constellation::Galileo, 15, 340.0, 40.0, Some(39.0), true),
                sat(Constellation::Galileo, 27, 220.0, 25.0, Some(33.0), true),
                sat(Constellation::Glonass, 9, 130.0, 48.0, Some(40.0), true),
                sat(Constellation::Glonass, 22, 20.0, 30.0, Some(35.0), false),
                sat(
                    Constellation::Glonass,
                    14,
                    240.0,
                    45.0,
                    Some(NO_DATA_SENTINEL_DB_HZ),
                    true,
                ),
                sat(Constellation::Beidou, 14, 275.0, 65.0, Some(41.0), true),
                sat(Constellation::Beidou, 31, 185.0, 20.0, Some(28.0), false),
                test_util::satellite(
                    Constellation::Qzss,
                    1,
                    None,
                    Some(Elevation(50.0)),
                    Some(36.0),
                    false,
                ),
                test_util::satellite(Constellation::Navic, 4, None, None, None, false),
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
        harness.snapshot_with_color_tolerance(name);
    }

    #[test]
    fn highlight_matches_the_right_satellites() {
        let gps_fix = sat(Constellation::Gps, 5, 90.0, 45.0, None, true);
        let gps_idle = sat(Constellation::Gps, 9, 90.0, 45.0, None, false);
        let gal_fix = sat(Constellation::Galileo, 3, 90.0, 45.0, None, true);

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

    /// Snapshot: the three states of the tooltip, an in-fix satellite with a
    /// full set of measurements, a tracked one missing its SNR, and one whose
    /// receiver reported the no-data SNR value.
    ///
    /// Rendering two in one `Ui` also guards the tooltip grid's id. The grid
    /// keeps its column widths under that id, so a shared one leaves two
    /// tooltips resizing to each other's measurements and repainting forever:
    /// the test would exceed its step budget.
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
        let no_data = Satellite::new(
            Constellation::Galileo,
            3,
            Some(48.0),
            Some(300.0),
            Some(NO_DATA_SENTINEL_DB_HZ),
            true,
        );
        let mut harness = TestHarness::builder()
            .size(egui::vec2(260.0, 320.0))
            .theme(true)
            .ui(move |ui| {
                crate::plot_common::satellite_tooltip(ui, &SkySatellite::from_legacy(&full), None);
                ui.separator();
                crate::plot_common::satellite_tooltip(
                    ui,
                    &SkySatellite::from_legacy(&snr_less),
                    None,
                );
                ui.separator();
                crate::plot_common::satellite_tooltip(
                    ui,
                    &SkySatellite::from_legacy(&no_data),
                    None,
                );
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
        let marks = vec![
            (
                sat(Constellation::Gps, 5, 90.0, 45.0, None, true),
                egui::pos2(100.0, 100.0),
            ),
            (
                sat(Constellation::Gps, 12, 90.0, 45.0, None, true),
                egui::pos2(112.0, 100.0),
            ),
        ];
        let marks: Vec<_> = marks
            .into_iter()
            .map(|(satellite, position)| (SkySatellite::from_legacy(&satellite), position))
            .collect();
        let nearest = super::nearest_mark(&marks, pointer).map(|s| s.prn().value());
        assert_eq!(nearest, expected_prn);
    }
}

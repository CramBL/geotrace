//! The solar flare markers: one vertical line at each archived flare's peak,
//! coloured by the flare's class.
//!
//! The lines are drawn from the archive across the plot's whole visible span,
//! like the context metric lines, so a flare shows even where no recording
//! covers it.

use chrono::{DateTime, Utc};
use egui::epaint::{Shape, Stroke};
use egui::{Color32, Ui};
use egui_plot::{PlotBounds, PlotGeometry, PlotItem, PlotItemBase, PlotPoint, PlotTransform};
use gt_flare::SolarFlare;
use gt_flare::text::{self, FormattedFlareTimes};

use super::lines::{NearestHoverLabel, PlotHoverLabel};

/// Stroke width of a marker line, above the data lines' default so a flare
/// stays findable across a crowded plot.
const MARKER_WIDTH: f32 = 1.5;

/// Pixel distance from a marker within which the pointer is hovering it.
const HOVER_RADIUS_PX: f32 = 5.0;

/// How the marker hover writes the three times. The catalog publishes them to
/// the minute.
const FLARE_TIME_FORMAT: &str = "%Y-%m-%dT%H:%M";

/// The flare markers of one span, as a plot item that contributes nothing to
/// the plot's auto-bounds.
///
/// The archive reaches beyond the loaded recordings, so letting a marker feed
/// the bounds would widen the view to whatever was downloaded.
struct FlareMarkers {
    base: PlotItemBase,
    /// Peak time in Unix seconds and the class colour, in the order they were
    /// offered.
    markers: Vec<(f64, Color32)>,
    /// What the plot's own legend entry is drawn in, since the markers
    /// themselves have a colour each.
    legend_color: Color32,
}

impl PlotItem for FlareMarkers {
    fn shapes(&self, _ui: &Ui, transform: &PlotTransform, shapes: &mut Vec<Shape>) {
        let frame = *transform.frame();
        for &(peak_secs, color) in &self.markers {
            let x = transform
                .position_from_point(&PlotPoint::new(peak_secs, 0.0))
                .x;
            shapes.push(Shape::line_segment(
                [egui::pos2(x, frame.top()), egui::pos2(x, frame.bottom())],
                Stroke::new(MARKER_WIDTH, color),
            ));
        }
    }

    fn initialize(&mut self, _x_range: std::ops::RangeInclusive<f64>) {}

    fn color(&self) -> Color32 {
        self.legend_color
    }

    /// No geometry: hovering is handled by [`SolarFlareHover`], which reports
    /// the whole event rather than a point on a line.
    fn geometry(&self) -> PlotGeometry<'_> {
        PlotGeometry::None
    }

    fn bounds(&self) -> PlotBounds {
        PlotBounds::NOTHING
    }

    fn base(&self) -> &PlotItemBase {
        &self.base
    }

    fn base_mut(&mut self) -> &mut PlotItemBase {
        &mut self.base
    }
}

/// The visible span the markers are clipped to, and the theme they are
/// coloured for.
#[derive(Clone, Copy)]
pub(super) struct FlareViewport {
    pub(super) x_min: f64,
    pub(super) x_max: f64,
    pub(super) dark_mode: bool,
}

impl FlareViewport {
    fn holds(self, peak_secs: f64) -> bool {
        (self.x_min..=self.x_max).contains(&peak_secs)
    }
}

/// Draw a marker for every flare inside the visible span and, when the
/// pointer is within [`HOVER_RADIUS_PX`] of one, record the nearest in
/// `nearest` so the caller can show its tooltip.
pub(super) fn add_flare_markers(
    plot_ui: &mut egui_plot::PlotUi<'_>,
    flares: &[SolarFlare],
    viewport: FlareViewport,
    pointer: Option<egui::Pos2>,
    nearest: &mut NearestHoverLabel,
) {
    let visible: Vec<&SolarFlare> = flares
        .iter()
        .filter(|flare| viewport.holds(peak_secs(flare)))
        .collect();
    if visible.is_empty() {
        return;
    }

    plot_ui.add(FlareMarkers {
        base: PlotItemBase::new(text::LAYER_LABEL.to_owned()),
        markers: visible
            .iter()
            .map(|flare| {
                (
                    peak_secs(flare),
                    gt_ui_theme::solar_flare_color(
                        flare.classification.peak_flux_watts_per_square_meter(),
                    )
                    .resolve(viewport.dark_mode),
                )
            })
            .collect(),
        legend_color: gt_ui_theme::FLARE_M_CLASS.resolve(viewport.dark_mode),
    });

    let Some(pointer) = pointer else {
        return;
    };
    for flare in visible {
        let x = plot_ui
            .screen_from_plot(PlotPoint::new(peak_secs(flare), 0.0))
            .x;
        let distance = (x - pointer.x).abs();
        if distance <= HOVER_RADIUS_PX {
            nearest.offer(distance, || {
                PlotHoverLabel::SolarFlare(SolarFlareHover::of_archived_flare(flare))
            });
        }
    }
}

/// The plot x a flare is marked at.
fn peak_secs(flare: &SolarFlare) -> f64 {
    flare.peak.timestamp() as f64
}

/// Pre-formatted tooltip contents for one flare.
pub(super) struct SolarFlareHover {
    lines: Vec<String>,
}

impl SolarFlareHover {
    fn of_archived_flare(flare: &SolarFlare) -> Self {
        let formatted = |time: DateTime<Utc>| time.format(FLARE_TIME_FORMAT).to_string();
        let end = flare.end.map(formatted);
        Self {
            lines: text::flare_summary(
                flare,
                FormattedFlareTimes {
                    begin: &formatted(flare.begin),
                    peak: &formatted(flare.peak),
                    end: end.as_deref(),
                },
            ),
        }
    }

    pub(super) fn show(&self, ui: &mut egui::Ui) {
        let mut lines = self.lines.iter();
        if let Some(headline) = lines.next() {
            ui.strong(headline);
        }
        for line in lines {
            ui.label(line);
        }
        ui.separator();
        ui.label(text::SOURCE_CAVEAT);
    }
}

#[cfg(test)]
mod tests {
    use chrono::NaiveDate;
    use gt_flare::FlareClass;
    use rstest::rstest;

    use super::*;

    /// One flare of the catalog, with the times the storm's X2.2 had.
    pub(super) fn flare(peak: &str, class_type: &str) -> SolarFlare {
        let peak = gt_flare::wire::parse_flare_time(peak).expect("a catalog time");
        SolarFlare {
            id: format!("{peak}-FLR-001"),
            begin: peak - chrono::TimeDelta::minutes(28),
            peak,
            end: Some(peak + chrono::TimeDelta::minutes(23)),
            classification: class_type.parse().expect("a published class"),
            source_location: Some("S20W25".to_owned()),
            active_region: Some(13664),
        }
    }

    fn midnight(day: NaiveDate) -> f64 {
        day.and_hms_opt(0, 0, 0)
            .map_or(0.0, |naive| naive.and_utc().timestamp() as f64)
    }

    /// A span of one UTC day, as the plot shows it.
    fn viewport(day: (i32, u32, u32)) -> FlareViewport {
        let day = NaiveDate::from_ymd_opt(day.0, day.1, day.2).unwrap_or_default();
        FlareViewport {
            x_min: midnight(day),
            x_max: midnight(day) + 24.0 * 60.0 * 60.0,
            dark_mode: true,
        }
    }

    /// The colour steps where the classes do: the theme's breakpoints and the
    /// classification's own floors are two copies of the same physics.
    #[rstest]
    #[case::c_class(FlareClass::C, gt_ui_theme::FLARE_C_CLASS_FLUX)]
    #[case::m_class(FlareClass::M, gt_ui_theme::FLARE_M_CLASS_FLUX)]
    #[case::x_class(FlareClass::X, gt_ui_theme::FLARE_X_CLASS_FLUX)]
    fn the_colour_breakpoints_are_the_class_floors(
        #[case] class: FlareClass,
        #[case] breakpoint: f64,
    ) {
        assert_eq!(
            class.lowest_flux_watts_per_square_meter().to_bits(),
            breakpoint.to_bits(),
            "{class} begins at another flux than its marker colour steps at"
        );
    }

    #[test]
    fn a_flare_outside_the_visible_span_is_not_marked() {
        let viewport = viewport((2024, 5, 9));
        assert!(viewport.holds(peak_secs(&flare("2024-05-09T09:13Z", "X2.2"))));
        assert!(!viewport.holds(peak_secs(&flare("2024-05-11T01:23Z", "X5.8"))));
    }

    /// The hover leads with the classification and closes with the standing
    /// caveat's subject.
    #[test]
    fn the_hover_reports_the_whole_event() {
        let hover = SolarFlareHover::of_archived_flare(&flare("2024-05-09T09:13Z", "X2.2"));
        assert_eq!(
            hover.lines,
            [
                "X2.2 solar flare",
                "R3 strong radio blackout",
                "Peaked at 2024-05-09T09:13 (UTC)",
                "Began 2024-05-09T08:45, ended 2024-05-09T09:36",
                "Active region 13664 at S20W25",
            ]
        );
    }
}

#[cfg(test)]
mod snapshot_tests {
    use chrono::NaiveDate;
    use egui_plot::{Line, PlotPoints};
    use gt_test_utils::TestHarness;
    use rstest::rstest;

    use super::tests::flare;
    use super::*;

    /// The May 2024 storm day, as the archive holds it: an X-class flare
    /// among M-class ones, and a C-class flare below the blackout scale.
    fn storm_day() -> Vec<SolarFlare> {
        [
            ("2024-05-09T01:15Z", "M1.8"),
            ("2024-05-09T03:32Z", "C4.5"),
            ("2024-05-09T09:13Z", "X2.2"),
            ("2024-05-09T17:44Z", "M9.0"),
            ("2024-05-09T23:08Z", "M1.2"),
        ]
        .into_iter()
        .map(|(peak, class_type)| flare(peak, class_type))
        .collect()
    }

    fn day_bounds() -> (f64, f64) {
        let day = NaiveDate::from_ymd_opt(2024, 5, 9).unwrap_or_default();
        let midnight = day
            .and_hms_opt(0, 0, 0)
            .map_or(0.0, |naive| naive.and_utc().timestamp() as f64);
        (midnight, midnight + 24.0 * 60.0 * 60.0)
    }

    /// The markers over a metric line, which is what the plot draws them
    /// against.
    #[rstest]
    #[case::dark("solar_flare_markers_dark", true)]
    #[case::light("solar_flare_markers_light", false)]
    fn solar_flare_markers(#[case] name: &str, #[case] dark_mode: bool) {
        let flares = storm_day();
        let (x_min, x_max) = day_bounds();
        let mut harness = TestHarness::builder()
            .size(egui::vec2(420.0, 220.0))
            .theme(dark_mode)
            .ui(|ui| {
                egui_plot::Plot::new("flare_markers")
                    .show_grid(false)
                    .show(ui, |plot_ui| {
                        plot_ui
                            .set_plot_bounds(PlotBounds::from_min_max([x_min, 0.0], [x_max, 10.0]));
                        plot_ui.line(Line::new(
                            "Metric",
                            PlotPoints::new(vec![[x_min, 2.0], [x_max, 6.0]]),
                        ));
                        add_flare_markers(
                            plot_ui,
                            &flares,
                            FlareViewport {
                                x_min,
                                x_max,
                                dark_mode,
                            },
                            None,
                            &mut NearestHoverLabel::default(),
                        );
                    });
            });
        harness.run();
        harness.snapshot(name);
    }
}

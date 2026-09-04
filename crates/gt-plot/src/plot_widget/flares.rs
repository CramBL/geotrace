//! The solar flare markers: one vertical line at each archived flare's peak,
//! coloured by the flare's class, and the band marking how long a flare
//! lasted.
//!
//! The lines are drawn from the archive across the plot's whole visible span,
//! like the context metric lines, so a flare shows even where no recording
//! covers it.

use egui::Color32;
use egui::epaint::{Shape, Stroke};
use egui_plot::{PlotPoint, PlotTransform, Span};
use gt_flare::text;
use gt_flare::{MarkedFlare, SolarFlare};

use super::lines::{self, NearestHoverLabel, PlotHoverLabel};
use super::overlay::{OverlayItem, OverlayPainter};

/// Stroke width of a marker line, above the data lines' default so a flare
/// stays findable across a crowded plot.
const MARKER_WIDTH: f32 = 1.5;

/// Pixel distance from a marker within which the pointer is hovering it.
const HOVER_RADIUS_PX: f32 = 5.0;

/// The flare markers of one span.
struct FlareMarkers {
    /// Peak time in Unix seconds and the class colour, in the order they were
    /// offered.
    markers: Vec<(f64, Color32)>,
    /// What the plot's own legend entry is drawn in, since the markers
    /// themselves have a colour each.
    legend_color: Color32,
}

impl OverlayPainter for FlareMarkers {
    fn legend_color(&self) -> Color32 {
        self.legend_color
    }

    fn paint(&self, transform: &PlotTransform, shapes: &mut Vec<Shape>) {
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
}

/// The stretch of plot x one flare's band covers.
#[derive(Debug, Clone, Copy, PartialEq)]
struct FlareSpan {
    start_secs: f64,
    end_secs: f64,
}

impl FlareSpan {
    fn of_flare(flare: &SolarFlare) -> Self {
        Self {
            start_secs: flare.begin.timestamp() as f64,
            end_secs: flare.end_or_peak().timestamp() as f64,
        }
    }
}

/// Whose span is shaded this frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum FlareSpanMarking {
    /// Every flare reaching into the visible span, whether or not its peak
    /// does. Chosen while the flare chip is hovered, and while the setting
    /// behind that chip's context menu is on.
    EveryFlareInView,
    /// The one flare whose peak marker the pointer rests on.
    OnlyTheHoveredFlare,
}

/// The visible span the markers are clipped to, whose spans are shaded, and
/// the theme they are coloured for.
#[derive(Clone, Copy)]
pub(super) struct FlareViewport {
    pub(super) x_min: f64,
    pub(super) x_max: f64,
    pub(super) span_marking: FlareSpanMarking,
    pub(super) dark_mode: bool,
}

impl FlareViewport {
    fn holds(self, peak_secs: f64) -> bool {
        (self.x_min..=self.x_max).contains(&peak_secs)
    }

    fn reaches_into_view(self, span: FlareSpan) -> bool {
        span.start_secs <= self.x_max && self.x_min <= span.end_secs
    }

    /// The flares to shade, in the order the archive holds them.
    fn flares_with_shaded_span<'f>(
        self,
        flares: &'f [MarkedFlare],
        hovered: Option<&'f MarkedFlare>,
    ) -> Vec<&'f MarkedFlare> {
        match self.span_marking {
            FlareSpanMarking::EveryFlareInView => flares
                .iter()
                .filter(|marked| self.reaches_into_view(FlareSpan::of_flare(&marked.flare)))
                .collect(),
            FlareSpanMarking::OnlyTheHoveredFlare => hovered.into_iter().collect(),
        }
    }
}

/// Draw a marker for every flare inside the visible span, shade the spans
/// [`FlareViewport::span_marking`] selects, and, when the pointer is within
/// [`HOVER_RADIUS_PX`] of a marker, record the nearest in `nearest` so the
/// caller can show its tooltip.
pub(super) fn add_flare_markers(
    plot_ui: &mut egui_plot::PlotUi<'_>,
    flares: &[MarkedFlare],
    viewport: FlareViewport,
    pointer: Option<egui::Pos2>,
    nearest: &mut NearestHoverLabel,
) {
    let visible: Vec<&MarkedFlare> = flares
        .iter()
        .filter(|flare| viewport.holds(peak_secs(flare)))
        .collect();
    let hovered = pointer.and_then(|pointer| nearest_hovered_peak(plot_ui, &visible, pointer));

    // Before the markers, so a band never covers the peak line it belongs to.
    add_flare_spans(
        plot_ui,
        &viewport.flares_with_shaded_span(flares, hovered.map(|(_, marked)| marked)),
        viewport.dark_mode,
    );

    if !visible.is_empty() {
        let markers = FlareMarkers {
            markers: visible
                .iter()
                .map(|marked| {
                    (
                        peak_secs(marked),
                        gt_ui_theme::solar_flare_color(
                            marked
                                .flare
                                .classification
                                .peak_flux_watts_per_square_meter(),
                        )
                        .resolve(viewport.dark_mode),
                    )
                })
                .collect(),
            legend_color: gt_ui_theme::FLARE_M_CLASS.resolve(viewport.dark_mode),
        };
        plot_ui.add(OverlayItem::new(text::LAYER_LABEL, markers));
    }

    if let Some((distance, marked)) = hovered {
        nearest.offer(distance, || {
            PlotHoverLabel::SolarFlare(SolarFlareHover::of_archived_flare(marked))
        });
    }
}

/// The flare whose peak marker the pointer is nearest, and how far it is from
/// it, or [`None`] with no marker within [`HOVER_RADIUS_PX`]. Only the
/// horizontal distance counts: a marker runs the plot's full height.
fn nearest_hovered_peak<'f>(
    plot_ui: &egui_plot::PlotUi<'_>,
    visible: &[&'f MarkedFlare],
    pointer: egui::Pos2,
) -> Option<(f32, &'f MarkedFlare)> {
    lines::nearest_under_pointer(
        visible,
        |marked| {
            let x = plot_ui
                .screen_from_plot(PlotPoint::new(peak_secs(marked), 0.0))
                .x;
            (x - pointer.x).abs()
        },
        HOVER_RADIUS_PX,
    )
    .map(|(distance, marked)| (distance, *marked))
}

/// Shade each flare's span in its class colour. The view never re-fits to a
/// band: an unnamed [`Span`] fills the plot's full height on its own and
/// contributes nothing to the auto-bounds.
fn add_flare_spans(plot_ui: &mut egui_plot::PlotUi<'_>, flares: &[&MarkedFlare], dark_mode: bool) {
    for marked in flares {
        let span = FlareSpan::of_flare(&marked.flare);
        let fill = gt_ui_theme::solar_flare_span_fill(
            marked
                .flare
                .classification
                .peak_flux_watts_per_square_meter(),
        )
        .resolve(dark_mode);
        plot_ui.span(Span::new("", span.start_secs..=span.end_secs).fill(fill));
    }
}

/// The plot x a flare is marked at.
fn peak_secs(marked: &MarkedFlare) -> f64 {
    marked.flare.peak.timestamp() as f64
}

/// Pre-formatted tooltip contents for one flare.
pub(super) struct SolarFlareHover {
    lines: Vec<String>,
}

impl SolarFlareHover {
    fn of_archived_flare(marked: &MarkedFlare) -> Self {
        Self {
            lines: text::flare_summary(marked),
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
    use chrono::{DateTime, NaiveDate, Utc};
    use gt_flare::FlareClass;
    use gt_types::SunlitSide;
    use rstest::rstest;

    use super::*;

    pub(super) fn parse_time(time: &str) -> DateTime<Utc> {
        gt_flare::wire::parse_flare_time(time).expect("a catalog time")
    }

    /// One flare of the catalog, with the times the storm's X2.2 had, marked
    /// with no recording loaded to place the receiver.
    pub(super) fn flare(peak: &str, class_type: &str) -> MarkedFlare {
        let peak = parse_time(peak);
        MarkedFlare {
            flare: gt_flare::SolarFlare {
                id: format!("{peak}-FLR-001"),
                begin: peak - chrono::TimeDelta::minutes(28),
                peak,
                end: Some(peak + chrono::TimeDelta::minutes(23)),
                classification: class_type.parse().expect("a published class"),
                source_location: Some("S20W25".to_owned()),
                active_region: Some(13664),
            },
            receiver_side: None,
        }
    }

    fn midnight(day: NaiveDate) -> f64 {
        day.and_hms_opt(0, 0, 0)
            .map_or(0.0, |naive| naive.and_utc().timestamp() as f64)
    }

    /// A span of one UTC day, as the plot shows it.
    fn viewport(day: (i32, u32, u32), span_marking: FlareSpanMarking) -> FlareViewport {
        let day = NaiveDate::from_ymd_opt(day.0, day.1, day.2).unwrap_or_default();
        FlareViewport {
            x_min: midnight(day),
            x_max: midnight(day) + 24.0 * 60.0 * 60.0,
            span_marking,
            dark_mode: true,
        }
    }

    /// The flares' catalog identifiers, which name them in an assertion.
    fn ids<'f>(flares: &[&'f MarkedFlare]) -> Vec<&'f str> {
        flares
            .iter()
            .map(|marked| marked.flare.id.as_str())
            .collect()
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
        let viewport = viewport((2024, 5, 9), FlareSpanMarking::OnlyTheHoveredFlare);
        assert!(viewport.holds(peak_secs(&flare("2024-05-09T09:13Z", "X2.2"))));
        assert!(!viewport.holds(peak_secs(&flare("2024-05-11T01:23Z", "X5.8"))));
    }

    /// Hovering the chip shades every flare the view holds a part of, which
    /// includes one that peaked before the view began and decayed into it.
    #[test]
    fn every_flare_reaching_into_the_view_is_shaded_with_the_chip_hovered() {
        let flares = [
            flare("2024-05-08T23:50Z", "M1.8"),
            flare("2024-05-09T09:13Z", "X2.2"),
            flare("2024-05-10T01:23Z", "X5.8"),
        ];

        let shaded = viewport((2024, 5, 9), FlareSpanMarking::EveryFlareInView)
            .flares_with_shaded_span(&flares, None);
        assert_eq!(
            ids(&shaded),
            [
                "2024-05-08 23:50:00 UTC-FLR-001",
                "2024-05-09 09:13:00 UTC-FLR-001"
            ]
        );
    }

    /// Without the chip hovered, the pointer's own flare is the only one
    /// shaded, even where the view holds others.
    #[test]
    fn only_the_hovered_flare_is_shaded_without_the_chip_hovered() {
        let hovered = flare("2024-05-09T09:13Z", "X2.2");
        let flares = [flare("2024-05-09T01:15Z", "M1.8"), hovered.clone()];

        let viewport = viewport((2024, 5, 9), FlareSpanMarking::OnlyTheHoveredFlare);
        assert_eq!(
            ids(&viewport.flares_with_shaded_span(&flares, Some(&hovered))),
            ["2024-05-09 09:13:00 UTC-FLR-001"]
        );
        assert!(viewport.flares_with_shaded_span(&flares, None).is_empty());
    }

    /// The hover leads with the classification and closes on the side of Earth
    /// the loaded recording puts the receiver on.
    #[test]
    fn the_hover_reports_the_whole_event() {
        let marked = MarkedFlare {
            receiver_side: Some(SunlitSide::Sunlit),
            ..flare("2024-05-09T09:13Z", "X2.2")
        };
        let hover = SolarFlareHover::of_archived_flare(&marked);
        assert_eq!(
            hover.lines,
            [
                "X2.2 solar flare",
                "R3 strong radio blackout",
                "Peak: 2024-05-09 09:13 UTC",
                "08:45–09:36 UTC",
                "AR 13664, S20W25",
                "Receiver: sunlit side",
            ]
        );
    }
}

#[cfg(test)]
mod snapshot_tests {
    use chrono::NaiveDate;
    use egui_plot::{Line, PlotBounds, PlotPoints};
    use gt_test_utils::TestHarness;
    use rstest::rstest;

    use super::tests::{flare, parse_time};
    use super::*;

    /// The May 2024 storm day, as the archive holds it: an X-class flare
    /// among M-class ones, and a C-class flare below the blackout scale.
    fn storm_day() -> Vec<MarkedFlare> {
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
    /// against, and the bands the two span markings shade.
    #[rstest]
    #[case::markers_dark(
        "solar_flare_markers_dark",
        true,
        FlareSpanMarking::OnlyTheHoveredFlare,
        None
    )]
    #[case::markers_light(
        "solar_flare_markers_light",
        false,
        FlareSpanMarking::OnlyTheHoveredFlare,
        None
    )]
    #[case::every_span_dark(
        "solar_flare_spans_dark",
        true,
        FlareSpanMarking::EveryFlareInView,
        None
    )]
    #[case::every_span_light(
        "solar_flare_spans_light",
        false,
        FlareSpanMarking::EveryFlareInView,
        None
    )]
    #[case::hovered_span(
        "solar_flare_hovered_span",
        true,
        FlareSpanMarking::OnlyTheHoveredFlare,
        Some("2024-05-09T09:13Z")
    )]
    fn solar_flare_markers(
        #[case] name: &str,
        #[case] dark_mode: bool,
        #[case] span_marking: FlareSpanMarking,
        #[case] hovered_peak: Option<&str>,
    ) {
        let flares = storm_day();
        let (x_min, x_max) = day_bounds();
        let hovered_peak_secs = hovered_peak.map(|peak| parse_time(peak).timestamp() as f64);
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
                        // A pointer resting exactly on the peak marker, placed
                        // by the plot's own transform.
                        let pointer = hovered_peak_secs
                            .map(|secs| plot_ui.screen_from_plot(PlotPoint::new(secs, 0.0)));
                        add_flare_markers(
                            plot_ui,
                            &flares,
                            FlareViewport {
                                x_min,
                                x_max,
                                span_marking,
                                dark_mode,
                            },
                            pointer,
                            &mut NearestHoverLabel::default(),
                        );
                    });
            });
        harness.run();
        harness.snapshot_loose(name);
    }
}

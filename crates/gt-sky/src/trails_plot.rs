//! The whole-track sky trails plot: every satellite's path across the sky
//! over a track, drawn as a time-ramped polyline, with a scrubber marker and
//! a per-constellation filter/focus.

use egui::{Color32, Pos2, Sense, Stroke, Vec2};

use gt_types::satellites::{Constellation, ConstellationSet, Satellite};
use gt_types::{GpsTime, GpsTimeRange};

use crate::style;
use crate::trails::{SkyTrail, SkyTrails, SlipMark, TrailEpoch, TrailSample};
use crate::{grid, plot_common, projection};

/// The per-frame plot geometry shared by the trail and marker painting.
#[derive(Clone, Copy)]
struct Frame {
    center: Pos2,
    radius: f32,
    time_range: GpsTimeRange,
}

impl Frame {
    fn project(self, azimuth: f32, elevation: f32) -> Pos2 {
        self.center + projection::unit_disc_position(azimuth, elevation) * self.radius
    }
}

/// The whole-track trails plot. Reuses the point plot's grid and projection,
/// so a satellite lands in the same place here as on the per-report plot.
pub struct SkyTrailsPlot<'a> {
    trails: &'a SkyTrails,
    diameter: f32,
    /// Which constellations' trails to draw (the window's checkboxes).
    shown: ConstellationSet,
    /// The focused constellation, if any: its trails stay at full strength
    /// and the rest dim (the window's hover).
    focus: Option<Constellation>,
    /// The scrubbed time: a marker is dropped on each shown trail at this
    /// instant. `None` draws the trails without markers.
    scrub: Option<GpsTime>,
    elevation_mask_deg: Option<f32>,
}

impl<'a> SkyTrailsPlot<'a> {
    pub fn new(trails: &'a SkyTrails, diameter: f32) -> Self {
        Self {
            trails,
            diameter,
            shown: ConstellationSet::all(),
            focus: None,
            scrub: None,
            elevation_mask_deg: None,
        }
    }

    /// Restrict to the given constellations. Defaults to all.
    pub fn shown(self, shown: ConstellationSet) -> Self {
        Self { shown, ..self }
    }

    /// Focus one constellation, dimming the rest.
    pub fn focus(self, focus: Option<Constellation>) -> Self {
        Self { focus, ..self }
    }

    /// Drop a marker on each trail at the given time.
    pub fn scrub(self, scrub: Option<GpsTime>) -> Self {
        Self { scrub, ..self }
    }

    /// Draw the elevation mask as a dashed ring.
    pub fn with_elevation_mask_deg(self, mask_deg: f32) -> Self {
        Self {
            elevation_mask_deg: Some(mask_deg),
            ..self
        }
    }

    pub fn ui(&self, ui: &mut egui::Ui) -> egui::Response {
        let (rect, response) = ui.allocate_exact_size(Vec2::splat(self.diameter), Sense::hover());
        if !ui.is_rect_visible(rect) {
            return response;
        }
        let center = rect.center();
        let radius = self.diameter / 2.0 - style::FULL_RIM_MARGIN_PX;
        // The trails plot is always full size, so the grid is fully labelled.
        grid::draw_grid(ui, center, radius, true);
        // The mask angle when the pointer is on its ring, else `None` - one
        // value that answers both "is it hovered" and "which angle to label".
        let hovered_mask_deg = self
            .elevation_mask_deg
            .zip(response.hover_pos())
            .filter(|&(mask_deg, pointer)| grid::mask_ring_hit(center, radius, mask_deg, pointer))
            .map(|(mask_deg, _)| mask_deg);
        if let Some(mask_deg) = self.elevation_mask_deg {
            grid::draw_mask_ring(ui, center, radius, mask_deg, hovered_mask_deg.is_some());
        }

        // Nothing to ramp against without a time span. The mask ring is still
        // drawn above (context), so honor its hover even with no trails.
        let Some(time_range) = self.trails.time_range else {
            if let Some(mask_deg) = hovered_mask_deg {
                mask_tooltip(ui, &response, mask_deg);
            }
            return response;
        };
        let frame = Frame {
            center,
            radius,
            time_range,
        };
        let dark_mode = ui.visuals().dark_mode;
        let panel = ui.visuals().panel_fill;
        let epochs = &self.trails.epochs;

        // The scrub markers drawn this frame, kept for hover: each satellite at
        // the scrubbed instant, with its position matching the drawn dot.
        let mut markers: Vec<(Satellite, Pos2)> = Vec::new();
        for trail in &self.trails.trails {
            if !self.shown.contains(trail.constellation) {
                continue;
            }
            let base = gt_ui_theme::constellation_color(trail.constellation, dark_mode);
            let focus_factor = self.focus_factor(trail.constellation);
            paint_trail(ui, frame, trail, epochs, base, focus_factor);

            if let Some(scrub) = self.scrub
                && let Some((azimuth, elevation, exact)) = sample_at(trail, epochs, scrub)
            {
                let pos = frame.project(azimuth, elevation);
                ui.painter().circle(
                    pos,
                    style::TRAIL_MARKER_RADIUS_PX,
                    base.gamma_multiply(focus_factor),
                    Stroke::new(style::TRAIL_MARKER_EDGE_PX, panel),
                );
                // Hover shows the report's SNR and fix state, so the marker is
                // only a hover target when it sits on an actual report.
                if let Some(sample) = exact {
                    markers.push((satellite_from_sample(trail, sample), pos));
                }
            }
        }

        // Slip marks on top of the trails they annotate.
        for slip in &self.trails.slips {
            if !self.shown.contains(slip.constellation) {
                continue;
            }
            let color = gt_ui_theme::constellation_color(slip.constellation, dark_mode)
                .gamma_multiply(self.focus_factor(slip.constellation));
            paint_slip_mark(ui, frame.project(slip.azimuth, slip.elevation), color);
        }
        // Hover precedence, most to least specific: a satellite's scrub marker
        // (the moving current-instant dot, the window's primary target), then a
        // slip mark, then the mask ring. At most one tooltip shows, so they
        // never collide.
        let pointer = response.hover_pos();
        let hovered_marker = pointer.and_then(|pointer| {
            let candidates = markers.iter().map(|(satellite, pos)| (satellite, *pos));
            plot_common::nearest_within(candidates, pointer, style::MARK_HOVER_RADIUS_PX)
        });
        let hovered_slip = pointer
            .and_then(|pointer| nearest_slip(&self.trails.slips, self.shown, frame, pointer));
        if let Some(satellite) = hovered_marker {
            show_marker_tooltip(ui, &response, satellite);
        } else if let Some(slip) = hovered_slip {
            show_slip_tooltip(ui, &response, slip);
        } else if let Some(mask_deg) = hovered_mask_deg {
            mask_tooltip(ui, &response, mask_deg);
        }

        response
    }

    /// The alpha for a constellation given the current focus: full when it is
    /// focused or nothing is, dimmed otherwise.
    fn focus_factor(&self, constellation: Constellation) -> f32 {
        match self.focus {
            Some(focused) if focused != constellation => style::TRAIL_DIMMED_ALPHA,
            _ => 1.0,
        }
    }
}

/// Paint one trail: a polyline through its samples, each segment's alpha
/// ramping with time so the sweep direction reads, broken wherever an epoch
/// falls between two samples (the satellite dropped out there).
fn paint_trail(
    ui: &egui::Ui,
    frame: Frame,
    trail: &SkyTrail,
    epochs: &[TrailEpoch],
    color: Color32,
    focus_factor: f32,
) {
    let painter = ui.painter();
    for pair in trail.samples.windows(2) {
        let [a, b] = pair else {
            continue;
        };
        if epoch_between(epochs, a.time, b.time) {
            continue; // the satellite was absent between these samples
        }
        let ramp = style::TRAIL_MIN_ALPHA
            + (style::TRAIL_MAX_ALPHA - style::TRAIL_MIN_ALPHA)
                * frame.time_range.normalize(b.time);
        painter.line_segment(
            [
                frame.project(a.azimuth, a.elevation),
                frame.project(b.azimuth, b.elevation),
            ],
            Stroke::new(
                style::TRAIL_WIDTH_PX,
                color.gamma_multiply(ramp * focus_factor),
            ),
        );
    }
}

/// Paint a slip mark as an "×" centered on `pos`, distinct from the round
/// scrub markers and the trail lines.
fn paint_slip_mark(ui: &egui::Ui, pos: Pos2, color: Color32) {
    let r = style::SLIP_MARK_RADIUS_PX;
    let stroke = Stroke::new(style::SLIP_MARK_WIDTH_PX, color);
    let painter = ui.painter();
    painter.line_segment([pos + Vec2::new(-r, -r), pos + Vec2::new(r, r)], stroke);
    painter.line_segment([pos + Vec2::new(-r, r), pos + Vec2::new(r, -r)], stroke);
}

/// The satellite behind a scrub marker, for its hover tooltip - a report's
/// sample carries everything the tooltip needs.
fn satellite_from_sample(trail: &SkyTrail, sample: &TrailSample) -> Satellite {
    Satellite::new(
        trail.constellation,
        trail.prn.value(),
        Some(sample.elevation),
        Some(sample.azimuth),
        sample.snr.map(|snr| snr.value()),
        sample.in_fix,
    )
}

/// Show the hover tooltip for a satellite's scrub marker, identical to the
/// per-report plot's satellite tooltip.
fn show_marker_tooltip(ui: &egui::Ui, response: &egui::Response, satellite: &Satellite) {
    egui::Tooltip::always_open(
        ui.ctx().clone(),
        ui.layer_id(),
        response
            .id
            .with(("marker", satellite.constellation(), satellite.prn())),
        egui::PopupAnchor::Pointer,
    )
    .show(|ui| plot_common::satellite_tooltip(ui, satellite));
}

/// Show the hover tooltip for a slip mark.
fn show_slip_tooltip(ui: &egui::Ui, response: &egui::Response, slip: &SlipMark) {
    egui::Tooltip::always_open(
        ui.ctx().clone(),
        ui.layer_id(),
        response.id.with(("slip", slip.constellation, slip.prn)),
        egui::PopupAnchor::Pointer,
    )
    .show(|ui| {
        ui.label(egui::RichText::new(slip_label(slip)).strong());
        let degree = gt_ui_theme::DEGREE_SIGN;
        ui.label(format!("Elevation {:.0}{degree}", slip.elevation));
        ui.label(format!("Azimuth {:.0}{degree}", slip.azimuth));
    });
}

/// Show the elevation-mask ring's hover tooltip, naming it and its angle so it
/// reads as a labelled element rather than a mystery circle.
fn mask_tooltip(ui: &egui::Ui, response: &egui::Response, mask_deg: f32) {
    egui::Tooltip::always_open(
        ui.ctx().clone(),
        ui.layer_id(),
        response.id.with("mask_ring"),
        egui::PopupAnchor::Pointer,
    )
    .show(|ui| {
        ui.label(egui::RichText::new("Elevation mask").strong());
        ui.label(format!("{mask_deg:.0}{}", gt_ui_theme::DEGREE_SIGN));
    });
}

/// The shown slip mark nearest to `pointer`, within
/// [`style::SLIP_MARK_HOVER_RADIUS_PX`].
fn nearest_slip(
    slips: &[SlipMark],
    shown: ConstellationSet,
    frame: Frame,
    pointer: Pos2,
) -> Option<&SlipMark> {
    let candidates = slips
        .iter()
        .filter(|slip| shown.contains(slip.constellation))
        .map(|slip| (slip, frame.project(slip.azimuth, slip.elevation)));
    plot_common::nearest_within(candidates, pointer, style::SLIP_MARK_HOVER_RADIUS_PX)
}

/// "G05 GPS - lost lock" for a slip mark's tooltip header.
fn slip_label(slip: &SlipMark) -> String {
    format!(
        "{} - {}",
        plot_common::satellite_designator(slip.constellation, slip.prn),
        slip.cause.label(),
    )
}

/// Whether any report epoch falls strictly between `a` and `b` - i.e. the
/// satellite skipped a report there, so its trail should break.
fn epoch_between(epochs: &[TrailEpoch], a: GpsTime, b: GpsTime) -> bool {
    let after_a = epochs.partition_point(|e| e.time <= a);
    epochs.get(after_a).is_some_and(|e| e.time < b)
}

/// The satellite's `(azimuth, elevation)` at `time`, plus the underlying
/// [`TrailSample`] when `time` lands exactly on a report (so the caller gets
/// its SNR and fix state without a second lookup). Between reports the position
/// is interpolated and the sample is `None`. `None` overall when `time` is
/// outside the trail or inside one of its gaps.
fn sample_at<'a>(
    trail: &'a SkyTrail,
    epochs: &[TrailEpoch],
    time: GpsTime,
) -> Option<(f32, f32, Option<&'a TrailSample>)> {
    let samples = &trail.samples;
    let idx = samples.partition_point(|s| s.time < time);
    // Exact hit on a sample.
    if let Some(s) = samples.get(idx)
        && s.time == time
    {
        return Some((s.azimuth, s.elevation, Some(s)));
    }
    // Otherwise interpolate between the bracketing samples, unless the pair
    // spans a gap or `time` is outside the trail.
    let (Some(a), Some(b)) = (
        idx.checked_sub(1).and_then(|i| samples.get(i)),
        samples.get(idx),
    ) else {
        return None;
    };
    if epoch_between(epochs, a.time, b.time) {
        return None;
    }
    let span = b.time.signed_duration_since(a.time).num_milliseconds();
    if span <= 0 {
        return Some((a.azimuth, a.elevation, Some(a)));
    }
    let f = time.signed_duration_since(a.time).num_milliseconds() as f32 / span as f32;
    Some((
        a.azimuth + (b.azimuth - a.azimuth) * f,
        a.elevation + (b.elevation - a.elevation) * f,
        None,
    ))
}

#[cfg(test)]
mod tests {
    use chrono::{DateTime, Duration, Utc};
    use rstest::rstest;

    use gt_test_utils::TestHarness;
    use gt_types::satellites::{Constellation, ConstellationSet, Satellite, Satellites};
    use gt_types::{GpsTime, Latitude, Longitude, NavPoint, PointIdx, TimePositionVelocity};

    use super::{SkyTrail, SkyTrailsPlot, SlipMark, TrailEpoch, epoch_between, sample_at};
    use crate::extract_trails;
    use crate::trails::{SkyTrails, TrailSample};

    fn start() -> DateTime<Utc> {
        DateTime::<Utc>::from_timestamp(1_748_000_000, 0).expect("valid")
    }

    fn at(secs: i64) -> GpsTime {
        GpsTime::from_utc(start() + Duration::seconds(secs))
    }

    // The point index is irrelevant to the time-only helpers under test.
    fn epoch(secs: i64) -> TrailEpoch {
        TrailEpoch {
            time: at(secs),
            point_index: PointIdx::new(0),
        }
    }

    #[rstest]
    #[case::adjacent(0, 1, false)]
    #[case::skips_one(0, 2, true)]
    #[case::skips_two(1, 3, true)]
    #[case::last_pair_adjacent(2, 3, false)]
    fn epoch_between_detects_skipped_reports(
        #[case] a: i64,
        #[case] b: i64,
        #[case] expected: bool,
    ) {
        let epochs = [epoch(0), epoch(1), epoch(2), epoch(3)];
        assert_eq!(epoch_between(&epochs, at(a), at(b)), expected);
    }

    fn trail_sample(secs: i64, azimuth: f32, elevation: f32) -> TrailSample {
        TrailSample {
            time: at(secs),
            point_index: PointIdx::new(0),
            azimuth,
            elevation,
            snr: None,
            in_fix: true,
        }
    }

    /// A trail with samples at t0 and t2 - the satellite is absent at the t1
    /// epoch, so `[epoch(0), epoch(1), epoch(2)]` puts a gap between them.
    fn gapped_trail() -> SkyTrail {
        SkyTrail {
            constellation: Constellation::Gps,
            prn: gt_types::satellites::Prn::new(5),
            samples: vec![trail_sample(0, 40.0, 60.0), trail_sample(2, 60.0, 40.0)],
        }
    }

    #[rstest]
    #[case::exact_hit(at(0), Some((40.0, 60.0)))]
    #[case::in_gap(at(1), None)]
    #[case::before_first_sample(at(-1), None)]
    #[case::after_last_sample(at(5), None)]
    fn sample_at_respects_trail_bounds_and_gaps(
        #[case] time: GpsTime,
        #[case] expected: Option<(f32, f32)>,
    ) {
        let epochs = [epoch(0), epoch(1), epoch(2)];
        let position = sample_at(&gapped_trail(), &epochs, time).map(|(az, el, _)| (az, el));
        assert_eq!(position, expected);
    }

    #[test]
    fn sample_at_carries_the_sample_only_on_an_exact_hit() {
        let trail = gapped_trail();
        let epochs = [epoch(0), epoch(2)];
        // Exactly on a report: the sample comes back for the tooltip.
        let (_, _, exact) = sample_at(&trail, &epochs, at(0)).expect("hit");
        assert!(exact.is_some());
        // Interpolated between reports: position but no single sample.
        let (az, el, exact) = sample_at(&trail, &epochs, at(1)).expect("interpolated");
        assert_eq!((az, el), (50.0, 50.0));
        assert!(exact.is_none());
    }

    struct Spec {
        c: Constellation,
        prn: u32,
        az: (f32, f32),
        el: (f32, f32),
        absent: &'static [usize],
    }

    /// A synthetic track: several satellites drifting across the sky over ten
    /// report epochs, one dropping out mid-track to leave a gap.
    fn demo_trails() -> SkyTrails {
        const EPOCHS: usize = 10;
        let specs = [
            Spec {
                c: Constellation::Gps,
                prn: 5,
                az: (40.0, 95.0),
                el: (58.0, 71.0),
                absent: &[],
            },
            Spec {
                c: Constellation::Gps,
                prn: 12,
                az: (85.0, 130.0),
                el: (20.0, 47.0),
                absent: &[],
            },
            Spec {
                c: Constellation::Galileo,
                prn: 3,
                az: (60.0, 30.0),
                el: (52.0, 40.0),
                absent: &[],
            },
            Spec {
                c: Constellation::Glonass,
                prn: 9,
                az: (170.0, 205.0),
                el: (48.0, 28.0),
                absent: &[],
            },
            Spec {
                c: Constellation::Beidou,
                prn: 14,
                az: (250.0, 230.0),
                el: (66.0, 51.0),
                absent: &[4, 5],
            },
        ];
        let lerp = |(a, b): (f32, f32), f: f32| a + (b - a) * f;
        let points = (0..EPOCHS)
            .map(|i| {
                let f = i as f32 / (EPOCHS - 1) as f32;
                let sats: Vec<Satellite> = specs
                    .iter()
                    .filter(|s| !s.absent.contains(&i))
                    .map(|s| {
                        Satellite::new(
                            s.c,
                            s.prn,
                            Some(lerp(s.el, f)),
                            Some(lerp(s.az, f)),
                            Some(40.0),
                            true,
                        )
                    })
                    .collect();
                let tpv = TimePositionVelocity::builder()
                    .time(at(i as i64))
                    .lat(Latitude::new(55.0))
                    .lon(Longitude::new(12.0))
                    .build();
                NavPoint::new(tpv, Some(Satellites::new(None, None, sats)))
            })
            .collect();
        extract_trails(&gt_test_utils::loaded_track_with_points(points))
    }

    fn snapshot(
        name: &str,
        shown: ConstellationSet,
        focus: Option<Constellation>,
        scrub: Option<GpsTime>,
    ) {
        let trails = demo_trails();
        let mut harness = TestHarness::builder()
            .size(egui::vec2(320.0, 320.0))
            .theme(true)
            .ui(move |ui| {
                SkyTrailsPlot::new(&trails, 300.0)
                    .shown(shown)
                    .focus(focus)
                    .scrub(scrub)
                    .with_elevation_mask_deg(10.0)
                    .ui(ui);
            });
        harness.run();
        harness.snapshot(name);
    }

    #[test]
    fn sky_trails_full() {
        // Every trail, time-ramped, with the BeiDou gap.
        snapshot("sky_trails_full", ConstellationSet::all(), None, None);
    }

    #[test]
    fn sky_trails_focused() {
        // GPS focused: its trails stay bright, the rest dim.
        snapshot(
            "sky_trails_focused",
            ConstellationSet::all(),
            Some(Constellation::Gps),
            None,
        );
    }

    #[test]
    fn sky_trails_filtered() {
        // Only GPS and Galileo shown.
        let shown = ConstellationSet::single(Constellation::Gps).with(Constellation::Galileo);
        snapshot("sky_trails_filtered", shown, None, None);
    }

    #[test]
    fn sky_trails_scrubbed() {
        // A marker on each trail at a mid-track instant.
        snapshot(
            "sky_trails_scrubbed",
            ConstellationSet::all(),
            None,
            Some(at(4)),
        );
    }

    #[test]
    fn sky_trails_with_slips() {
        use gt_types::satellites::{Prn, SlipCause};

        let mut trails = demo_trails();
        // Two slip marks (an "×" each), on distinct constellations.
        trails.slips = vec![
            SlipMark {
                constellation: Constellation::Gps,
                prn: Prn::new(5),
                azimuth: 70.0,
                elevation: 64.0,
                cause: SlipCause::LostLock,
            },
            SlipMark {
                constellation: Constellation::Galileo,
                prn: Prn::new(3),
                azimuth: 45.0,
                elevation: 46.0,
                cause: SlipCause::SnrDrop,
            },
        ];
        let mut harness = TestHarness::builder()
            .size(egui::vec2(320.0, 320.0))
            .theme(true)
            .ui(move |ui| {
                SkyTrailsPlot::new(&trails, 300.0)
                    .with_elevation_mask_deg(10.0)
                    .ui(ui);
            });
        harness.run();
        harness.snapshot("sky_trails_with_slips");
    }

    #[test]
    fn sky_trails_mask_ring_hover_lights_up_and_labels() {
        use std::cell::Cell;
        use std::rc::Rc;

        let trails = demo_trails();
        // The plot's rendered centre, captured so the hover point can be placed
        // on the ring regardless of the harness's layout margins.
        let center = Rc::new(Cell::new(egui::Pos2::ZERO));
        let sink = Rc::clone(&center);
        let mut harness = TestHarness::builder()
            .size(egui::vec2(320.0, 360.0))
            .theme(true)
            .ui(move |ui| {
                let response = SkyTrailsPlot::new(&trails, 300.0)
                    .with_elevation_mask_deg(10.0)
                    .ui(ui);
                sink.set(response.rect.center());
            });
        harness.run();

        // Hover a point on the mask ring (its east side).
        let radius = 300.0 / 2.0 - super::style::FULL_RIM_MARGIN_PX;
        let ring_radius = radius * super::projection::unit_disc_radius(10.0);
        let on_ring = center.get() + egui::vec2(ring_radius, 0.0);
        harness.inner.hover_at(on_ring);
        // Tooltips appear after egui's hover delay; step until it elapses.
        for _ in 0..60 {
            harness.run();
        }
        harness.snapshot_loose("sky_trails_mask_ring_hover");
    }

    #[test]
    fn sky_trails_marker_hover_shows_the_satellite() {
        use std::cell::Cell;
        use std::rc::Rc;

        let trails = demo_trails();
        // The GPS-5 satellite's position at the scrubbed epoch, so the hover
        // point can land on its marker.
        let scrub = at(4);
        let gps5 = trails
            .trails
            .iter()
            .find(|t| t.constellation == Constellation::Gps && t.prn.value() == 5)
            .expect("gps-5 trail");
        let sample = gps5.sample_exactly_at(scrub).expect("sample at the epoch");
        let offset = super::projection::unit_disc_position(sample.azimuth, sample.elevation);

        let center = Rc::new(Cell::new(egui::Pos2::ZERO));
        let sink = Rc::clone(&center);
        let trails_for_ui = trails.clone();
        let mut harness = TestHarness::builder()
            .size(egui::vec2(320.0, 360.0))
            .theme(true)
            .ui(move |ui| {
                let response = SkyTrailsPlot::new(&trails_for_ui, 300.0)
                    .scrub(Some(scrub))
                    .ui(ui);
                sink.set(response.rect.center());
            });
        harness.run();

        let radius = 300.0 / 2.0 - super::style::FULL_RIM_MARGIN_PX;
        harness.inner.hover_at(center.get() + offset * radius);
        for _ in 0..60 {
            harness.run();
        }
        harness.snapshot_loose("sky_trails_marker_hover");
    }

    #[test]
    fn satellite_from_sample_carries_the_reports_facts() {
        let trail = SkyTrail {
            constellation: Constellation::Gps,
            prn: gt_types::satellites::Prn::new(5),
            samples: vec![],
        };
        let sample = TrailSample {
            time: at(0),
            point_index: PointIdx::new(0),
            azimuth: 40.0,
            elevation: 60.0,
            snr: Some(gt_types::satellites::Snr::new(42.0)),
            in_fix: true,
        };

        let satellite = super::satellite_from_sample(&trail, &sample);
        assert_eq!(satellite.constellation(), Constellation::Gps);
        assert_eq!(satellite.prn().value(), 5);
        assert_eq!(satellite.azimuth(), Some(40.0));
        assert_eq!(satellite.elevation(), Some(60.0));
        assert_eq!(satellite.snr().map(|s| s.value()), Some(42.0));
        assert!(satellite.in_fix());
    }

    #[test]
    fn slip_label_names_the_satellite_and_cause() {
        use gt_types::satellites::{Prn, SlipCause};
        let slip = SlipMark {
            constellation: Constellation::Gps,
            prn: Prn::new(5),
            azimuth: 70.0,
            elevation: 64.0,
            cause: SlipCause::LostLock,
        };
        assert_eq!(super::slip_label(&slip), "G05 GPS - lost lock");
    }

    #[rstest]
    // A slip at azimuth 90 / elevation 0 sits on the east rim (radius = the
    // frame radius from the center). Pointer near it hits; far misses; a
    // hidden constellation is never selected.
    #[case::hits(egui::pos2(102.0, 100.0), ConstellationSet::all(), true)]
    #[case::beyond_radius(egui::pos2(130.0, 100.0), ConstellationSet::all(), false)]
    #[case::constellation_hidden(egui::pos2(102.0, 100.0), ConstellationSet::empty(), false)]
    fn nearest_slip_respects_radius_and_filter(
        #[case] pointer: egui::Pos2,
        #[case] shown: ConstellationSet,
        #[case] expected: bool,
    ) {
        use gt_types::satellites::{Prn, SlipCause};
        let frame = super::Frame {
            center: egui::pos2(100.0, 100.0),
            radius: 1.0,
            time_range: gt_types::GpsTimeRange::new(at(0), at(1)),
        };
        // Placed on the east rim: unit-disc (1, 0) -> center + (1, 0).
        let slips = [SlipMark {
            constellation: Constellation::Gps,
            prn: Prn::new(5),
            azimuth: 90.0,
            elevation: 0.0,
            cause: SlipCause::LostLock,
        }];
        let hit = super::nearest_slip(&slips, shown, frame, pointer).is_some();
        assert_eq!(hit, expected);
    }
}

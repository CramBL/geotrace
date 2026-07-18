//! The whole-track sky trails plot: every satellite's path across the sky
//! over a track, drawn as a time-ramped polyline, with a scrubber marker and
//! a per-constellation filter/focus.

use egui::{Color32, Pos2, Sense, Stroke, Vec2};

use gt_types::satellites::{Constellation, ConstellationSet};
use gt_types::{GpsTime, GpsTimeRange};

use crate::style;
use crate::trails::{SkyTrail, SkyTrails, TrailEpoch};
use crate::{grid, projection};

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
        if let Some(mask_deg) = self.elevation_mask_deg {
            grid::draw_mask_ring(ui, center, radius, mask_deg);
        }

        // Nothing to ramp against without a time span.
        let Some(time_range) = self.trails.time_range else {
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

        for trail in &self.trails.trails {
            if !self.shown.contains(trail.constellation) {
                continue;
            }
            let base = gt_ui_theme::constellation_color(trail.constellation, dark_mode);
            let focus_factor = match self.focus {
                Some(focused) if focused != trail.constellation => style::TRAIL_DIMMED_ALPHA,
                _ => 1.0,
            };
            paint_trail(ui, frame, trail, epochs, base, focus_factor);

            if let Some(scrub) = self.scrub
                && let Some((azimuth, elevation)) = sample_at(trail, epochs, scrub)
            {
                ui.painter().circle(
                    frame.project(azimuth, elevation),
                    style::TRAIL_MARKER_RADIUS_PX,
                    base.gamma_multiply(focus_factor),
                    Stroke::new(style::TRAIL_MARKER_EDGE_PX, panel),
                );
            }
        }
        response
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

/// Whether any report epoch falls strictly between `a` and `b` - i.e. the
/// satellite skipped a report there, so its trail should break.
fn epoch_between(epochs: &[TrailEpoch], a: GpsTime, b: GpsTime) -> bool {
    let after_a = epochs.partition_point(|e| e.time <= a);
    epochs.get(after_a).is_some_and(|e| e.time < b)
}

/// The satellite's interpolated `(azimuth, elevation)` at `time`, or `None`
/// when `time` is outside the trail or inside one of its gaps.
fn sample_at(trail: &SkyTrail, epochs: &[TrailEpoch], time: GpsTime) -> Option<(f32, f32)> {
    let samples = &trail.samples;
    let idx = samples.partition_point(|s| s.time < time);
    // Exact hit on a sample.
    if let Some(s) = samples.get(idx)
        && s.time == time
    {
        return Some((s.azimuth, s.elevation));
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
        return Some((a.azimuth, a.elevation));
    }
    let f = time.signed_duration_since(a.time).num_milliseconds() as f32 / span as f32;
    Some((
        a.azimuth + (b.azimuth - a.azimuth) * f,
        a.elevation + (b.elevation - a.elevation) * f,
    ))
}

#[cfg(test)]
mod tests {
    use chrono::{DateTime, Duration, Utc};
    use rstest::rstest;

    use gt_test_utils::TestHarness;
    use gt_types::satellites::{Constellation, ConstellationSet, Satellite, Satellites};
    use gt_types::{GpsTime, Latitude, Longitude, NavPoint, PointIdx, TimePositionVelocity};

    use super::{SkyTrail, SkyTrailsPlot, TrailEpoch, epoch_between, sample_at};
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
        assert_eq!(sample_at(&gapped_trail(), &epochs, time), expected);
    }

    #[test]
    fn sample_at_interpolates_a_contiguous_span() {
        // Without the skipped t1 epoch, the same span interpolates linearly.
        let contiguous = [epoch(0), epoch(2)];
        assert_eq!(
            sample_at(&gapped_trail(), &contiguous, at(1)),
            Some((50.0, 50.0))
        );
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
}

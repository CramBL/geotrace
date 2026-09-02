use crate::AnalysisConfig;
use gt_analysis::clock_offset::ClockOffsetExcursion;
use gt_analysis::satellite_utilization::UtilAnomaly;
use gt_egui_mipmap::{MipMap, WrapPeriod};
use gt_types::LoadedFile;
use gt_types::satellites::{Constellation, ConstellationSet};
use uom::si::angle::degree;

const MICROS_PER_SEC: f64 = 1_000_000.0;

/// A track's mipmap series together with the index of the file it belongs to,
/// which the plot learns only once that file joins the loaded-files list.
#[derive(Debug, Clone)]
pub(crate) struct PlacedTrackSeries {
    pub fi: usize,
    pub series: TrackSeries,
}

impl PlacedTrackSeries {
    pub(crate) fn track_ref(&self) -> gt_types::TrackRef {
        gt_types::TrackRef::new(
            gt_types::FileIdx::new(self.fi),
            gt_types::TrackIdx::new(self.series.ti),
        )
    }
}

/// Mipmap series for a single track.
///
/// Built from all points in the track regardless of current visibility or filter.
/// Visibility and time-range clamping are applied at render time in
/// [`super::plot_widget`] so the cache stays valid across filter changes.
#[derive(Debug, Clone)]
pub(crate) struct TrackSeries {
    /// Track index within its file.
    pub ti: usize,
    /// `(x_min, x_max)` in Unix seconds, or `None` when the track has no
    /// points.
    pub x_range: Option<(f64, f64)>,
    pub total_seen: MipMap,
    pub total_fix: MipMap,
    pub gps_seen: MipMap,
    pub gps_fix: MipMap,
    pub glonass_seen: MipMap,
    pub glonass_fix: MipMap,
    pub galileo_seen: MipMap,
    pub galileo_fix: MipMap,
    pub beidou_seen: MipMap,
    pub beidou_fix: MipMap,
    pub navic_seen: MipMap,
    pub navic_fix: MipMap,
    pub qzss_seen: MipMap,
    pub qzss_fix: MipMap,
    /// Constellations that appear at least once in this track's satellite
    /// reports.  The plot uses the union across tracks to select which
    /// per-constellation chips and lines to show, so a constellation with no
    /// data never clutters the UI.
    pub present: ConstellationSet,
    pub velocity_kmh: MipMap,
    pub eph_m: MipMap,
    pub heading_deg: MipMap,
    /// GPS-clock lead over the host system clock, in milliseconds.
    /// Positive = GPS clock ahead, negative = system clock ahead.
    /// Only present when the TPV record carries a system timestamp.
    ///
    /// Excludes the samples in [`Self::clock_excursions`]: a single sample
    /// carrying a whole recording gap would otherwise set the auto-bounds of
    /// the y-axis every other metric shares, flattening the plot.  Those samples
    /// are drawn by the off-scale indicator instead, never dropped.
    pub clock_delta_ms: MipMap,
    /// Isolated departures of the clock offset from this track's baseline, kept
    /// off [`Self::clock_delta_ms`] and marked on their own.  Threshold-
    /// dependent: recomputed by [`TrackSeries::apply_analysis`].
    pub clock_excursions: Vec<ClockOffsetExcursion>,
    /// Satellite utilization rate (percent), all constellations combined, and
    /// broken down per constellation.  Mask-dependent: recomputed by
    /// [`TrackSeries::apply_analysis`] when the elevation mask changes.
    pub util_all: MipMap,
    pub util_gps: MipMap,
    pub util_glonass: MipMap,
    pub util_galileo: MipMap,
    pub util_beidou: MipMap,
    pub util_navic: MipMap,
    pub util_qzss: MipMap,
    /// Epochs where the receiver used a satellite below the elevation mask, so
    /// that satellite is excluded from the utilization rate.  Surfaced as plot
    /// markers; also mask-dependent.
    pub util_anomalies: Vec<UtilAnomaly>,
    /// Loss-of-lock (slip) rate per minute, all constellations combined and per
    /// constellation.  Depends on the elevation mask, the SNR-drop threshold,
    /// and the averaging window, so it is recomputed by
    /// [`TrackSeries::apply_analysis`] whenever any of those change.
    pub slip_all: MipMap,
    pub slip_gps: MipMap,
    pub slip_glonass: MipMap,
    pub slip_galileo: MipMap,
    pub slip_beidou: MipMap,
    pub slip_navic: MipMap,
    pub slip_qzss: MipMap,
    /// The track's ad-hoc sensor channels, one entry per channel in the
    /// track's own order. Dynamic (channels are per-file names), unlike the
    /// fixed metric fields above.
    pub channels: Vec<ChannelSeries>,
}

/// One ad-hoc channel's plot series: a line per component, on the channel's
/// own sample clock (channels are correlated with the track by time, not
/// resampled onto the fixes).
#[derive(Debug, Clone)]
pub(crate) struct ChannelSeries {
    pub name: String,
    /// The producer's unit label (`g`, `deg`), shown in the chip and the
    /// line names. Purely presentational here - the plot draws raw values.
    pub unit: Option<String>,
    /// One line per component, in channel order. A scalar channel is a
    /// single component labelled by the channel name alone.
    pub components: Vec<ChannelComponentSeries>,
}

/// One channel component's line: its display label and mipmapped samples.
#[derive(Debug, Clone)]
pub(crate) struct ChannelComponentSeries {
    /// `accel.x` for a vector component, `incline` for a scalar channel.
    pub label: String,
    pub mipmap: MipMap,
}

/// Build the per-component plot series of one channel.
fn build_channel_series(channel: &gt_types::Channel) -> ChannelSeries {
    let columns = channel.component_count();
    let period = channel
        .period
        .and_then(|period| WrapPeriod::new(period.get::<degree>()));
    let times: Vec<f64> = channel
        .times
        .iter()
        .map(|t| t.timestamp_micros() as f64 / MICROS_PER_SEC)
        .collect();
    let components = (0..columns)
        .map(|col| {
            let pts: Vec<[f64; 2]> = times
                .iter()
                .zip(channel.values.chunks(columns))
                .filter_map(|(&t, row)| row.get(col).map(|&v| [t, v]))
                .collect();
            let label = match channel.components.get(col) {
                Some(component) => format!("{}.{component}", channel.name),
                None => channel.name.clone(),
            };
            ChannelComponentSeries {
                label,
                mipmap: match period {
                    Some(period) => MipMap::build_wrapping(pts, period),
                    None => MipMap::build(pts),
                },
            }
        })
        .collect();
    ChannelSeries {
        name: channel.name.clone(),
        unit: channel.unit.as_ref().map(ToString::to_string),
        components,
    }
}

impl TrackSeries {
    /// Recompute only the analysis-dependent series (utilization rate +
    /// anomalies and slip rate) for `track` under `analysis`, leaving the
    /// analysis-independent mipmaps intact.
    pub(crate) fn apply_analysis(
        &mut self,
        track: &gt_types::LoadedTrack,
        analysis: AnalysisConfig,
    ) {
        let u = gt_analysis::satellite_utilization::compute_util(
            &track.points,
            analysis.elevation_mask_deg,
        );
        self.util_all = MipMap::build(u.all);
        self.util_gps = MipMap::build(u.gps);
        self.util_glonass = MipMap::build(u.glonass);
        self.util_galileo = MipMap::build(u.galileo);
        self.util_beidou = MipMap::build(u.beidou);
        self.util_navic = MipMap::build(u.navic);
        self.util_qzss = MipMap::build(u.qzss);
        self.util_anomalies = u.anomalies;

        let s = gt_analysis::loss_of_lock::slip_rate_series(
            &track.points,
            analysis.elevation_mask_deg,
            analysis.snr_drop_db,
            analysis.slip_window_min,
        );
        self.slip_all = MipMap::build(s.all);
        self.slip_gps = MipMap::build(s.gps);
        self.slip_glonass = MipMap::build(s.glonass);
        self.slip_galileo = MipMap::build(s.galileo);
        self.slip_beidou = MipMap::build(s.beidou);
        self.slip_navic = MipMap::build(s.navic);
        self.slip_qzss = MipMap::build(s.qzss);

        let (clock_delta_pts, excursions) =
            clock_delta_series(track, analysis.clock_excursion_threshold_s);
        self.clock_delta_ms = MipMap::build(clock_delta_pts);
        self.clock_excursions = excursions;
    }
}

/// The clock-offset line points and the excursions held back from it.
///
/// The plot marks each held-back sample at the edge of the view, with its true
/// offset on hover.
fn clock_delta_series(
    track: &gt_types::LoadedTrack,
    threshold_s: f32,
) -> (Vec<[f64; 2]>, Vec<ClockOffsetExcursion>) {
    let excursions = gt_analysis::clock_offset::detect_excursions(&track.points, threshold_s);
    let excluded = gt_analysis::clock_offset::excursion_indices(&excursions);
    let points = track
        .points
        .iter()
        .enumerate()
        .filter(|(i, _)| excluded.binary_search(i).is_err())
        .filter_map(|(_, point)| {
            let delta_ms = point.tpv.gps_system_clock_offset()?.num_milliseconds();
            Some([point.tpv.time().as_secs_f64(), delta_ms as f64])
        })
        .collect();
    (points, excursions)
}

/// Build mipmap series for every track in a single file, using `fi` as the file
/// index (for cache keying).
///
/// No visibility check or time filter is applied - that is done at render time
/// so the cache stays valid across filter changes without a rebuild.
pub(crate) fn build_file_series(file: &LoadedFile, analysis: AnalysisConfig) -> Vec<TrackSeries> {
    file.tracks
        .iter()
        .enumerate()
        .map(|(ti, track)| build_track_series(ti, track, analysis))
        .collect()
}

/// Build mipmap series for every track in every file.
pub(crate) fn build_all_series(
    files: &[LoadedFile],
    analysis: AnalysisConfig,
) -> Vec<PlacedTrackSeries> {
    files
        .iter()
        .enumerate()
        .flat_map(|(fi, file)| {
            build_file_series(file, analysis)
                .into_iter()
                .map(move |series| PlacedTrackSeries { fi, series })
        })
        .collect()
}

fn build_track_series(
    ti: usize,
    track: &gt_types::LoadedTrack,
    analysis: AnalysisConfig,
) -> TrackSeries {
    let mut total_seen_pts: Vec<[f64; 2]> = Vec::with_capacity(track.points.len());
    let mut total_fix_pts: Vec<[f64; 2]> = Vec::with_capacity(track.points.len());
    let mut gps_seen_pts: Vec<[f64; 2]> = Vec::new();
    let mut gps_fix_pts: Vec<[f64; 2]> = Vec::new();
    let mut glonass_seen_pts: Vec<[f64; 2]> = Vec::new();
    let mut glonass_fix_pts: Vec<[f64; 2]> = Vec::new();
    let mut galileo_seen_pts: Vec<[f64; 2]> = Vec::new();
    let mut galileo_fix_pts: Vec<[f64; 2]> = Vec::new();
    let mut beidou_seen_pts: Vec<[f64; 2]> = Vec::new();
    let mut beidou_fix_pts: Vec<[f64; 2]> = Vec::new();
    let mut navic_seen_pts: Vec<[f64; 2]> = Vec::new();
    let mut navic_fix_pts: Vec<[f64; 2]> = Vec::new();
    let mut qzss_seen_pts: Vec<[f64; 2]> = Vec::new();
    let mut qzss_fix_pts: Vec<[f64; 2]> = Vec::new();
    let mut present = ConstellationSet::empty();
    let mut velocity_kmh_pts: Vec<[f64; 2]> = Vec::with_capacity(track.points.len());
    let mut eph_m_pts: Vec<[f64; 2]> = Vec::new();
    let mut heading_deg_pts: Vec<[f64; 2]> = Vec::new();

    for point in &track.points {
        let t = point.tpv.time().as_secs_f64();

        if let Some(sats) = &point.satellites {
            total_seen_pts.push([t, sats.satellite_count() as f64]);
            total_fix_pts.push([t, sats.fix_count() as f64]);

            let seen_and_fix = |c| {
                sats.by_constellation(c)
                    .fold((0usize, 0usize), |(seen, fix), sat| {
                        (seen + 1, fix + sat.in_fix() as usize)
                    })
            };
            let (gps_seen, gps_fix) = seen_and_fix(Constellation::Gps);
            let (gln_seen, gln_fix) = seen_and_fix(Constellation::Glonass);
            let (gal_seen, gal_fix) = seen_and_fix(Constellation::Galileo);
            let (bei_seen, bei_fix) = seen_and_fix(Constellation::Beidou);
            let (nav_seen, nav_fix) = seen_and_fix(Constellation::Navic);
            let (qzs_seen, qzs_fix) = seen_and_fix(Constellation::Qzss);

            gps_seen_pts.push([t, gps_seen as f64]);
            gps_fix_pts.push([t, gps_fix as f64]);
            glonass_seen_pts.push([t, gln_seen as f64]);
            glonass_fix_pts.push([t, gln_fix as f64]);
            galileo_seen_pts.push([t, gal_seen as f64]);
            galileo_fix_pts.push([t, gal_fix as f64]);
            beidou_seen_pts.push([t, bei_seen as f64]);
            beidou_fix_pts.push([t, bei_fix as f64]);
            navic_seen_pts.push([t, nav_seen as f64]);
            navic_fix_pts.push([t, nav_fix as f64]);
            qzss_seen_pts.push([t, qzs_seen as f64]);
            qzss_fix_pts.push([t, qzs_fix as f64]);

            for (count, c) in [
                (gps_seen, Constellation::Gps),
                (gln_seen, Constellation::Glonass),
                (gal_seen, Constellation::Galileo),
                (bei_seen, Constellation::Beidou),
                (nav_seen, Constellation::Navic),
                (qzs_seen, Constellation::Qzss),
            ] {
                if count > 0 {
                    present.insert(c);
                }
            }
        }

        if let Some(v) = point.tpv.velocity_kmh() {
            velocity_kmh_pts.push([t, v]);
        }

        if let Some(eph) = point.tpv.eph_m() {
            eph_m_pts.push([t, eph as f64]);
        }

        if let Some(h) = point.tpv.heading() {
            heading_deg_pts.push([t, h.get::<degree>()]);
        }
    }

    let (clock_delta_ms_pts, clock_excursions) =
        clock_delta_series(track, analysis.clock_excursion_threshold_s);

    let x_range = track
        .points
        .first()
        .zip(track.points.last())
        .map(|(first, last)| {
            (
                first.tpv.time().as_secs_f64(),
                last.tpv.time().as_secs_f64(),
            )
        });

    let util = gt_analysis::satellite_utilization::compute_util(
        &track.points,
        analysis.elevation_mask_deg,
    );
    let slip = gt_analysis::loss_of_lock::slip_rate_series(
        &track.points,
        analysis.elevation_mask_deg,
        analysis.snr_drop_db,
        analysis.slip_window_min,
    );

    TrackSeries {
        ti,
        x_range,
        total_seen: MipMap::build(total_seen_pts),
        total_fix: MipMap::build(total_fix_pts),
        gps_seen: MipMap::build(gps_seen_pts),
        gps_fix: MipMap::build(gps_fix_pts),
        glonass_seen: MipMap::build(glonass_seen_pts),
        glonass_fix: MipMap::build(glonass_fix_pts),
        galileo_seen: MipMap::build(galileo_seen_pts),
        galileo_fix: MipMap::build(galileo_fix_pts),
        beidou_seen: MipMap::build(beidou_seen_pts),
        beidou_fix: MipMap::build(beidou_fix_pts),
        navic_seen: MipMap::build(navic_seen_pts),
        navic_fix: MipMap::build(navic_fix_pts),
        qzss_seen: MipMap::build(qzss_seen_pts),
        qzss_fix: MipMap::build(qzss_fix_pts),
        present,
        velocity_kmh: MipMap::build(velocity_kmh_pts),
        eph_m: MipMap::build(eph_m_pts),
        heading_deg: MipMap::build_wrapping(heading_deg_pts, WrapPeriod::full_turn_degrees()),
        clock_delta_ms: MipMap::build(clock_delta_ms_pts),
        clock_excursions,
        util_all: MipMap::build(util.all),
        util_gps: MipMap::build(util.gps),
        util_glonass: MipMap::build(util.glonass),
        util_galileo: MipMap::build(util.galileo),
        util_beidou: MipMap::build(util.beidou),
        util_navic: MipMap::build(util.navic),
        util_qzss: MipMap::build(util.qzss),
        util_anomalies: util.anomalies,
        slip_all: MipMap::build(slip.all),
        slip_gps: MipMap::build(slip.gps),
        slip_glonass: MipMap::build(slip.glonass),
        slip_beidou: MipMap::build(slip.beidou),
        slip_galileo: MipMap::build(slip.galileo),
        slip_navic: MipMap::build(slip.navic),
        slip_qzss: MipMap::build(slip.qzss),
        channels: track.channels.iter().map(build_channel_series).collect(),
    }
}

#[cfg(test)]
mod tests {
    use geotrace_sdk_units::Unit;
    use gt_egui_mipmap::SelectionRange;

    use super::*;

    /// A vector channel becomes one line per component, on the channel's own
    /// sample clock - not resampled onto the nav points.
    #[test]
    #[expect(
        clippy::float_cmp,
        reason = "the values pass through untransformed, so equality is exact"
    )]
    fn a_vector_channel_builds_one_mipmap_per_component() {
        let t =
            |secs: i64| chrono::DateTime::from_timestamp(1_700_000_000 + secs, 0).expect("valid");
        let channel = gt_types::Channel {
            name: "accel".to_owned(),
            unit: Some(Unit::G.into()),
            period: None,
            description: None,
            components: vec!["x".to_owned(), "y".to_owned(), "z".to_owned()],
            times: vec![t(0), t(1)],
            values: vec![0.1, 0.2, 0.3, 0.4, 0.5, 0.6],
        };
        let series = build_channel_series(&channel);
        assert_eq!(series.name, "accel");
        assert_eq!(series.unit.as_deref(), Some("g"));
        let labels: Vec<&str> = series.components.iter().map(|c| c.label.as_str()).collect();
        assert_eq!(labels, ["accel.x", "accel.y", "accel.z"]);
        // Each component line carries its column at the channel's timestamps.
        let full = series.components[1].mipmap.select_indices(
            SelectionRange::within_viewport(f64::NEG_INFINITY..=f64::INFINITY),
            usize::MAX,
        );
        let pts = series.components[1].mipmap.slice_at(full);
        assert_eq!(pts.len(), 2);
        assert_eq!(pts[0].x, 1_700_000_000.0);
        assert_eq!(pts[0].y, 0.2, "y column, row 0");
        assert_eq!(pts[1].y, 0.5, "y column, row 1");
    }

    /// A track at 1 Hz whose clock offset holds near −234 ms, with the sample
    /// at `spike_at` carrying a 1 h 09 m recording gap - the `gnss.h5.gtd`
    /// case, where the receiver reported its pre-gap GPS epoch after resuming.
    fn track_with_a_clock_spike(count: i64, spike_at: i64) -> gt_types::LoadedTrack {
        let points = (0..count)
            .map(|i| {
                let gps = chrono::DateTime::from_timestamp(1_700_000_000 + i, 0).expect("valid");
                let ahead_ms = if i == spike_at { 4_127_054 } else { 234 };
                let tpv = gt_types::tpv::TimePositionVelocity::builder()
                    .time(gt_types::time_types::GpsTime::from_utc(gps))
                    .lat(gt_types::coordinates::Latitude::new(55.0))
                    .lon(gt_types::coordinates::Longitude::new(12.0))
                    .sys_time(gt_types::time_types::SysTime::from_utc(
                        gps + chrono::Duration::milliseconds(ahead_ms),
                    ))
                    .build();
                gt_types::nav_point::NavPoint::new(tpv, None)
            })
            .collect();
        gt_test_utils::loaded_track_with_points(points)
    }

    /// The y-extent of the clock offset line, over every point it carries.
    fn clock_delta_extent(series: &TrackSeries) -> (f64, f64) {
        let full = series.clock_delta_ms.select_indices(
            SelectionRange::within_viewport(f64::NEG_INFINITY..=f64::INFINITY),
            usize::MAX,
        );
        series
            .clock_delta_ms
            .slice_at(full)
            .iter()
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), p| {
                (lo.min(p.y), hi.max(p.y))
            })
    }

    /// The whole point of the excursion split: one sample carrying a recording
    /// gap must not set the auto-bounds of the y-axis every metric shares.
    #[test]
    fn a_clock_spike_stays_off_the_line_and_out_of_its_extent() {
        let track = track_with_a_clock_spike(8, 4);
        let series = build_track_series(0, &track, AnalysisConfig::default());

        let (lo, hi) = clock_delta_extent(&series);
        assert!(
            lo >= -1000.0 && hi <= 0.0,
            "the line keeps the track's own scale, got {lo}..{hi}"
        );
        let [excursion] = series.clock_excursions.as_slice() else {
            panic!("expected one excursion, got {:?}", series.clock_excursions);
        };
        assert_eq!(excursion.peak().offset_ms, -4_127_054, "value is not lost");
    }

    /// Raising the threshold past the departure puts the sample back on the
    /// line: the split is the user's call, not a fixed rule.
    #[test]
    #[expect(
        clippy::float_cmp,
        reason = "the offset passes through untransformed, so equality is exact"
    )]
    fn the_threshold_determines_what_leaves_the_line() {
        let track = track_with_a_clock_spike(8, 4);
        let analysis = AnalysisConfig {
            clock_excursion_threshold_s: 4200.0,
            ..AnalysisConfig::default()
        };
        let series = build_track_series(0, &track, analysis);

        assert!(series.clock_excursions.is_empty());
        let (lo, _) = clock_delta_extent(&series);
        assert_eq!(lo, -4_127_054.0, "the sample is back on the line");
    }

    /// `apply_analysis` re-derives the split, so changing the threshold in
    /// Settings lands without a reload.
    #[test]
    #[expect(
        clippy::float_cmp,
        reason = "the offset passes through untransformed, so equality is exact"
    )]
    fn apply_analysis_re_derives_the_excursion_split() {
        let track = track_with_a_clock_spike(8, 4);
        let mut series = build_track_series(0, &track, AnalysisConfig::default());
        assert_eq!(series.clock_excursions.len(), 1);

        series.apply_analysis(
            &track,
            AnalysisConfig {
                clock_excursion_threshold_s: 4200.0,
                ..AnalysisConfig::default()
            },
        );
        assert!(series.clock_excursions.is_empty());
        assert_eq!(clock_delta_extent(&series).0, -4_127_054.0);
    }

    /// A scalar channel is a single component labelled by the name alone.
    #[test]
    fn a_scalar_channel_builds_one_component() {
        let t =
            |secs: i64| chrono::DateTime::from_timestamp(1_700_000_000 + secs, 0).expect("valid");
        let channel = gt_types::Channel {
            name: "incline".to_owned(),
            unit: Some(Unit::DEG.into()),
            period: None,
            description: None,
            components: vec![],
            times: vec![t(0), t(1)],
            values: vec![1.5, 2.5],
        };
        let series = build_channel_series(&channel);
        assert_eq!(series.components.len(), 1);
        assert_eq!(series.components[0].label, "incline");
    }
}

/// Heading, a quantity that wraps at 360°, from the fix values through the
/// [`MipMap`] levels to what the plot draws.
///
/// Lives in the source file because [`build_track_series`] and
/// [`build_channel_series`] are private to this module.
#[cfg(test)]
mod heading_wrap {
    use geotrace_sdk_units::Unit;
    use gt_egui_mipmap::SelectionRange;
    use gt_types::coordinates::{Latitude, Longitude};
    use gt_types::nav_point::NavPoint;
    use gt_types::time_types::GpsTime;
    use gt_types::tpv::TimePositionVelocity;
    use rstest::rstest;
    use uom::si::f64::Angle;

    use super::*;

    /// Target sample count that selects the first downsampled level of an
    /// eight-fix track.
    const COARSE_TARGET: usize = 4;

    fn at_second(i: i64) -> chrono::DateTime<chrono::Utc> {
        chrono::DateTime::from_timestamp(1_700_000_000 + i, 0).expect("valid timestamp")
    }

    /// A 1 Hz track whose fixes carry `headings`, in degrees. `None` is a ghost
    /// fix: the receiver reported a position but no direction.
    fn track_with_headings(headings: &[Option<f64>]) -> gt_types::LoadedTrack {
        let points = headings
            .iter()
            .enumerate()
            .map(|(i, heading)| {
                let tpv = TimePositionVelocity::builder()
                    .time(GpsTime::from_utc(at_second(i as i64)))
                    .lat(Latitude::new(55.0))
                    .lon(Longitude::new(12.0))
                    .maybe_heading(heading.map(Angle::new::<degree>))
                    .build();
                NavPoint::new(tpv, None)
            })
            .collect();
        gt_test_utils::loaded_track_with_points(points)
    }

    fn heading_series(headings: &[Option<f64>]) -> MipMap {
        let track = track_with_headings(headings);
        build_track_series(0, &track, AnalysisConfig::default()).heading_deg
    }

    /// A scalar channel of `values` in degrees, sampled at 1 Hz, declaring
    /// `period_deg` as its wrap period.
    fn degree_channel(period_deg: Option<f64>, values: &[f64]) -> gt_types::Channel {
        gt_types::Channel {
            name: "compass".to_owned(),
            unit: Some(Unit::DEG.into()),
            period: period_deg.map(Angle::new::<degree>),
            description: None,
            components: vec![],
            times: (0..values.len() as i64).map(at_second).collect(),
            values: values.to_vec(),
        }
    }

    /// The y values the plot draws for one series at the level a view wanting
    /// `target` samples selects, in the order they are drawn.
    fn drawn_values(mipmap: &MipMap, target: usize) -> Vec<f64> {
        let level = mipmap.select_indices(
            SelectionRange::within_viewport(f64::NEG_INFINITY..=f64::INFINITY),
            target,
        );
        mipmap.slice_at(level).iter().map(|p| p.y).collect()
    }

    /// The one component of a scalar channel's series.
    fn scalar_channel_values(channel: &gt_types::Channel, target: usize) -> Vec<f64> {
        let series = build_channel_series(channel);
        let [component] = series.components.as_slice() else {
            panic!("a scalar channel has one component");
        };
        drawn_values(&component.mipmap, target)
    }

    /// Around north a bucket's linear minimum and maximum are ~0° and ~359°,
    /// and the U-turn between them is the outlier the mip-map exists to keep.
    #[test]
    fn a_southward_swing_survives_a_bucket_of_northward_headings() {
        let headings = [359.0, 1.0, 180.0, 2.0, 358.0, 0.0, 359.0, 1.0].map(Some);
        let drawn = drawn_values(&heading_series(&headings), COARSE_TARGET);
        assert!(
            drawn.contains(&180.0),
            "the southward fix must survive downsampling, drawn values are {drawn:?}"
        );
    }

    /// A channel states its own wrap period, which is the period its samples
    /// are downsampled over.
    #[rstest]
    #[case::full_turn(360.0, [359.0, 1.0, 180.0, 2.0, 358.0, 0.0, 359.0, 1.0], 180.0)]
    #[case::half_turn(180.0, [179.0, 1.0, 90.0, 2.0, 178.0, 0.0, 179.0, 1.0], 90.0)]
    fn a_swing_survives_a_bucket_of_a_channel_declaring_a_wrap_period(
        #[case] period_deg: f64,
        #[case] values: [f64; 8],
        #[case] swing: f64,
    ) {
        let channel = degree_channel(Some(period_deg), &values);
        let drawn = scalar_channel_values(&channel, COARSE_TARGET);
        assert!(
            drawn.contains(&swing),
            "the {swing}° sample must survive downsampling, drawn values are {drawn:?}"
        );
    }

    /// A channel that declares no period keeps each bucket's linear minimum and
    /// maximum, however angular its unit reads.
    #[test]
    fn a_degree_channel_without_a_declared_period_is_downsampled_linearly() {
        let channel = degree_channel(None, &[359.0, 1.0, 180.0, 2.0, 358.0, 0.0, 359.0, 1.0]);
        assert_eq!(
            scalar_channel_values(&channel, COARSE_TARGET),
            [359.0, 1.0, 0.0, 359.0]
        );
    }

    /// At full detail the plot draws the headings the receiver reported, in
    /// the values it reported them.
    #[test]
    fn every_recorded_heading_is_drawn_at_full_detail() {
        let headings = [359.0, 1.0, 180.0, 2.0, 358.0, 0.0, 359.0, 1.0];
        assert_eq!(
            drawn_values(&heading_series(&headings.map(Some)), usize::MAX),
            headings
        );
    }

    /// A ghost fix has no heading, which is not a heading of north.
    #[test]
    fn a_ghost_fixs_missing_heading_contributes_no_sample() {
        let series = heading_series(&[Some(10.0), None, None, Some(20.0)]);
        assert_eq!(drawn_values(&series, usize::MAX), [10.0, 20.0]);
    }
}

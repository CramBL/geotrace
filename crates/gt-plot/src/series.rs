use crate::AnalysisConfig;
use gt_analysis::satellite_utilization::UtilAnomaly;
use gt_egui_mipmap::MipMap;
use gt_types::LoadedFile;
use gt_types::satellites::Constellation;
use std::collections::HashSet;
use uom::si::angle::degree;

/// Mipmap series for a single track.
///
/// Built from all points in the track regardless of current visibility or filter.
/// Visibility and time-range clamping are applied at render time in
/// [`super::plot_widget`] so the cache stays valid across filter changes.
#[derive(Debug, Clone)]
pub(crate) struct TrackSeries {
    /// File index within the loaded files list.
    pub fi: usize,
    /// Track index within that file.
    pub ti: usize,
    pub label: String,
    /// Precomputed `(x_min, x_max)` in Unix seconds, or `None` when the track
    /// has no points.  Computed once at build time from the first and last
    /// point timestamps - O(1) field access vs the previous `find_map` over
    /// eight mipmaps.
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
    /// reports.  The plot uses the union across tracks to decide which
    /// per-constellation chips and lines to show, so a constellation with no
    /// data never clutters the UI.
    pub present: HashSet<Constellation>,
    pub velocity_kmh: MipMap,
    pub eph_m: MipMap,
    pub heading_deg: MipMap,
    /// GPS-clock lead over the host system clock, in milliseconds.
    /// Positive = GPS clock ahead, negative = system clock ahead.
    /// Only present when the TPV record carries a system timestamp.
    pub clock_delta_ms: MipMap,
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
}

impl TrackSeries {
    /// Recompute only the analysis-dependent series (utilization rate +
    /// anomalies and slip rate) for `track` under `analysis`, leaving the
    /// analysis-independent mipmaps intact.
    ///
    /// This is the targeted rebuild used when the user changes the elevation
    /// mask, SNR-drop threshold, or slip window, avoiding a full re-derivation
    /// of every metric.  The detection itself lives in `gt_analysis`; this only
    /// wraps the resulting point series in mipmaps.
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
    }
}

/// Build mipmap series for every track in a single file, using `fi` as the file
/// index (for cache keying).
///
/// No visibility check or time filter is applied - that is done at render time
/// so the cache stays valid across filter changes without a rebuild.
pub(crate) fn build_file_series(
    fi: usize,
    file: &LoadedFile,
    analysis: AnalysisConfig,
) -> Vec<TrackSeries> {
    file.tracks
        .iter()
        .enumerate()
        .map(|(ti, track)| build_track_series(fi, ti, file, track, analysis))
        .collect()
}

/// Build mipmap series for every track in every file.
pub(crate) fn build_all_series(files: &[LoadedFile], analysis: AnalysisConfig) -> Vec<TrackSeries> {
    files
        .iter()
        .enumerate()
        .flat_map(|(fi, file)| {
            file.tracks
                .iter()
                .enumerate()
                .map(move |(ti, track)| build_track_series(fi, ti, file, track, analysis))
        })
        .collect()
}

fn build_track_series(
    fi: usize,
    ti: usize,
    file: &LoadedFile,
    track: &gt_types::LoadedTrack,
    analysis: AnalysisConfig,
) -> TrackSeries {
    let label = if file.tracks.len() == 1 {
        file.metadata.filename.clone()
    } else {
        format!("{} T{}", file.metadata.filename, ti + 1)
    };

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
    let mut present: HashSet<Constellation> = HashSet::new();
    let mut velocity_kmh_pts: Vec<[f64; 2]> = Vec::with_capacity(track.points.len());
    let mut eph_m_pts: Vec<[f64; 2]> = Vec::new();
    let mut heading_deg_pts: Vec<[f64; 2]> = Vec::new();
    let mut clock_delta_ms_pts: Vec<[f64; 2]> = Vec::new();

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

        if let Some(sys) = point.tpv.sys_time() {
            let delta_ms = point.tpv.time().offset_from_sys(sys).num_milliseconds();
            clock_delta_ms_pts.push([t, delta_ms as f64]);
        }
    }

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
        fi,
        ti,
        label,
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
        heading_deg: MipMap::build(heading_deg_pts),
        clock_delta_ms: MipMap::build(clock_delta_ms_pts),
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
        slip_galileo: MipMap::build(slip.galileo),
        slip_beidou: MipMap::build(slip.beidou),
        slip_navic: MipMap::build(slip.navic),
        slip_qzss: MipMap::build(slip.qzss),
    }
}

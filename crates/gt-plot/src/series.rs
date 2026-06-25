use crate::AnalysisConfig;
use gt_egui_mipmap::MipMap;
use gt_types::LoadedFile;
use gt_types::satellites::{Constellation, Prn};
use uom::si::angle::degree;

/// Y-position for an anomaly marker when the masked baseline is empty (no
/// in-view satellite above the mask), where the rate is undefined.  The marker
/// lands at the 0 % line - no above-mask satellite was usable, let alone used.
const ANOMALY_FALLBACK_RATE: f64 = 0.0;

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
    /// Epochs where the receiver used a satellite below the elevation mask, so
    /// that satellite is excluded from the utilization rate.  Surfaced as plot
    /// markers; also mask-dependent.
    pub util_anomalies: Vec<UtilAnomaly>,
}

/// A satellite that was used in the fix while sitting below the elevation mask.
#[derive(Debug, Clone)]
pub(crate) struct MaskedSat {
    pub constellation: Constellation,
    pub prn: Prn,
    pub elevation: f32,
}

/// One epoch at which at least one [`MaskedSat`] was used in the fix.
#[derive(Debug, Clone)]
pub(crate) struct UtilAnomaly {
    /// Epoch, in Unix seconds (the marker's x-position).
    pub t: f64,
    /// All-constellations utilization-rate percentage at this epoch, used as the
    /// marker's y-position so it sits on the `UtilAll` line.  Falls back to
    /// [`ANOMALY_FALLBACK_RATE`] when the masked baseline is empty (no in-view
    /// satellite above the mask), which would otherwise leave the rate undefined.
    pub value: f64,
    /// The used sub-mask satellites, sorted by ascending elevation.
    pub masked: Vec<MaskedSat>,
}

/// Per-track utilization point series plus the masked-satellite anomalies, all
/// derived together from one pass over the track at a given elevation mask.
struct UtilPoints {
    all: Vec<[f64; 2]>,
    gps: Vec<[f64; 2]>,
    glonass: Vec<[f64; 2]>,
    galileo: Vec<[f64; 2]>,
    beidou: Vec<[f64; 2]>,
    anomalies: Vec<UtilAnomaly>,
}

/// Utilization rate as a percentage, or `None` when the masked baseline is
/// empty (division undefined - the epoch contributes no line point).
fn rate_percent(num: usize, den: usize) -> Option<f64> {
    (den > 0).then(|| (num as f64 / den as f64) * 100.0)
}

/// Compute the utilization-rate series and masked-satellite anomalies for one
/// track at the given elevation mask.  Shared by [`build_track_series`] and the
/// in-place rebuild in [`TrackSeries::apply_analysis`].
fn compute_util(track: &gt_types::LoadedTrack, mask_deg: f32) -> UtilPoints {
    let mut out = UtilPoints {
        all: Vec::new(),
        gps: Vec::new(),
        glonass: Vec::new(),
        galileo: Vec::new(),
        beidou: Vec::new(),
        anomalies: Vec::new(),
    };

    for point in &track.points {
        let Some(sats) = &point.satellites else {
            continue;
        };
        let t = point.tpv.time().as_secs_f64();

        let all_rate = rate_percent(
            sats.in_fix_above_mask(None, mask_deg),
            sats.in_view_above_mask(None, mask_deg),
        );
        if let Some(r) = all_rate {
            out.all.push([t, r]);
        }

        let rate_for = |c: Constellation| {
            rate_percent(
                sats.in_fix_above_mask(Some(c), mask_deg),
                sats.in_view_above_mask(Some(c), mask_deg),
            )
        };
        if let Some(r) = rate_for(Constellation::Gps) {
            out.gps.push([t, r]);
        }
        if let Some(r) = rate_for(Constellation::Glonass) {
            out.glonass.push([t, r]);
        }
        if let Some(r) = rate_for(Constellation::Galileo) {
            out.galileo.push([t, r]);
        }
        if let Some(r) = rate_for(Constellation::Beidou) {
            out.beidou.push([t, r]);
        }

        let mut masked: Vec<MaskedSat> = sats
            .masked_out_in_fix(mask_deg)
            // `elevation` is guaranteed `Some` by `masked_out_in_fix`'s predicate.
            .filter_map(|s| {
                s.elevation().map(|e| MaskedSat {
                    constellation: s.constellation(),
                    prn: s.prn(),
                    elevation: e,
                })
            })
            .collect();
        if !masked.is_empty() {
            masked.sort_by(|a, b| a.elevation.total_cmp(&b.elevation));
            out.anomalies.push(UtilAnomaly {
                t,
                value: all_rate.unwrap_or(ANOMALY_FALLBACK_RATE),
                masked,
            });
        }
    }

    out
}

impl TrackSeries {
    /// Recompute only the mask-dependent series (utilization rate + anomalies)
    /// for `track` under `analysis`, leaving the mask-independent mipmaps intact.
    ///
    /// This is the targeted rebuild used when the user changes the elevation
    /// mask, avoiding a full re-derivation of every metric.
    pub(crate) fn apply_analysis(
        &mut self,
        track: &gt_types::LoadedTrack,
        analysis: AnalysisConfig,
    ) {
        let u = compute_util(track, analysis.elevation_mask_deg);
        self.util_all = MipMap::build(u.all);
        self.util_gps = MipMap::build(u.gps);
        self.util_glonass = MipMap::build(u.glonass);
        self.util_galileo = MipMap::build(u.galileo);
        self.util_beidou = MipMap::build(u.beidou);
        self.util_anomalies = u.anomalies;
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

            gps_seen_pts.push([t, gps_seen as f64]);
            gps_fix_pts.push([t, gps_fix as f64]);
            glonass_seen_pts.push([t, gln_seen as f64]);
            glonass_fix_pts.push([t, gln_fix as f64]);
            galileo_seen_pts.push([t, gal_seen as f64]);
            galileo_fix_pts.push([t, gal_fix as f64]);
            beidou_seen_pts.push([t, bei_seen as f64]);
            beidou_fix_pts.push([t, bei_fix as f64]);
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

    let util = compute_util(track, analysis.elevation_mask_deg);

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
        velocity_kmh: MipMap::build(velocity_kmh_pts),
        eph_m: MipMap::build(eph_m_pts),
        heading_deg: MipMap::build(heading_deg_pts),
        clock_delta_ms: MipMap::build(clock_delta_ms_pts),
        util_all: MipMap::build(util.all),
        util_gps: MipMap::build(util.gps),
        util_glonass: MipMap::build(util.glonass),
        util_galileo: MipMap::build(util.galileo),
        util_beidou: MipMap::build(util.beidou),
        util_anomalies: util.anomalies,
    }
}

/// Find the index of the point in `points` whose GPS timestamp is closest to
/// `target_secs` (Unix seconds).  Returns `None` if `points` is empty.
pub(crate) fn closest_point_index(
    points: &[gt_types::NavPoint],
    target_secs: f64,
) -> Option<usize> {
    points
        .iter()
        .enumerate()
        .min_by(|(_, a), (_, b)| {
            let da = (a.tpv.time().as_secs_f64() - target_secs).abs();
            let db = (b.tpv.time().as_secs_f64() - target_secs).abs();
            da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|(i, _)| i)
}

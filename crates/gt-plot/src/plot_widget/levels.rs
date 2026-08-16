//! Mipmap level selection: the per-frame sample budget and the cached
//! per-track level choices every metric line draws from.

use gt_egui_mipmap::{LevelSelection, MipMap};
use gt_types::MetricKind;

use crate::series::TrackSeries;

/// Overlap budget expressed as a multiple of the single-track target
/// (`≈ 2 × plot_width_px`).  Tracks that overlap in time each span the full
/// width.  This many of them can do so at full resolution before [`budget_cap`]
/// starts sharing the budget between them.  See [`budget_cap`].
const BUDGET_TRACK_MULTIPLE: usize = 8;

/// Full-resolution sample target for a single track filling the plot width:
/// ~2 samples per pixel, floored so a very narrow plot still has usable detail.
pub(super) fn single_target(available_width: f32) -> usize {
    #[expect(
        clippy::cast_sign_loss,
        reason = "available_width is always ≥ 0 in practice; .max(0.0) makes it explicit"
    )]
    let px = available_width.max(0.0) as usize;
    (px * 2).max(400)
}

/// Upper bound on any single track's sample target.
///
/// Tracks that overlap in time all span the full plot width, so without a cap
/// N of them would each request [`single_target`] points.  Sharing a budget of
/// `single × BUDGET_TRACK_MULTIPLE` across the visible tracks bounds the total
/// handed to egui_plot in that worst case.  Tracks that occupy only part of the
/// width get far less via [`track_target`].  This cap only bites when many tracks
/// pile up in the same time range.
pub(super) fn budget_cap(available_width: f32, visible_count: usize) -> usize {
    let single = single_target(available_width);
    let count = visible_count.max(1);
    (single.saturating_mul(BUDGET_TRACK_MULTIPLE) / count).clamp(2, single)
}

/// Sample target for one track: ~2 points per pixel of the track's *visible*
/// width within the current view, capped by `cap` and floored at 2 (a single
/// segment).
pub(super) fn track_target(
    x_range: Option<(f64, f64)>,
    x_min: f64,
    x_max: f64,
    available_width: f32,
    cap: usize,
) -> usize {
    let view = x_max - x_min;
    let Some((lo, hi)) = x_range else { return 2 };
    if !view.is_finite() || view <= 0.0 {
        return cap;
    }
    let visible = (hi.min(x_max) - lo.max(x_min)).max(0.0);
    let pixels = f64::from(available_width) * (visible / view);
    #[expect(
        clippy::cast_sign_loss,
        clippy::cast_possible_truncation,
        reason = "pixels is finite and ≥ 0; truncating a ~2-per-pixel count to an integer is intended"
    )]
    let want = (2.0 * pixels) as usize;
    want.clamp(2, cap)
}

/// Cached level selections for every metric of one track's series, plus one
/// per channel component (dynamic, hence no `Copy`).
#[derive(Debug, Clone, Default)]
pub(super) struct TrackLevelCache {
    total_seen: LevelSelection,
    total_fix: LevelSelection,
    gps_seen: LevelSelection,
    gps_fix: LevelSelection,
    glonass_seen: LevelSelection,
    glonass_fix: LevelSelection,
    galileo_seen: LevelSelection,
    galileo_fix: LevelSelection,
    beidou_seen: LevelSelection,
    beidou_fix: LevelSelection,
    navic_seen: LevelSelection,
    navic_fix: LevelSelection,
    qzss_seen: LevelSelection,
    qzss_fix: LevelSelection,
    velocity_kmh: LevelSelection,
    eph_m: LevelSelection,
    heading_deg: LevelSelection,
    clock_delta_ms: LevelSelection,
    util_all: LevelSelection,
    util_gps: LevelSelection,
    util_glonass: LevelSelection,
    util_galileo: LevelSelection,
    util_beidou: LevelSelection,
    util_navic: LevelSelection,
    util_qzss: LevelSelection,
    slip_all: LevelSelection,
    slip_gps: LevelSelection,
    slip_glonass: LevelSelection,
    slip_galileo: LevelSelection,
    slip_beidou: LevelSelection,
    slip_navic: LevelSelection,
    slip_qzss: LevelSelection,
    /// One selection per channel component (outer: channel, inner: component).
    pub(super) channels: Vec<Vec<LevelSelection>>,
}

impl TrackLevelCache {
    /// `None` for metrics with no mipmap: snap error draws from the external
    /// per-run series, not from `TrackSeries`.
    pub(super) fn level_for(&self, kind: MetricKind) -> Option<LevelSelection> {
        Some(match kind {
            MetricKind::SatsSeen => self.total_seen,
            MetricKind::SatsFix => self.total_fix,
            MetricKind::GpsSeen => self.gps_seen,
            MetricKind::GpsFix => self.gps_fix,
            MetricKind::GlonassSeen => self.glonass_seen,
            MetricKind::GlonassFix => self.glonass_fix,
            MetricKind::GalileoSeen => self.galileo_seen,
            MetricKind::GalileoFix => self.galileo_fix,
            MetricKind::BeidouSeen => self.beidou_seen,
            MetricKind::BeidouFix => self.beidou_fix,
            MetricKind::NavicSeen => self.navic_seen,
            MetricKind::NavicFix => self.navic_fix,
            MetricKind::QzssSeen => self.qzss_seen,
            MetricKind::QzssFix => self.qzss_fix,
            MetricKind::Velocity => self.velocity_kmh,
            MetricKind::Eph => self.eph_m,
            MetricKind::HeadingDeg => self.heading_deg,
            MetricKind::ClockDeltaMs => self.clock_delta_ms,
            MetricKind::UtilAll => self.util_all,
            MetricKind::UtilGps => self.util_gps,
            MetricKind::UtilGlonass => self.util_glonass,
            MetricKind::UtilGalileo => self.util_galileo,
            MetricKind::UtilBeidou => self.util_beidou,
            MetricKind::UtilNavic => self.util_navic,
            MetricKind::UtilQzss => self.util_qzss,
            MetricKind::SlipAll => self.slip_all,
            MetricKind::SlipGps => self.slip_gps,
            MetricKind::SlipGlonass => self.slip_glonass,
            MetricKind::SlipGalileo => self.slip_galileo,
            MetricKind::SlipBeidou => self.slip_beidou,
            MetricKind::SlipNavic => self.slip_navic,
            MetricKind::SlipQzss => self.slip_qzss,
            MetricKind::SnapError | MetricKind::Jamming => return None,
        })
    }
}

impl crate::series::TrackSeries {
    /// `None` for metrics with no mipmap, matching
    /// [`TrackLevelCache::level_for`].
    pub(super) fn mipmap_for(&self, kind: MetricKind) -> Option<&gt_egui_mipmap::MipMap> {
        Some(match kind {
            MetricKind::SatsSeen => &self.total_seen,
            MetricKind::SatsFix => &self.total_fix,
            MetricKind::GpsSeen => &self.gps_seen,
            MetricKind::GpsFix => &self.gps_fix,
            MetricKind::GlonassSeen => &self.glonass_seen,
            MetricKind::GlonassFix => &self.glonass_fix,
            MetricKind::GalileoSeen => &self.galileo_seen,
            MetricKind::GalileoFix => &self.galileo_fix,
            MetricKind::BeidouSeen => &self.beidou_seen,
            MetricKind::BeidouFix => &self.beidou_fix,
            MetricKind::NavicSeen => &self.navic_seen,
            MetricKind::NavicFix => &self.navic_fix,
            MetricKind::QzssSeen => &self.qzss_seen,
            MetricKind::QzssFix => &self.qzss_fix,
            MetricKind::Velocity => &self.velocity_kmh,
            MetricKind::Eph => &self.eph_m,
            MetricKind::HeadingDeg => &self.heading_deg,
            MetricKind::ClockDeltaMs => &self.clock_delta_ms,
            MetricKind::UtilAll => &self.util_all,
            MetricKind::UtilGps => &self.util_gps,
            MetricKind::UtilGlonass => &self.util_glonass,
            MetricKind::UtilGalileo => &self.util_galileo,
            MetricKind::UtilBeidou => &self.util_beidou,
            MetricKind::UtilNavic => &self.util_navic,
            MetricKind::UtilQzss => &self.util_qzss,
            MetricKind::SlipAll => &self.slip_all,
            MetricKind::SlipGps => &self.slip_gps,
            MetricKind::SlipGlonass => &self.slip_glonass,
            MetricKind::SlipGalileo => &self.slip_galileo,
            MetricKind::SlipBeidou => &self.slip_beidou,
            MetricKind::SlipNavic => &self.slip_navic,
            MetricKind::SlipQzss => &self.slip_qzss,
            MetricKind::SnapError | MetricKind::Jamming => return None,
        })
    }
}
/// Compute fresh level selections for all metrics of one track's series.
///
/// The sample target is derived per track from how many pixels the track
/// occupies in the current view ([`track_target`]), so a track that is only a
/// few pixels wide selects a coarse mipmap level with just a few points.
pub(super) fn compute_level_cache(
    series: &TrackSeries,
    x_min: f64,
    x_max: f64,
    available_width: f32,
    sample_cap: usize,
) -> TrackLevelCache {
    let target = track_target(series.x_range, x_min, x_max, available_width, sample_cap);
    let sel = |mm: &MipMap| mm.select_indices(x_min, x_max, target);
    TrackLevelCache {
        total_seen: sel(&series.total_seen),
        total_fix: sel(&series.total_fix),
        gps_seen: sel(&series.gps_seen),
        gps_fix: sel(&series.gps_fix),
        glonass_seen: sel(&series.glonass_seen),
        glonass_fix: sel(&series.glonass_fix),
        galileo_seen: sel(&series.galileo_seen),
        galileo_fix: sel(&series.galileo_fix),
        beidou_seen: sel(&series.beidou_seen),
        beidou_fix: sel(&series.beidou_fix),
        navic_seen: sel(&series.navic_seen),
        navic_fix: sel(&series.navic_fix),
        qzss_seen: sel(&series.qzss_seen),
        qzss_fix: sel(&series.qzss_fix),
        velocity_kmh: sel(&series.velocity_kmh),
        eph_m: sel(&series.eph_m),
        heading_deg: sel(&series.heading_deg),
        clock_delta_ms: sel(&series.clock_delta_ms),
        util_all: sel(&series.util_all),
        util_gps: sel(&series.util_gps),
        util_glonass: sel(&series.util_glonass),
        util_galileo: sel(&series.util_galileo),
        util_beidou: sel(&series.util_beidou),
        util_navic: sel(&series.util_navic),
        util_qzss: sel(&series.util_qzss),
        slip_all: sel(&series.slip_all),
        slip_gps: sel(&series.slip_gps),
        slip_glonass: sel(&series.slip_glonass),
        slip_galileo: sel(&series.slip_galileo),
        slip_beidou: sel(&series.slip_beidou),
        slip_navic: sel(&series.slip_navic),
        slip_qzss: sel(&series.slip_qzss),
        channels: series
            .channels
            .iter()
            .map(|c| c.components.iter().map(|comp| sel(&comp.mipmap)).collect())
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn track_target_scales_with_visible_pixels() {
        let width = 1000.0;
        let cap = single_target(width);

        // A track spanning the whole view gets ~2 points per pixel (width is
        // 1000 px, so a full-width track should exceed that).
        let full = track_target(Some((0.0, 100.0)), 0.0, 100.0, width, cap);
        assert!(full > 1000, "full-width track should be ~2 pts/pixel");
        assert!(full <= cap);

        // A track occupying ~1% of the view (~10 px) hands over only a handful
        // of points.
        let tiny = track_target(Some((0.0, 1.0)), 0.0, 100.0, width, cap);
        assert!(tiny >= 2);
        assert!(
            tiny <= 32,
            "few-pixel track must hand over few points, got {tiny}"
        );

        // An empty track stays minimal.  A degenerate (zero-width) view never
        // divides by zero and falls back to the cap.
        assert_eq!(track_target(None, 0.0, 100.0, width, cap), 2);
        assert_eq!(track_target(Some((0.0, 1.0)), 5.0, 5.0, width, cap), cap);
    }

    #[test]
    fn budget_cap_bounds_overlapping_tracks() {
        let width = 1000.0;
        let single = single_target(width);
        let budget = single * BUDGET_TRACK_MULTIPLE;

        // Up to BUDGET_TRACK_MULTIPLE overlapping tracks keep full resolution.
        assert_eq!(budget_cap(width, 1), single);
        assert_eq!(budget_cap(width, BUDGET_TRACK_MULTIPLE), single);

        // Beyond that the cap shares the budget so total full-width points stay
        // bounded (allowing the integer-division remainder).
        for count in [BUDGET_TRACK_MULTIPLE + 1, 50, 500] {
            let cap = budget_cap(width, count);
            assert!((2..=single).contains(&cap));
            assert!(
                cap * count <= budget + count,
                "cap {cap} × {count} exceeds budget {budget}"
            );
        }

        // Zero visible count must not divide by zero.
        assert_eq!(budget_cap(width, 0), single);
    }
}

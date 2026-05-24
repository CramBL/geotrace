use chrono::{DateTime, Duration, Utc};
use uom::si::angle::degree;
use uom::si::f64::Angle;

use crate::error::BuildError;
use crate::types::{Annotation, Marker, Meta, NavFile, NavFix, NavPoint, SatelliteReport};

/// Builder for creating a [`NavFile`].
///
/// Add nav fixes, satellite reports, and annotations independently — they do
/// not need to be in any particular order. Call [`finish`](Self::finish) when
/// done; it sorts, associates, and validates all data before returning the
/// ready-to-write [`NavFile`].
pub struct NavFileBuilder {
    fixes: Vec<NavFix>,
    satellite_reports: Vec<SatelliteReport>,
    annotations: Vec<Annotation>,
    meta: Option<Meta>,
    satellite_window: Duration,
    continue_on_error: bool,
}

impl NavFileBuilder {
    /// Create a new builder with default settings.
    ///
    /// Defaults: satellite window = 500 ms, strict mode (errors on dropped data).
    pub fn new() -> Self {
        Self {
            fixes: Vec::new(),
            satellite_reports: Vec::new(),
            annotations: Vec::new(),
            meta: None,
            satellite_window: Duration::milliseconds(500),
            continue_on_error: false,
        }
    }

    /// Attach file-level metadata (title, device, notes).
    pub fn with_meta(mut self, meta: Meta) -> Self {
        self.meta = Some(meta);
        self
    }

    /// Override the maximum time gap for associating a satellite report to a nav fix.
    pub fn with_satellite_window(mut self, window: Duration) -> Self {
        self.satellite_window = window;
        self
    }

    /// Downgrade annotation-out-of-range errors to warnings and continue.
    ///
    /// When `true`, annotations that fall outside the nav fix time range are
    /// clamped to the nearest endpoint and a warning is logged rather than
    /// returning [`BuildError::AnnotationsOutsideRange`].
    pub fn with_continue_on_error(mut self, value: bool) -> Self {
        self.continue_on_error = value;
        self
    }

    pub fn add_nav_fix(&mut self, fix: NavFix) -> &mut Self {
        self.fixes.push(fix);
        self
    }

    pub fn add_satellite_report(&mut self, report: SatelliteReport) -> &mut Self {
        self.satellite_reports.push(report);
        self
    }

    pub fn add_annotation(&mut self, annotation: Annotation) -> &mut Self {
        self.annotations.push(annotation);
        self
    }

    /// Validate and process all added data.
    ///
    /// Steps performed in order:
    /// 1. Discard satellite reports that carry no timestamp (both `gps_time` and
    ///    `sys_time` absent).
    /// 2. Sort fixes, satellite reports, and annotations by time.
    /// 3. Associate each satellite report to its nearest nav fix within the
    ///    configured window.  Reports with `gps_time` are matched directly;
    ///    reports with only `sys_time` use that as the comparison timestamp and
    ///    will typically fall outside the window (becoming orphans).
    ///    Each fix receives at most one report; on equal distance the earlier report wins.
    /// 4. Orphan satellite reports get ghost nav fixes:
    ///    - Between two real fixes: position is interpolated proportionally using
    ///      the corrected GPS timestamp.  The correction applies the GPS/system-clock
    ///      delta derived from the `sys_time` fields of the surrounding NavFixes.
    ///      Falls back to even distribution when no delta information is available.
    ///    - After the last real fix: dead-reckoned 1 m for the first ghost (a
    ///      fix-lost indicator), then 2 m per subsequent ghost; `heading = None`
    ///      so the app renders circles.
    ///    - Before the first real fix: silently dropped (no reference position).
    /// 5. Interpolate each annotation's position from the surrounding fixes.
    /// 6. In strict mode (default), return an error if any annotation falls
    ///    outside the nav fix time range.  In lenient mode it is clamped with a
    ///    warning.
    pub fn finish(mut self) -> Result<NavFile, BuildError> {
        // Drop reports with no usable timestamp before any sorting or association.
        self.satellite_reports.retain(|r| {
            if r.gps_time.is_none() && r.sys_time.is_none() {
                log::warn!("satellite report with no timestamp dropped");
                false
            } else {
                true
            }
        });

        self.fixes.sort_by_key(|f| f.gps_time);
        self.satellite_reports
            .sort_by_key(|r| r.gps_time.or(r.sys_time));
        self.annotations.sort_by_key(|a| a.time);

        if self.fixes.is_empty() && !self.annotations.is_empty() {
            return Err(BuildError::NoNavFixes);
        }

        let (sat_assignments, unassociated) =
            associate_satellites(&self.fixes, self.satellite_reports, &self.satellite_window);

        // Build ghost nav fixes for orphaned satellite reports.
        let ghost_points = ghost_nav_points_for(&self.fixes, unassociated);
        if !ghost_points.is_empty() {
            log::debug!(
                "{} ghost nav fix(es) created for satellite reports outside the {}ms association window",
                ghost_points.len(),
                self.satellite_window.num_milliseconds(),
            );
        }

        let (resolved_markers, out_of_range) =
            interpolate_annotations(&self.fixes, self.annotations, self.continue_on_error);
        let out_of_range_count = out_of_range.len();

        if !self.continue_on_error && out_of_range_count > 0 {
            log::error!(
                "{} annotation(s) fall outside the nav fix time range",
                out_of_range_count
            );
            return Err(BuildError::AnnotationsOutsideRange {
                count: out_of_range_count,
            });
        }
        if out_of_range_count > 0 {
            log::warn!(
                "{} annotation(s) dropped: outside the nav fix time range",
                out_of_range_count
            );
        }

        // Merge real nav points and ghost nav points, sorted by time.
        let mut nav_points: Vec<NavPoint> = self
            .fixes
            .into_iter()
            .zip(sat_assignments)
            .map(|(fix, satellites)| NavPoint { fix, satellites })
            .collect();
        nav_points.extend(ghost_points);
        nav_points.sort_by_key(|p| p.fix.gps_time);

        let markers = resolved_markers
            .into_iter()
            .map(|(annotation, lat_deg, lon_deg)| Marker {
                annotation,
                lat: Angle::new::<degree>(lat_deg),
                lon: Angle::new::<degree>(lon_deg),
            })
            .collect();

        Ok(NavFile {
            meta: self.meta.unwrap_or_default(),
            nav_points,
            markers,
        })
    }
}

impl Default for NavFileBuilder {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Ghost fix creation ──────────────────────────────────────────────────────

/// Create ghost [`NavPoint`]s to carry [`SatelliteReport`]s that fall outside
/// the association window of every real fix.
///
/// Reports are first partitioned into segments by their best-guess GPS position
/// relative to the sorted real fixes, then placed as follows:
///
/// **Between two real fixes** — position interpolated proportionally using the
/// corrected GPS timestamp.  The correction adds the GPS/system-clock delta
/// derived from the `sys_time` fields of the bounding NavFixes; the delta is
/// linearly interpolated between the two anchors.  Reports that already carry
/// `gps_time` are used directly.  When no delta can be computed and no report
/// carries `gps_time`, the builder falls back to even spatial distribution so
/// the output is still usable.  Heading = spherical bearing, fix A to fix B.
///
/// **After the last real fix** — dead-reckoned in the last known heading.  The
/// first ghost is placed 1 m ahead (a fix-lost indicator); subsequent ghosts
/// step 2 m each.  `heading = None` so the app renders circles.
///
/// **Before the first real fix** — silently dropped (no reference position).
fn ghost_nav_points_for(
    real_fixes: &[NavFix],
    orphan_reports: Vec<SatelliteReport>,
) -> Vec<NavPoint> {
    if real_fixes.is_empty() || orphan_reports.is_empty() {
        return Vec::new();
    }

    // ── 1. Build delta anchors ────────────────────────────────────────────────
    //
    // delta_us = gps_us - sys_us at each NavFix that has sys_time.
    // Stored as (gps_us, delta_us) sorted by gps_us (fixes are already sorted).
    let delta_anchors: Vec<(i64, i64)> = real_fixes
        .iter()
        .filter_map(|f| {
            f.sys_time.map(|s| {
                (
                    f.gps_time.timestamp_micros(),
                    f.gps_time.timestamp_micros() - s.timestamp_micros(),
                )
            })
        })
        .collect();

    // ── 2. Partition orphan reports ───────────────────────────────────────────
    //
    // Use best-guess GPS time for each report (corrected if possible) to
    // determine which segment it belongs to.

    let n_segs = real_fixes.len().saturating_sub(1);
    let mut segments: Vec<Vec<SatelliteReport>> = (0..n_segs).map(|_| Vec::new()).collect();
    let mut after_last: Vec<SatelliteReport> = Vec::new();

    for report in orphan_reports {
        let Some(guess_us) = best_guess_gps_us(&report, &delta_anchors) else {
            continue; // both timestamps absent; already filtered in finish()
        };
        let pos = real_fixes.partition_point(|f| f.gps_time.timestamp_micros() < guess_us);

        if pos == 0 {
            // Before first fix: drop.
        } else if pos >= real_fixes.len() {
            after_last.push(report);
        } else if let Some(seg) = segments.get_mut(pos - 1) {
            seg.push(report);
        }
    }

    let mut ghost_points = Vec::new();

    // ── 3. Between-fix segments ───────────────────────────────────────────────

    for (seg_idx, reports) in segments.into_iter().enumerate() {
        if reports.is_empty() {
            continue;
        }
        let Some(b) = real_fixes.get(seg_idx) else {
            continue;
        };
        let Some(a) = real_fixes.get(seg_idx + 1) else {
            continue;
        };

        let b_lat = b.lat.get::<degree>();
        let b_lon = b.lon.get::<degree>();
        let a_lat = a.lat.get::<degree>();
        let a_lon = a.lon.get::<degree>();
        let hdg = ghost_bearing(b_lat, b_lon, a_lat, a_lon);
        let b_gps_us = b.gps_time.timestamp_micros();
        let a_gps_us = a.gps_time.timestamp_micros();
        let span_us = (a_gps_us - b_gps_us) as f64;

        // Per-segment delta anchors.
        let delta_b = b.sys_time.map(|s| b_gps_us - s.timestamp_micros());
        let delta_a = a.sys_time.map(|s| a_gps_us - s.timestamp_micros());

        let can_correct =
            delta_b.is_some() || delta_a.is_some() || reports.iter().any(|r| r.gps_time.is_some());

        if can_correct {
            // Compute the corrected GPS time for each report and sort by it.
            let mut timed: Vec<(i64, SatelliteReport)> = reports
                .into_iter()
                .map(|r| {
                    let ct = segment_corrected_gps_us(&r, b, a, delta_b, delta_a);
                    (ct, r)
                })
                .collect();
            timed.sort_by_key(|(ct, _)| *ct);

            for (corrected_us, report) in timed {
                let frac = if span_us < 1.0 {
                    0.5
                } else {
                    ((corrected_us - b_gps_us) as f64 / span_us).clamp(0.0, 1.0)
                };
                ghost_points.push(NavPoint {
                    fix: NavFix::builder()
                        .gps_time(micros_to_datetime(corrected_us))
                        .lat(Angle::new::<degree>(b_lat + frac * (a_lat - b_lat)))
                        .lon(Angle::new::<degree>(b_lon + frac * (a_lon - b_lon)))
                        .heading(Angle::new::<degree>(hdg))
                        .build(),
                    satellites: Some(report),
                });
            }
        } else {
            // No time correction possible: distribute evenly so output is usable.
            let n = reports.len();
            for (i, report) in reports.into_iter().enumerate() {
                let frac = (i + 1) as f64 / (n + 1) as f64;
                let approx_us = b_gps_us + (span_us * frac) as i64;
                ghost_points.push(NavPoint {
                    fix: NavFix::builder()
                        .gps_time(micros_to_datetime(approx_us))
                        .lat(Angle::new::<degree>(b_lat + frac * (a_lat - b_lat)))
                        .lon(Angle::new::<degree>(b_lon + frac * (a_lon - b_lon)))
                        .heading(Angle::new::<degree>(hdg))
                        .build(),
                    satellites: Some(report),
                });
            }
        }
    }

    // ── 4. After last fix: dead-reckon ────────────────────────────────────────
    //
    // First ghost: 1 m ahead (fix-lost indicator).  Subsequent: 2 m each.

    if !after_last.is_empty() {
        #[expect(
            clippy::expect_used,
            reason = "real_fixes is non-empty (checked at top of fn)"
        )]
        let last = real_fixes.last().expect("real_fixes is non-empty");
        let last_hdg = last.heading.map_or(0.0, |h| h.get::<degree>());
        let last_delta = last
            .sys_time
            .map(|s| last.gps_time.timestamp_micros() - s.timestamp_micros());
        let mut extrap_pos: Option<(f64, f64)> = None;

        for (i, report) in after_last.into_iter().enumerate() {
            let cur = extrap_pos.unwrap_or((last.lat.get::<degree>(), last.lon.get::<degree>()));
            let dist = if i == 0 { 1.0 } else { 2.0 };
            let next = ghost_step(cur.0, cur.1, last_hdg, dist);
            extrap_pos = Some(next);

            let ghost_time_us = dead_reckoned_gps_us(&report, last, last_delta, i);
            ghost_points.push(NavPoint {
                fix: NavFix::builder()
                    .gps_time(micros_to_datetime(ghost_time_us))
                    .lat(Angle::new::<degree>(next.0))
                    .lon(Angle::new::<degree>(next.1))
                    .maybe_heading(None) // None renders as a circle
                    .build(),
                satellites: Some(report),
            });
        }
    }

    ghost_points
}

/// Best-guess GPS timestamp (microseconds) for an orphan report.
///
/// Used for segment partitioning only.  Reports with `gps_time` are exact;
/// reports with only `sys_time` are corrected using the nearest delta anchor.
fn best_guess_gps_us(report: &SatelliteReport, anchors: &[(i64, i64)]) -> Option<i64> {
    if let Some(gt) = report.gps_time {
        return Some(gt.timestamp_micros());
    }
    let st_us = report.sys_time?.timestamp_micros();
    if anchors.is_empty() {
        return Some(st_us);
    }
    // Find the anchor whose sys_time (gps - delta) is closest to st_us.
    let delta = anchors
        .iter()
        .min_by_key(|&&(gps_us, delta_us)| (gps_us - delta_us - st_us).unsigned_abs())
        .map_or(0, |&(_, d)| d);
    Some(st_us + delta)
}

/// Corrected GPS timestamp for an orphan report within a specific segment.
///
/// Delta is linearly interpolated between the two bounding fix anchors using
/// the report's sys_time position within the segment's sys_time range.
/// Returns the GPS time if present, the corrected sys_time if correctable, or
/// sys_time as-is when no delta information is available.
fn segment_corrected_gps_us(
    report: &SatelliteReport,
    b: &NavFix,
    a: &NavFix,
    delta_b: Option<i64>,
    delta_a: Option<i64>,
) -> i64 {
    if let Some(gt) = report.gps_time {
        return gt.timestamp_micros();
    }
    let Some(st) = report.sys_time else {
        // Both timestamps absent; shouldn't reach here after finish() pre-filter.
        return (b.gps_time.timestamp_micros() + a.gps_time.timestamp_micros()) / 2;
    };
    let st_us = st.timestamp_micros();

    match (delta_b, delta_a) {
        (Some(db), Some(da)) => {
            // Interpolate delta by the report's sys_time position in the segment.
            let sys_b = b
                .sys_time
                .map_or(b.gps_time.timestamp_micros() - db, |s| s.timestamp_micros());
            let sys_a = a
                .sys_time
                .map_or(a.gps_time.timestamp_micros() - da, |s| s.timestamp_micros());
            let span = (sys_a - sys_b) as f64;
            let delta = if span < 1.0 {
                (db + da) / 2
            } else {
                let frac = ((st_us - sys_b) as f64 / span).clamp(0.0, 1.0);
                db + ((da - db) as f64 * frac) as i64
            };
            st_us + delta
        }
        (Some(db), None) => st_us + db,
        (None, Some(da)) => st_us + da,
        (None, None) => st_us, // no correction; caller uses even-distribution fallback
    }
}

/// Corrected GPS timestamp for a dead-reckoned report after the last real fix.
///
/// Uses the last fix's delta when available; falls back to sys_time or an
/// index-based estimate if no timestamp is usable.
fn dead_reckoned_gps_us(
    report: &SatelliteReport,
    last: &NavFix,
    last_delta: Option<i64>,
    idx: usize,
) -> i64 {
    if let Some(gt) = report.gps_time {
        return gt.timestamp_micros();
    }
    if let Some(st) = report.sys_time {
        return st.timestamp_micros() + last_delta.unwrap_or(0);
    }
    // No timestamp at all: space out by 1 s from the last fix.
    last.gps_time.timestamp_micros() + (idx as i64 + 1) * 1_000_000
}

// ─── Geometry helpers ────────────────────────────────────────────────────────

/// Spherical forward bearing from A to B in degrees \[0, 360).
fn ghost_bearing(lat0_deg: f64, lon0_deg: f64, lat1_deg: f64, lon1_deg: f64) -> f64 {
    let lat0 = lat0_deg.to_radians();
    let lat1 = lat1_deg.to_radians();
    let dlon = (lon1_deg - lon0_deg).to_radians();
    let y = dlon.sin() * lat1.cos();
    let x = lat0.cos() * lat1.sin() - lat0.sin() * lat1.cos() * dlon.cos();
    (y.atan2(x).to_degrees() + 360.0) % 360.0
}

/// Move `dist_m` metres from (lat, lon) along `heading_deg`; return new (lat, lon).
fn ghost_step(lat_deg: f64, lon_deg: f64, heading_deg: f64, dist_m: f64) -> (f64, f64) {
    const R: f64 = 6_371_000.0;
    let ang = dist_m / R;
    let hdg = heading_deg.to_radians();
    let lat = lat_deg.to_radians();
    let new_lat = (lat.sin() * ang.cos() + lat.cos() * ang.sin() * hdg.cos()).asin();
    let new_lon = lon_deg.to_radians()
        + (hdg.sin() * ang.sin() * lat.cos()).atan2(ang.cos() - lat.sin() * new_lat.sin());
    (new_lat.to_degrees(), new_lon.to_degrees())
}

// ─── Satellite association ───────────────────────────────────────────────────

/// Assign each satellite report to its nearest nav fix within `window`.
///
/// The association timestamp for each report is `gps_time` when present,
/// otherwise `sys_time`.  Reports with `gps_time` will match fix times
/// directly; reports with only `sys_time` typically fall outside the window
/// and become orphans for ghost-fix creation.
///
/// Returns `(assignments, unassociated)` where `assignments[i]` is the
/// satellite report for `fixes[i]` (if any), and `unassociated` contains
/// every report that could not be matched.
///
/// When two reports are equidistant from the same fix, the earlier report
/// (lower index in the sorted slice) wins.
fn associate_satellites(
    fixes: &[NavFix],
    reports: Vec<SatelliteReport>,
    window: &Duration,
) -> (Vec<Option<SatelliteReport>>, Vec<SatelliteReport>) {
    let window_us = window.num_microseconds().unwrap_or(500_000);

    let mut fix_claims: Vec<Option<(i64, usize)>> = vec![None; fixes.len()];
    let mut had_candidate: Vec<bool> = vec![false; reports.len()];

    for (rep_idx, report) in reports.iter().enumerate() {
        let rep_us = report
            .gps_time
            .or(report.sys_time)
            .map_or(i64::MIN, |t| t.timestamp_micros());

        let pos = fixes.partition_point(|f| f.gps_time.timestamp_micros() < rep_us);

        let mut best: Option<(i64, usize)> = None;

        if pos > 0
            && let Some(fix) = fixes.get(pos - 1)
        {
            let dist = (rep_us - fix.gps_time.timestamp_micros()).abs();
            if dist <= window_us {
                if let Some(slot) = had_candidate.get_mut(rep_idx) {
                    *slot = true;
                }
                best = Some((dist, pos - 1));
            }
        }

        if let Some(fix) = fixes.get(pos) {
            let dist = (rep_us - fix.gps_time.timestamp_micros()).abs();
            if dist <= window_us {
                if let Some(slot) = had_candidate.get_mut(rep_idx) {
                    *slot = true;
                }
                match best {
                    None => best = Some((dist, pos)),
                    Some((d, _)) if dist < d => best = Some((dist, pos)),
                    _ => {}
                }
            }
        }

        if let Some((dist, fix_idx)) = best {
            match fix_claims.get_mut(fix_idx) {
                Some(slot @ None) => *slot = Some((dist, rep_idx)),
                Some(Some((existing_dist, existing_rep)))
                    if dist < *existing_dist
                        || (dist == *existing_dist && rep_idx < *existing_rep) =>
                {
                    *existing_dist = dist;
                    *existing_rep = rep_idx;
                }
                _ => {}
            }
        }
    }

    let mut reports_opt: Vec<Option<SatelliteReport>> = reports.into_iter().map(Some).collect();

    let mut assignments: Vec<Option<SatelliteReport>> = vec![None; fixes.len()];
    for (fix_idx, claim) in fix_claims.into_iter().enumerate() {
        if let Some((_, rep_idx)) = claim
            && let Some(slot) = assignments.get_mut(fix_idx)
            && let Some(rep) = reports_opt.get_mut(rep_idx).and_then(|r| r.take())
        {
            *slot = Some(rep);
        }
    }

    let unassociated = reports_opt
        .into_iter()
        .enumerate()
        .filter_map(|(idx, opt)| opt.filter(|_| !had_candidate.get(idx).copied().unwrap_or(false)))
        .collect();
    (assignments, unassociated)
}

// ─── Annotation interpolation ────────────────────────────────────────────────

/// Interpolate positions for each annotation.
///
/// Returns `(resolved, out_of_range)`. In lenient mode the out-of-range vec
/// is always empty (positions are clamped and a warning is logged). In strict
/// mode, out-of-range annotations go into the second vec.
fn interpolate_annotations(
    fixes: &[NavFix],
    annotations: Vec<Annotation>,
    lenient: bool,
) -> (Vec<(Annotation, f64, f64)>, Vec<Annotation>) {
    let mut resolved = Vec::new();
    let mut out_of_range = Vec::new();

    for annotation in annotations {
        let ann_time = annotation.time;

        let pos = fixes.partition_point(|f| f.gps_time <= ann_time);

        let before = if pos > 0 { fixes.get(pos - 1) } else { None };
        let after = fixes.get(pos);

        let position = match (before, after) {
            (Some(b), Some(a)) => {
                let b_us = b.gps_time.timestamp_micros();
                let a_us = a.gps_time.timestamp_micros();
                let ann_us = ann_time.timestamp_micros();
                let t = if a_us == b_us {
                    0.0_f64
                } else {
                    (ann_us - b_us) as f64 / (a_us - b_us) as f64
                };
                let b_lat = b.lat.get::<degree>();
                let b_lon = b.lon.get::<degree>();
                let a_lat = a.lat.get::<degree>();
                let a_lon = a.lon.get::<degree>();
                Some((b_lat + t * (a_lat - b_lat), b_lon + t * (a_lon - b_lon)))
            }
            (Some(b), None) => {
                if lenient {
                    log::warn!(
                        "Annotation at {} is after the last nav fix; clamping to last position",
                        ann_time
                    );
                    Some((b.lat.get::<degree>(), b.lon.get::<degree>()))
                } else {
                    None
                }
            }
            (None, Some(a)) => {
                if lenient {
                    log::warn!(
                        "Annotation at {} is before the first nav fix; clamping to first position",
                        ann_time
                    );
                    Some((a.lat.get::<degree>(), a.lon.get::<degree>()))
                } else {
                    None
                }
            }
            (None, None) => None,
        };

        match position {
            Some((lat, lon)) => resolved.push((annotation, lat, lon)),
            None => out_of_range.push(annotation),
        }
    }

    (resolved, out_of_range)
}

// ─── Time utilities ──────────────────────────────────────────────────────────

pub(crate) fn datetime_to_micros(dt: DateTime<Utc>) -> i64 {
    dt.timestamp_micros()
}

pub(crate) fn micros_to_datetime(us: i64) -> DateTime<Utc> {
    DateTime::from_timestamp_micros(us).unwrap_or_default()
}

/// Encode an optional `DateTime<Utc>` as u64 microseconds since Unix epoch.
///
/// `u64::MAX` is used as the sentinel for `None`; it corresponds to year ~584,542
/// which is impossible for real data.  All valid GPS/system timestamps are
/// positive i64 values that fit safely in u64.
pub(crate) fn opt_datetime_to_u64(dt: Option<DateTime<Utc>>) -> u64 {
    dt.map_or(u64::MAX, |t| t.timestamp_micros().cast_unsigned())
}

/// Decode a u64 microsecond value back to `Option<DateTime<Utc>>`, treating `u64::MAX` as absent.
pub(crate) fn u64_to_opt_datetime(v: u64) -> Option<DateTime<Utc>> {
    if v == u64::MAX {
        None
    } else {
        Some(micros_to_datetime(v as i64))
    }
}

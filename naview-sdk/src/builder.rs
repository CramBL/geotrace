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

    /// Drop unassociable data with a warning instead of returning an error.
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
    /// 1. Sort fixes, satellite reports, and annotations by time.
    /// 2. Associate each satellite report to its nearest nav fix within the
    ///    configured window. Each fix receives at most one report; on equal
    ///    distance, the earlier report wins.
    /// 3. Interpolate each annotation's position from the surrounding fixes.
    /// 4. In strict mode (default), return an error if any data was dropped.
    ///    In lenient mode, dropped data is logged at `warn` level.
    pub fn finish(mut self) -> Result<NavFile, BuildError> {
        self.fixes.sort_by_key(|f| f.time);
        self.satellite_reports.sort_by_key(|r| r.time);
        self.annotations.sort_by_key(|a| a.time);

        if self.fixes.is_empty() && !self.annotations.is_empty() {
            return Err(BuildError::NoNavFixes);
        }

        let window_ms = self.satellite_window.num_milliseconds();

        let (sat_assignments, unassociated) =
            associate_satellites(&self.fixes, self.satellite_reports, &self.satellite_window);
        let unassoc_count = unassociated.len();

        let (resolved_markers, out_of_range) =
            interpolate_annotations(&self.fixes, self.annotations, self.continue_on_error);
        let out_of_range_count = out_of_range.len();

        if !self.continue_on_error && (unassoc_count > 0 || out_of_range_count > 0) {
            if unassoc_count > 0 {
                log::error!(
                    "{} satellite report(s) could not be associated within the {}ms window",
                    unassoc_count,
                    window_ms
                );
            }
            if out_of_range_count > 0 {
                log::error!(
                    "{} annotation(s) fall outside the nav fix time range",
                    out_of_range_count
                );
            }
            return Err(match (unassoc_count, out_of_range_count) {
                (n, 0) => BuildError::UnassociatedSatelliteReports {
                    count: n,
                    window_ms,
                },
                (0, n) => BuildError::AnnotationsOutsideRange { count: n },
                (n, m) => BuildError::Multiple {
                    unassociated_satellite_reports: n,
                    annotations_outside_range: m,
                    window_ms,
                },
            });
        }

        if unassoc_count > 0 {
            log::warn!(
                "{} satellite report(s) dropped: no nav fix within {}ms",
                unassoc_count,
                window_ms
            );
        }
        if out_of_range_count > 0 {
            log::warn!(
                "{} annotation(s) dropped: outside the nav fix time range",
                out_of_range_count
            );
        }

        let nav_points = self
            .fixes
            .into_iter()
            .zip(sat_assignments)
            .map(|(fix, satellites)| NavPoint { fix, satellites })
            .collect();

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

/// Assign each satellite report to its nearest nav fix within `window`.
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

    // fix_claims[fix_idx] = (distance_us, report_idx) — best claim for this fix
    let mut fix_claims: Vec<Option<(i64, usize)>> = vec![None; fixes.len()];
    // had_candidate[rep_idx] — true if this report had at least one fix within the window.
    // Reports that had a candidate but lost to a competitor are silently dropped (not errors).
    let mut had_candidate: Vec<bool> = vec![false; reports.len()];

    for (rep_idx, report) in reports.iter().enumerate() {
        let rep_us = report.time.timestamp_micros();

        // Binary search: first fix with time >= report.time
        let pos = fixes.partition_point(|f| f.time.timestamp_micros() < rep_us);

        let mut best: Option<(i64, usize)> = None;

        // Check fix immediately before pos
        if pos > 0
            && let Some(fix) = fixes.get(pos - 1)
        {
            let dist = (rep_us - fix.time.timestamp_micros()).abs();
            if dist <= window_us {
                if let Some(slot) = had_candidate.get_mut(rep_idx) {
                    *slot = true;
                }
                best = Some((dist, pos - 1));
            }
        }

        // Check fix at pos; prefer whichever is closer
        if let Some(fix) = fixes.get(pos) {
            let dist = (rep_us - fix.time.timestamp_micros()).abs();
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

    // Materialise: take each winning report out of the vec
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

    // Only reports that had no valid candidate within the window are truly unassociated.
    // Reports that competed but lost are silently dropped and do not trigger an error.
    let unassociated = reports_opt
        .into_iter()
        .enumerate()
        .filter_map(|(idx, opt)| opt.filter(|_| !had_candidate.get(idx).copied().unwrap_or(false)))
        .collect();
    (assignments, unassociated)
}

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

        // partition_point with <= gives: pos = first index where fix.time > ann_time
        let pos = fixes.partition_point(|f| f.time <= ann_time);

        let before = if pos > 0 { fixes.get(pos - 1) } else { None };
        let after = fixes.get(pos);

        let position = match (before, after) {
            (Some(b), Some(a)) => {
                let b_us = b.time.timestamp_micros();
                let a_us = a.time.timestamp_micros();
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
            (None, None) => {
                // fixes is empty — checked before calling this function
                None
            }
        };

        match position {
            Some((lat, lon)) => resolved.push((annotation, lat, lon)),
            None => out_of_range.push(annotation),
        }
    }

    (resolved, out_of_range)
}

pub(crate) fn datetime_to_micros(dt: DateTime<Utc>) -> i64 {
    dt.timestamp_micros()
}

pub(crate) fn micros_to_datetime(us: i64) -> DateTime<Utc> {
    DateTime::from_timestamp_micros(us).unwrap_or_default()
}

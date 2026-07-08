use crate::{Angle, Velocity};
use chrono::{DateTime, Duration, Utc};

use crate::error::BuildError;
use crate::time_types::{GpsTime, SysTime};
use crate::types::{
    Annotation, Channel, Constellation, EventMarker, EventMarkerColor, EventMarkerIconChoice,
    EventMarkerPoint, EventMarkerStyle, Marker, Meta, NavFile, NavFix, NavPoint, Satellite,
    SatelliteReport,
};
use crate::variant_path::EventKind;

// User-supplied `NavFix` and `SatelliteReport` structs carry `DateTime<Utc>` for
// public-API simplicity.  Immediately on intake the builder converts them into
// these internal types so that all internal processing distinguishes GPS-receiver
// time from host system-clock time at the type level.
//
// The final output (`NavPoint` / `NavFile`) converts back to the public types with
// plain `DateTime<Utc>` fields so downstream callers (`gt-io`, external readers)
// see no change.
/// Builder-internal representation of a nav fix with typed clock domains.
struct InternalFix {
    gps_time: Option<GpsTime>,
    sys_time: Option<SysTime>,
    lat: Angle,
    lon: Angle,
    heading: Option<Angle>,
    speed: Option<Velocity>,
    eph_m: Option<f64>,
}

impl InternalFix {
    fn from_nav_fix(f: NavFix) -> Self {
        Self {
            gps_time: f.gps_time.map(GpsTime::from_utc),
            sys_time: f.sys_time.map(SysTime::from_utc),
            lat: f.lat,
            lon: f.lon,
            heading: f.heading,
            speed: f.speed,
            eph_m: f.eph_m,
        }
    }

    /// Build a ghost fix whose position and time are fully computed.
    fn ghost(gps_time: GpsTime, lat: Angle, lon: Angle, heading: Option<Angle>) -> Self {
        Self {
            gps_time: Some(gps_time),
            sys_time: None,
            lat,
            lon,
            heading,
            speed: None,
            eph_m: None,
        }
    }

    /// Convert back to the public `NavFix` for the final output `NavPoint`.
    fn into_nav_fix(self) -> NavFix {
        NavFix {
            gps_time: self.gps_time.map(GpsTime::utc),
            sys_time: self.sys_time.map(SysTime::utc),
            lat: self.lat,
            lon: self.lon,
            heading: self.heading,
            speed: self.speed,
            eph_m: self.eph_m,
        }
    }
}

/// Builder-internal representation of a satellite report with typed clock domains.
struct InternalSatReport {
    gps_time: Option<GpsTime>,
    sys_time: Option<SysTime>,
    tracked: Vec<Satellite>,
}

impl InternalSatReport {
    fn from_sat_report(r: SatelliteReport) -> Self {
        Self {
            gps_time: r.gps_time.map(GpsTime::from_utc),
            sys_time: r.sys_time.map(SysTime::from_utc),
            tracked: r.tracked,
        }
    }

    fn into_sat_report(self) -> SatelliteReport {
        SatelliteReport {
            gps_time: self.gps_time.map(GpsTime::utc),
            sys_time: self.sys_time.map(SysTime::utc),
            tracked: self.tracked,
        }
    }
}

/// Builder-internal nav point.
struct InternalPoint {
    fix: InternalFix,
    satellites: Option<InternalSatReport>,
}

impl InternalPoint {
    fn into_nav_point(self) -> NavPoint {
        NavPoint {
            fix: self.fix.into_nav_fix(),
            satellites: self.satellites.map(InternalSatReport::into_sat_report),
        }
    }
}

/// Configuration builder for creating a [`NavRecorder`].
///
/// Set global options with the fluent `with_*` methods, then call
/// [`open`](Self::open) to obtain a [`NavRecorder`] for data ingestion.
///
/// ```no_run
/// use geotrace_sdk::{NavFileBuilder, Meta};
///
/// # fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let mut recorder = NavFileBuilder::new()
///     .with_meta(Meta::builder().title("My Track").build())
///     .open();
///
/// // Add data to `recorder`, then call `recorder.finish()`.
/// # Ok(())
/// # }
/// ```
pub struct NavFileBuilder {
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

    /// Set the file title.
    pub fn with_title(mut self, title: impl Into<String>) -> Self {
        let m = self.meta.get_or_insert_with(Meta::default);
        m.title = Some(title.into());
        self
    }

    /// Set the device or sensor name.
    pub fn with_device(mut self, device: impl Into<String>) -> Self {
        let m = self.meta.get_or_insert_with(Meta::default);
        m.device = Some(device.into());
        self
    }

    /// Set free-text notes for the file.
    pub fn with_notes(mut self, notes: impl Into<String>) -> Self {
        let m = self.meta.get_or_insert_with(Meta::default);
        m.notes = Some(notes.into());
        self
    }

    /// Set the stable identity key used by the app's history database.
    ///
    /// All recordings with the same identity are stored under the same group
    /// and appear together in the History window. The string should be stable
    /// across re-recordings - for example a device serial number or route name.
    pub fn with_identity(mut self, id: impl Into<String>) -> Self {
        let m = self.meta.get_or_insert_with(Meta::default);
        m.identity = Some(id.into());
        self
    }

    /// Override the maximum time gap for associating a satellite report to a nav fix.
    pub fn with_satellite_window(mut self, window: Duration) -> Self {
        self.satellite_window = window;
        self
    }

    /// Downgrade annotation-out-of-range errors to warnings and continue.
    ///
    /// Annotations that fall outside the nav fix time range are clamped to the
    /// nearest endpoint and a warning is logged rather than returning
    /// [`BuildError::AnnotationsOutsideRange`].
    pub fn with_lenient_errors(mut self) -> Self {
        self.continue_on_error = true;
        self
    }

    /// Consume the configuration and return a [`NavRecorder`] ready for data.
    pub fn open(self) -> NavRecorder {
        NavRecorder {
            fixes: Vec::new(),
            satellite_reports: Vec::new(),
            annotations: Vec::new(),
            pending_event_markers: Vec::new(),
            event_marker_styles: Vec::new(),
            channels: Vec::new(),
            styled_paths: std::collections::HashSet::new(),
            meta: self.meta,
            satellite_window: self.satellite_window,
            continue_on_error: self.continue_on_error,
        }
    }
}

impl Default for NavFileBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Data recorder for collecting nav fixes, satellite reports, annotations, and event markers.
///
/// Obtain via [`NavFileBuilder::open`]. Call [`finish`](Self::finish) when all
/// data has been added to validate and produce a [`NavFile`].
pub struct NavRecorder {
    fixes: Vec<InternalFix>,
    satellite_reports: Vec<InternalSatReport>,
    annotations: Vec<Annotation>,
    pending_event_markers: Vec<(String, chrono::DateTime<chrono::Utc>, Option<String>)>,
    event_marker_styles: Vec<EventMarkerStyle>,
    channels: Vec<Channel>,
    styled_paths: std::collections::HashSet<String>,
    meta: Option<Meta>,
    satellite_window: Duration,
    continue_on_error: bool,
}

/// A timeline object that [`NavRecorder::add`] dispatches on.
///
/// Implemented for [`NavFix`], [`SatelliteReport`], [`Annotation`],
/// [`EventMarker`], and [`Channel`]. Per-variant styling and typed events are
/// intentionally not `NavRecord`s: use [`add_event_marker_style`] and
/// [`add_event`] respectively.
///
/// [`add_event_marker_style`]: NavRecorder::add_event_marker_style
/// [`add_event`]: NavRecorder::add_event
pub trait NavRecord {
    /// Append `self` to `recorder` via its matching typed `add_*` method.
    fn add_to(self, recorder: &mut NavRecorder);
}

impl NavRecord for NavFix {
    fn add_to(self, recorder: &mut NavRecorder) {
        recorder.add_nav_fix(self);
    }
}

impl NavRecord for SatelliteReport {
    fn add_to(self, recorder: &mut NavRecorder) {
        recorder.add_satellite_report(self);
    }
}

impl NavRecord for Annotation {
    fn add_to(self, recorder: &mut NavRecorder) {
        recorder.add_annotation(self);
    }
}

impl NavRecord for EventMarker {
    fn add_to(self, recorder: &mut NavRecorder) {
        recorder.add_event_marker(self);
    }
}

impl NavRecord for Channel {
    fn add_to(self, recorder: &mut NavRecorder) {
        recorder.add_channel(self);
    }
}

impl NavRecorder {
    /// Add any timeline object (a [`NavFix`], [`SatelliteReport`],
    /// [`Annotation`], [`EventMarker`], or [`Channel`]), dispatched by type
    /// via [`NavRecord`].
    ///
    /// Ergonomic sugar for the matching `add_*` method:
    ///
    /// ```ignore
    /// recorder.add(fix).add(report).add(annotation);
    /// ```
    pub fn add(&mut self, item: impl NavRecord) -> &mut Self {
        item.add_to(self);
        self
    }

    /// Accept a nav fix.
    ///
    /// Timestamps are immediately wrapped into typed clock-domain values so all internal
    /// builder processing is clock-domain safe.
    pub fn add_nav_fix(&mut self, fix: impl Into<NavFix>) -> &mut Self {
        self.fixes.push(InternalFix::from_nav_fix(fix.into()));
        self
    }

    /// Accept a satellite report.
    ///
    /// As with nav fixes, timestamps are immediately wrapped into typed values.
    pub fn add_satellite_report(&mut self, report: impl Into<SatelliteReport>) -> &mut Self {
        self.satellite_reports
            .push(InternalSatReport::from_sat_report(report.into()));
        self
    }

    pub fn add_annotation(&mut self, annotation: impl Into<Annotation>) -> &mut Self {
        self.annotations.push(annotation.into());
        self
    }

    /// Add a validated event marker.
    ///
    /// The variant path was already validated by `EventMarker::builder().build()`,
    /// so this method is infallible.
    pub fn add_event_marker(&mut self, marker: impl Into<EventMarker>) -> &mut Self {
        let marker = marker.into();
        self.pending_event_markers
            .push((marker.variant_path, marker.sys_time, marker.annotation));
        self
    }

    /// Attach a scalar sensor [`Channel`] to be recorded alongside the track.
    ///
    /// The channel keeps its own sample timestamps; it is correlated with the
    /// nav points by time at query time, not resampled here. Build one with
    /// [`Channel::builder`].
    pub fn add_channel(&mut self, channel: Channel) -> &mut Self {
        self.channels.push(channel);
        self
    }

    /// Add a typed event marker derived from an [`EventKind`] implementation.
    ///
    /// If `event.variant_path()` returns `None` (e.g. a `#[event_kind(skip)]`
    /// variant), the call is a silent no-op.
    pub fn add_event(
        &mut self,
        event: &impl EventKind,
        sys_time: impl Into<chrono::DateTime<chrono::Utc>>,
    ) -> &mut Self {
        let Some(path) = event.variant_path() else {
            return self;
        };
        self.register_icon_for_path(&path, event);
        self.pending_event_markers
            .push((path, sys_time.into(), event.event_note()));
        self
    }

    /// Add a typed event marker with a free-text note.
    ///
    /// Identical to [`add_event`](Self::add_event) but attaches `note` as the
    /// marker's annotation string, visible in the app alongside the marker icon.
    pub fn add_event_with_note(
        &mut self,
        event: &impl EventKind,
        sys_time: impl Into<chrono::DateTime<chrono::Utc>>,
        note: impl Into<String>,
    ) -> &mut Self {
        let Some(path) = event.variant_path() else {
            return self;
        };
        self.register_icon_for_path(&path, event);
        self.pending_event_markers
            .push((path, sys_time.into(), Some(note.into())));
        self
    }

    fn register_icon_for_path(&mut self, path: &str, event: &impl EventKind) {
        if let Some(icon) = event.marker_icon()
            && self.styled_paths.insert(path.to_owned())
        {
            self.event_marker_styles.push(EventMarkerStyle {
                variant_path: path.to_owned(),
                icon: EventMarkerIconChoice::Icon(icon),
                color: EventMarkerColor::Auto,
            });
        }
    }

    /// Register an icon and color for a variant path.
    ///
    /// Variants with no registered style use the fallback hash color and Pin icon.
    /// When called before `add_event`, the manual style takes precedence over any
    /// icon declared via `#[event_kind(icon = …)]`.
    pub fn add_event_marker_style(&mut self, style: EventMarkerStyle) -> &mut Self {
        self.styled_paths.insert(style.variant_path.clone());
        self.event_marker_styles.push(style);
        self
    }

    /// Validate and process all added data.
    ///
    /// Steps performed in order:
    /// 1. Discard satellite reports that carry no timestamp (both `gps_time` and
    ///    `sys_time` absent).
    /// 2. Sort fixes, satellite reports, and annotations by time.
    /// 3. Associate each satellite report to its nearest nav fix within the
    ///    configured window.  Reports with `gps_time` are matched directly.
    ///    Reports with only `sys_time` are first corrected into the GPS time domain
    ///    using the GPS/sys-clock delta derived from fixes that have both timestamps
    ///    (`delta = gps_time − sys_time`). The nearest-anchor delta is applied
    ///    before the window comparison.
    ///    This handles the `--no-filter` pipeline where SAT records carry only
    ///    `sys_time` but can be reliably placed once the clock offset is known from
    ///    the surrounding fixes.
    ///    When no delta can be computed (no fix has both timestamps), raw `sys_time`
    ///    is used as a fallback - same behaviour as before.
    ///    Each fix receives at most one report. On equal distance the earlier report wins.
    /// 4. Orphan satellite reports get ghost nav fixes:
    ///    - Between two real fixes: position is interpolated proportionally using
    ///      the corrected GPS timestamp.  The correction applies the GPS/system-clock
    ///      delta derived from the `sys_time` fields of the surrounding NavFixes.
    ///      Falls back to even distribution when no delta information is available.
    ///    - After the last real fix: dead-reckoned 1 m for the first ghost (a
    ///      fix-lost indicator), then 2 m per subsequent ghost. `heading = None`
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

        validate_satellite_data(&self.satellite_reports);

        self.fixes.sort_by_key(effective_time);
        self.satellite_reports.sort_by_key(|r| {
            r.gps_time
                .map(GpsTime::utc)
                .or_else(|| r.sys_time.map(SysTime::utc))
        });
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
        let mut internal_points: Vec<InternalPoint> = self
            .fixes
            .into_iter()
            .zip(sat_assignments)
            .map(|(fix, satellites)| InternalPoint { fix, satellites })
            .collect();
        internal_points.extend(ghost_points);
        internal_points.sort_by_key(|p| effective_time(&p.fix));

        let event_markers = interpolate_event_markers(&internal_points, self.pending_event_markers);

        // Convert to public output types at the output boundary.
        let nav_points: Vec<NavPoint> = internal_points
            .into_iter()
            .map(InternalPoint::into_nav_point)
            .collect();

        let markers = resolved_markers
            .into_iter()
            .map(|(annotation, lat_deg, lon_deg)| Marker {
                annotation,
                lat: Angle::degrees(lat_deg),
                lon: Angle::degrees(lon_deg),
            })
            .collect();

        // Channels are keyed by name, so store them in a canonical name order
        // independent of the order they were added. Names become HDF5 group
        // names, so a duplicate would silently collide - reject it loudly.
        let mut channels = self.channels;
        channels.sort_by(|a, b| a.name.cmp(&b.name));
        if let Some(name) = channels.windows(2).find_map(|pair| match pair {
            [a, b] if a.name == b.name => Some(a.name.clone()),
            _ => None,
        }) {
            return Err(BuildError::DuplicateChannelName { name });
        }

        Ok(NavFile {
            meta: self.meta.unwrap_or_default(),
            nav_points,
            markers,
            event_markers,
            event_marker_styles: self.event_marker_styles,
            channels,
        })
    }
}

/// Create ghost [`InternalPoint`]s to carry [`InternalSatReport`]s that fall outside
/// the association window of every real fix.
///
/// Reports are first partitioned into segments by their best-guess GPS position
/// relative to the sorted real fixes, then placed as follows:
///
/// **Between two real fixes** - position interpolated proportionally using the
/// corrected GPS timestamp.  The correction adds the GPS/system-clock delta
/// derived from the `sys_time` fields of the bounding NavFixes. The delta is
/// linearly interpolated between the two anchors.  Reports that already carry
/// `gps_time` are used directly.  When no delta can be computed and no report
/// carries `gps_time`, the builder falls back to even spatial distribution so
/// the output is still usable.  Heading = spherical bearing, fix A to fix B.
///
/// **After the last real fix** - dead-reckoned in the last known heading.  The
/// first ghost is placed 1 m ahead (a fix-lost indicator). Subsequent ghosts
/// step 2 m each.  `heading = None` so the app renders circles.
///
/// **Before the first real fix** - silently dropped (no reference position).
fn ghost_nav_points_for(
    real_fixes: &[InternalFix],
    orphan_reports: Vec<InternalSatReport>,
) -> Vec<InternalPoint> {
    if real_fixes.is_empty() || orphan_reports.is_empty() {
        return Vec::new();
    }

    //
    // delta_us = gps_us - sys_us at each fix that has both a genuine GPS lock
    // and a sys_time.  Stored as (gps_us, delta_us) sorted by gps_us.
    let delta_anchors: Vec<(i64, i64)> = real_fixes
        .iter()
        .filter_map(|f| match (f.gps_time, f.sys_time) {
            (Some(gt), Some(st)) => Some((
                gt.timestamp_micros(),
                gt.timestamp_micros() - st.timestamp_micros(),
            )),
            _ => None,
        })
        .collect();

    let n_segs = real_fixes.len().saturating_sub(1);
    let mut segments: Vec<Vec<InternalSatReport>> = (0..n_segs).map(|_| Vec::new()).collect();
    let mut after_last: Vec<InternalSatReport> = Vec::new();

    for report in orphan_reports {
        let Some(guess_us) = best_guess_gps_us(&report, &delta_anchors) else {
            continue;
        };
        let pos = real_fixes.partition_point(|f| effective_time(f).timestamp_micros() < guess_us);

        if pos == 0 {
            // Before first fix: drop.
        } else if pos >= real_fixes.len() {
            after_last.push(report);
        } else if let Some(seg) = segments.get_mut(pos - 1) {
            seg.push(report);
        }
    }

    let mut ghost_points = Vec::new();

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

        let b_lat = b.lat.as_degrees();
        let b_lon = b.lon.as_degrees();
        let a_lat = a.lat.as_degrees();
        let a_lon = a.lon.as_degrees();
        let hdg = ghost_bearing(b_lat, b_lon, a_lat, a_lon);
        let b_gps_us = effective_time(b).timestamp_micros();
        let a_gps_us = effective_time(a).timestamp_micros();
        let span_us = (a_gps_us - b_gps_us) as f64;

        // Per-segment delta anchors - only defined when the fix has a genuine GPS lock.
        let delta_b = b.gps_time.and_then(|gt| {
            b.sys_time
                .map(|st| gt.timestamp_micros() - st.timestamp_micros())
        });
        let delta_a = a.gps_time.and_then(|gt| {
            a.sys_time
                .map(|st| gt.timestamp_micros() - st.timestamp_micros())
        });

        let can_correct =
            delta_b.is_some() || delta_a.is_some() || reports.iter().any(|r| r.gps_time.is_some());

        if can_correct {
            let mut timed: Vec<(i64, InternalSatReport)> = reports
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
                ghost_points.push(InternalPoint {
                    fix: InternalFix::ghost(
                        GpsTime::from_utc(micros_to_datetime(corrected_us)),
                        Angle::degrees(b_lat + frac * (a_lat - b_lat)),
                        Angle::degrees(b_lon + frac * (a_lon - b_lon)),
                        Some(Angle::degrees(hdg)),
                    ),
                    satellites: Some(report),
                });
            }
        } else {
            // No time correction possible: distribute evenly.
            let n = reports.len();
            for (i, report) in reports.into_iter().enumerate() {
                let frac = (i + 1) as f64 / (n + 1) as f64;
                let approx_us = b_gps_us + (span_us * frac) as i64;
                ghost_points.push(InternalPoint {
                    fix: InternalFix::ghost(
                        GpsTime::from_utc(micros_to_datetime(approx_us)),
                        Angle::degrees(b_lat + frac * (a_lat - b_lat)),
                        Angle::degrees(b_lon + frac * (a_lon - b_lon)),
                        Some(Angle::degrees(hdg)),
                    ),
                    satellites: Some(report),
                });
            }
        }
    }

    if !after_last.is_empty() {
        #[expect(
            clippy::expect_used,
            reason = "real_fixes is non-empty (checked at top of fn)"
        )]
        let last = real_fixes.last().expect("real_fixes is non-empty");
        let last_hdg = last.heading.map_or(0.0, |h| h.as_degrees());
        let last_delta = last.gps_time.and_then(|gt| {
            last.sys_time
                .map(|st| gt.timestamp_micros() - st.timestamp_micros())
        });
        let mut extrap_pos: Option<(f64, f64)> = None;

        for (i, report) in after_last.into_iter().enumerate() {
            let cur = extrap_pos.unwrap_or((last.lat.as_degrees(), last.lon.as_degrees()));
            let dist = if i == 0 { 1.0 } else { 2.0 };
            let next = ghost_step(cur.0, cur.1, last_hdg, dist);
            extrap_pos = Some(next);

            let ghost_time_us = dead_reckoned_gps_us(&report, last, last_delta, i);
            ghost_points.push(InternalPoint {
                fix: InternalFix::ghost(
                    GpsTime::from_utc(micros_to_datetime(ghost_time_us)),
                    Angle::degrees(next.0),
                    Angle::degrees(next.1),
                    None, // None renders as a circle
                ),
                satellites: Some(report),
            });
        }
    }

    ghost_points
}

/// Best-guess GPS timestamp (microseconds) for an orphan report.
///
/// Used for segment partitioning only.  Reports with `gps_time` are exact.
/// Reports with only `sys_time` are corrected using the nearest delta anchor.
fn best_guess_gps_us(report: &InternalSatReport, anchors: &[(i64, i64)]) -> Option<i64> {
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
    report: &InternalSatReport,
    b: &InternalFix,
    a: &InternalFix,
    delta_b: Option<i64>,
    delta_a: Option<i64>,
) -> i64 {
    if let Some(gt) = report.gps_time {
        return gt.timestamp_micros();
    }
    let Some(st) = report.sys_time else {
        // Both timestamps absent. Shouldn't reach here after finish() pre-filter.
        return (effective_time(b).timestamp_micros() + effective_time(a).timestamp_micros()) / 2;
    };
    let st_us = st.timestamp_micros();

    match (delta_b, delta_a) {
        (Some(db), Some(da)) => {
            // Interpolate delta by the report's sys_time position in the segment.
            let sys_b = b
                .sys_time
                .map_or(effective_time(b).timestamp_micros() - db, |s| {
                    s.timestamp_micros()
                });
            let sys_a = a
                .sys_time
                .map_or(effective_time(a).timestamp_micros() - da, |s| {
                    s.timestamp_micros()
                });
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
/// Uses the last fix's delta when available. Falls back to sys_time or an
/// index-based estimate if no timestamp is usable.
fn dead_reckoned_gps_us(
    report: &InternalSatReport,
    last: &InternalFix,
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
    effective_time(last).timestamp_micros() + (idx as i64 + 1) * 1_000_000
}

/// Spherical forward bearing from A to B in degrees \[0, 360).
fn ghost_bearing(lat0_deg: f64, lon0_deg: f64, lat1_deg: f64, lon1_deg: f64) -> f64 {
    let lat0 = lat0_deg.to_radians();
    let lat1 = lat1_deg.to_radians();
    let dlon = (lon1_deg - lon0_deg).to_radians();
    let y = dlon.sin() * lat1.cos();
    let x = lat0.cos() * lat1.sin() - lat0.sin() * lat1.cos() * dlon.cos();
    (y.atan2(x).to_degrees() + 360.0) % 360.0
}

/// Move `dist_m` metres from (lat, lon) along `heading_deg`, return new (lat, lon).
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

/// Assign each satellite report to its nearest nav fix within `window`.
///
/// **Comparison timestamp** - binary search uses a GPS-domain estimate so the
/// two nearest candidate fixes can be found (fixes are sorted by GPS time).
/// The *distance* to each candidate is then computed in the most accurate
/// clock domain available:
///
/// - Report has `gps_time` → GPS-domain comparison against `effective_time(fix)`.
/// - Report has only `sys_time` **and** the fix has `sys_time` → same-domain
///   comparison `|rep.sys − fix.sys|` (single clock, no delta approximation).
/// - Report has only `sys_time`, fix has no `sys_time` → GPS-domain comparison
///   using the delta-corrected estimate from the nearest anchor.
///
/// Returns `(assignments, unassociated)` where `assignments[i]` is the
/// satellite report for `fixes[i]` (if any), and `unassociated` contains
/// every report that could not be matched.
fn associate_satellites(
    fixes: &[InternalFix],
    reports: Vec<InternalSatReport>,
    window: &Duration,
) -> (Vec<Option<InternalSatReport>>, Vec<InternalSatReport>) {
    let window_us = window.num_microseconds().unwrap_or(500_000);

    // Compute GPS/sys-clock delta anchors from fixes that carry both timestamps.
    let delta_anchors: Vec<(i64, i64)> = fixes
        .iter()
        .filter_map(|f| match (f.gps_time, f.sys_time) {
            (Some(gt), Some(st)) => Some((
                gt.timestamp_micros(),
                gt.timestamp_micros() - st.timestamp_micros(),
            )),
            _ => None,
        })
        .collect();

    let mut fix_claims: Vec<Option<(i64, usize)>> = vec![None; fixes.len()];

    for (rep_idx, report) in reports.iter().enumerate() {
        let Some(rep_us) = best_guess_gps_us(report, &delta_anchors) else {
            continue;
        };

        // When the report has no GPS time, record its sys_time for same-domain
        // distance comparison against fixes that also carry sys_time.
        let rep_sys_us: Option<i64> = if report.gps_time.is_none() {
            report.sys_time.map(|st| st.timestamp_micros())
        } else {
            None
        };

        let dist_to = |fix: &InternalFix| -> i64 {
            if let (Some(rsu), Some(fst)) = (rep_sys_us, fix.sys_time) {
                (rsu - fst.timestamp_micros()).abs()
            } else {
                (rep_us - effective_time(fix).timestamp_micros()).abs()
            }
        };

        let pos = fixes.partition_point(|f| effective_time(f).timestamp_micros() < rep_us);

        let mut best: Option<(i64, usize)> = None;

        if pos > 0
            && let Some(fix) = fixes.get(pos - 1)
        {
            let dist = dist_to(fix);
            if dist <= window_us {
                best = Some((dist, pos - 1));
            }
        }

        if let Some(fix) = fixes.get(pos) {
            let dist = dist_to(fix);
            if dist <= window_us {
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

    let mut reports_opt: Vec<Option<InternalSatReport>> = reports.into_iter().map(Some).collect();

    let mut assignments: Vec<Option<InternalSatReport>> = (0..fixes.len()).map(|_| None).collect();
    for (fix_idx, claim) in fix_claims.into_iter().enumerate() {
        if let Some((_, rep_idx)) = claim
            && let Some(slot) = assignments.get_mut(fix_idx)
            && let Some(rep) = reports_opt.get_mut(rep_idx).and_then(|r| r.take())
        {
            *slot = Some(rep);
        }
    }

    let unassociated = reports_opt.into_iter().flatten().collect();
    (assignments, unassociated)
}

/// Best available anchor time for placing external events (annotations, markers)
/// on the nav timeline.
///
/// External events carry host system-clock timestamps. Comparing them against
/// the fix's `sys_time` - the same clock domain - gives the most accurate
/// placement. Falls back to `gps_time` when no `sys_time` is available, and to
/// the Unix epoch as a last resort.
///
/// Contrast with [`effective_time`], which is GPS-time-first and governs fix
/// ordering and satellite report association.
fn timeline_time(fix: &InternalFix) -> DateTime<Utc> {
    fix.sys_time
        .map(SysTime::utc)
        .or_else(|| fix.gps_time.map(GpsTime::utc))
        .unwrap_or_default()
}

/// Interpolate positions for each annotation.
///
/// Returns `(resolved, out_of_range)`. In lenient mode the out-of-range vec
/// is always empty (positions are clamped and a warning is logged). In strict
/// mode, out-of-range annotations go into the second vec.
///
/// Annotation timestamps are treated as host system-clock times and compared
/// against each fix's `sys_time` (falling back to `gps_time` via
/// [`timeline_time`]). This matches the clock domain of all external event
/// sources (log files, user annotations). The fix slice must be sorted by a
/// time consistent with `timeline_time` - in practice GPS and sys times are
/// monotonically consistent for well-formed data.
fn interpolate_annotations(
    fixes: &[InternalFix],
    annotations: Vec<Annotation>,
    lenient: bool,
) -> (Vec<(Annotation, f64, f64)>, Vec<Annotation>) {
    let mut resolved = Vec::new();
    let mut out_of_range = Vec::new();

    for annotation in annotations {
        let ann_time = annotation.time;

        let pos = fixes.partition_point(|f| timeline_time(f) <= ann_time);

        let before = if pos > 0 { fixes.get(pos - 1) } else { None };
        let after = fixes.get(pos);

        let position = match (before, after) {
            (Some(b), Some(a)) => {
                let b_us = timeline_time(b).timestamp_micros();
                let a_us = timeline_time(a).timestamp_micros();
                let ann_us = ann_time.timestamp_micros();
                let t = if a_us == b_us {
                    0.0_f64
                } else {
                    (ann_us - b_us) as f64 / (a_us - b_us) as f64
                };
                let b_lat = b.lat.as_degrees();
                let b_lon = b.lon.as_degrees();
                let a_lat = a.lat.as_degrees();
                let a_lon = a.lon.as_degrees();
                Some((b_lat + t * (a_lat - b_lat), b_lon + t * (a_lon - b_lon)))
            }
            (Some(b), None) => {
                if lenient {
                    log::warn!(
                        "Annotation at {} is after the last nav fix; clamping to last position",
                        ann_time
                    );
                    Some((b.lat.as_degrees(), b.lon.as_degrees()))
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
                    Some((a.lat.as_degrees(), a.lon.as_degrees()))
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

/// Interpolate geographic positions for event markers from the built nav track.
///
/// Uses the same algorithm as [`interpolate_annotations`]: the sys_time is
/// matched against each fix's `timeline_time` (sys_time-first), then the
/// surrounding fixes bracket the position via linear interpolation.  Markers
/// before the first fix or after the last fix are clamped to the endpoint.
/// Markers with no fixes at all are silently dropped.
fn interpolate_event_markers(
    points: &[InternalPoint],
    pending: Vec<(String, chrono::DateTime<chrono::Utc>, Option<String>)>,
) -> Vec<EventMarkerPoint> {
    if points.is_empty() {
        return Vec::new();
    }

    // Build a lightweight slice of (timeline_time, lat, lon) from the already-sorted
    // internal points so we can binary-search without dealing with InternalFix directly.
    let fixes_view: Vec<(chrono::DateTime<chrono::Utc>, f64, f64)> = points
        .iter()
        .map(|p| {
            (
                timeline_time(&p.fix),
                p.fix.lat.as_degrees(),
                p.fix.lon.as_degrees(),
            )
        })
        .collect();

    pending
        .into_iter()
        .filter_map(|(variant_path, sys_time, annotation)| {
            let pos = fixes_view.partition_point(|(t, _, _)| *t <= sys_time);
            let before = if pos > 0 {
                fixes_view.get(pos - 1)
            } else {
                None
            };
            let after = fixes_view.get(pos);

            let (lat_deg, lon_deg) = match (before, after) {
                (Some(&(bt, blat, blon)), Some(&(at, alat, alon))) => {
                    let b_us = bt.timestamp_micros();
                    let a_us = at.timestamp_micros();
                    let t_us = sys_time.timestamp_micros();
                    let t = if a_us == b_us {
                        0.0_f64
                    } else {
                        (t_us - b_us) as f64 / (a_us - b_us) as f64
                    };
                    (blat + t * (alat - blat), blon + t * (alon - blon))
                }
                (Some(&(_, lat, lon)), None) | (None, Some(&(_, lat, lon))) => (lat, lon),
                (None, None) => return None,
            };

            Some(EventMarkerPoint {
                variant_path,
                sys_time,
                lat: Angle::degrees(lat_deg),
                lon: Angle::degrees(lon_deg),
                annotation,
            })
        })
        .collect()
}

/// A structured data quality warning about satellite data in a recording.
///
/// Returned by [`collect_satellite_warnings`].
#[derive(Debug, Clone, Copy)]
pub struct SatelliteWarning {
    /// Number of occurrences of this issue across all satellite reports.
    pub count: u32,
    /// Short label identifying the issue (e.g. `"satellite(s) with PRN 0"`).
    pub issue: &'static str,
    /// Explanation of why the issue matters and how to resolve it.
    pub description: &'static str,
}

/// All counts are aggregated over all reports so the log is never flooded.
/// See [`collect_satellite_issues`] and [`log_satellite_warnings`].
#[derive(Default, Debug, PartialEq, Eq)]
struct SatelliteIssues {
    /// Satellites with PRN = 0, which is invalid in all NMEA constellations.
    prn_zero: u32,
    /// GPS satellites with PRN 33–64 (the SBAS range - WAAS, EGNOS, MSAS, GAGAN).
    gps_sbas_range: u32,
    /// GPS satellites with PRN > 64 (entirely outside the valid GPS/SBAS range).
    gps_out_of_range: u32,
    /// GLONASS satellites with PRN 65–96 (looks like an un-stripped GNGSV system-PRN offset).
    glo_offset_range: u32,
    /// GLONASS satellites with PRN outside 1–32 and not in the 65–96 GNGSV range.
    glo_out_of_range: u32,
    /// Galileo satellites with PRN > 36 (valid range is E01–E36).
    gal_out_of_range: u32,
    /// BeiDou satellites with PRN > 63 (valid range is C01–C63).
    bds_out_of_range: u32,
    /// NavIC satellites with PRN > 14 (valid range is I01–I14).
    navic_out_of_range: u32,
    /// QZSS satellites with PRN > 10 (valid range is J01–J10).
    qzss_out_of_range: u32,
    /// Satellites with elevation < 0° - below the horizon, outside the valid NMEA range [0°, 90°].
    elevation_negative: u32,
    /// Satellites with elevation > 90° - above the zenith, outside the valid NMEA range [0°, 90°].
    elevation_above_90: u32,
    /// Satellites with azimuth outside [0°, 360°).
    azimuth_out_of_range: u32,
    /// Satellites with SNR ≈ 99 dB-Hz (common firmware sentinel for "no data").
    snr_sentinel_99: u32,
    /// Satellites with SNR > 60 dB-Hz (above the physical limit for civil GNSS).
    snr_above_60: u32,
    /// Satellites with SNR < 0 dB-Hz (invalid).
    snr_negative: u32,
    /// Reports containing duplicate (constellation, PRN) pairs.
    reports_with_duplicate_prn: u32,
}

impl SatelliteIssues {
    fn to_records(&self) -> Vec<SatelliteWarning> {
        // One row per issue category: its count and the human-readable text.
        // Driving the records off a table keeps each kind a single line and
        // means a new issue (e.g. another constellation's PRN range) is one
        // more entry rather than another `if` branch.
        let table = [
            (
                self.prn_zero,
                "satellite(s) with PRN 0",
                "PRN 0 is reserved and undefined in NMEA",
            ),
            (
                self.gps_sbas_range,
                "GPS satellite(s) with PRN 33-64",
                "this range is reserved for SBAS (WAAS, EGNOS, MSAS, GAGAN) per \
                    NMEA; if these are SBAS satellites, tag them with the GPS constellation \
                    (SBAS is treated as a GPS PRN range in the data model)",
            ),
            (
                self.gps_out_of_range,
                "GPS satellite(s) with PRN > 64",
                "outside the valid NMEA GPS/SBAS range (1-64); check the source data",
            ),
            (
                self.glo_offset_range,
                "GLONASS satellite(s) with PRN 65-96",
                "looks like an un-stripped NMEA 4.11 GNGSV system-PRN (slot + 64); \
                    expected format is slot numbers 1-32 - subtract 64 before reporting",
            ),
            (
                self.glo_out_of_range,
                "GLONASS satellite(s) with PRN outside 1-32",
                "not in the valid range (1-32) or GNGSV offset range (65-96); \
                    check the source data",
            ),
            (
                self.gal_out_of_range,
                "Galileo satellite(s) with PRN > 36",
                "outside the valid range (E01-E36)",
            ),
            (
                self.bds_out_of_range,
                "BeiDou satellite(s) with PRN > 63",
                "outside the valid range (C01-C63)",
            ),
            (
                self.navic_out_of_range,
                "NavIC satellite(s) with PRN > 14",
                "outside the valid range (I01-I14)",
            ),
            (
                self.qzss_out_of_range,
                "QZSS satellite(s) with PRN > 10",
                "outside the valid range (J01-J10)",
            ),
            (
                self.elevation_negative,
                "satellite(s) with negative elevation",
                "below the horizon; valid NMEA elevation range is [0°, 90°]",
            ),
            (
                self.elevation_above_90,
                "satellite(s) with elevation > 90°",
                "above the zenith; valid NMEA elevation range is [0°, 90°]",
            ),
            (
                self.azimuth_out_of_range,
                "satellite(s) with azimuth outside [0°, 360°)",
                "azimuth must be in [0°, 360°) per NMEA",
            ),
            (
                self.snr_sentinel_99,
                "satellite(s) with SNR ≈ 99 dB-Hz",
                "common firmware sentinel for unavailable signal strength; omit \
                    the SNR field when no measurement is available",
            ),
            (
                self.snr_above_60,
                "satellite(s) with SNR > 60 dB-Hz",
                "above the physical limit for civil GNSS receivers; check for \
                    sentinel values or unit errors",
            ),
            (
                self.snr_negative,
                "satellite(s) with negative SNR",
                "SNR must be >= 0 dB-Hz",
            ),
            (
                self.reports_with_duplicate_prn,
                "satellite report(s) with duplicate (constellation, PRN) pairs",
                "each satellite should appear at most once per report",
            ),
        ];
        table
            .into_iter()
            .filter(|&(count, _, _)| count > 0)
            .map(|(count, issue, description)| SatelliteWarning {
                count,
                issue,
                description,
            })
            .collect()
    }

    fn to_warning_strings(&self) -> Vec<String> {
        self.to_records()
            .into_iter()
            .map(|w| format!("{} {} - {}", w.count, w.issue, w.description))
            .collect()
    }
}

/// Validate satellite reports and return one human-readable warning string per issue
/// category found across all reports.
///
/// The same checks are run by [`NavRecorder::finish`] (which additionally logs each
/// warning via `log::warn!`).  Callers that load an existing `.gtd` file can use this
/// to surface the same diagnostics without going through the builder.
///
/// ## PRN ranges (NMEA 0183 v4.11, talker-specific GSV sentences)
///
/// | Constellation | Valid native PRN range | Notes |
/// |---|---|---|
/// | GPS | 1–32 | PRN 33–64 = SBAS (WAAS, EGNOS, …) |
/// | GLONASS | 1–32 | Slot numbers R01–R32. 65–96 = NMEA GNGSV offset not stripped |
/// | Galileo | 1–36 | E01–E36 |
/// | BeiDou | 1–63 | C01–C63 (GEO + IGSO + MEO combined) |
///
/// PRN 0 is invalid for all constellations.
///
/// Accepts any iterator of `SatelliteReport` references so callers can pass a slice,
/// a filtered iterator from a `NavFile`, etc. without needing to clone the reports.
pub fn collect_satellite_warnings<'a>(
    reports: impl IntoIterator<Item = &'a SatelliteReport>,
) -> Vec<SatelliteWarning> {
    collect_satellite_issues_inner(reports.into_iter().map(|r| r.tracked.as_slice())).to_records()
}

/// Inner implementation shared by the public API and the internal builder path.
#[expect(
    clippy::cognitive_complexity,
    reason = "one branch per validation rule; splitting would obscure rather than clarify"
)]
fn collect_satellite_issues_inner<'a>(
    reports: impl Iterator<Item = &'a [Satellite]>,
) -> SatelliteIssues {
    use std::collections::HashSet;

    let mut issues = SatelliteIssues::default();

    for tracked in reports {
        // (constellation, prn) - used for per-report duplicate detection.
        let mut seen: HashSet<(Constellation, u32)> = HashSet::new();
        let mut has_duplicate = false;

        for sat in tracked {
            let prn = sat.prn;

            if !seen.insert((sat.constellation, prn)) {
                has_duplicate = true;
            }

            if prn == 0 {
                issues.prn_zero += 1;
                // Skip further range checks. PRN 0 is invalid for all constellations.
                continue;
            }

            match sat.constellation {
                Constellation::Gps => match prn {
                    1..=32 => {}
                    33..=64 => issues.gps_sbas_range += 1,
                    _ => issues.gps_out_of_range += 1,
                },
                Constellation::Glonass => match prn {
                    1..=32 => {}
                    65..=96 => issues.glo_offset_range += 1,
                    _ => issues.glo_out_of_range += 1,
                },
                Constellation::Galileo => {
                    if prn > 36 {
                        issues.gal_out_of_range += 1;
                    }
                }
                Constellation::Beidou => {
                    if prn > 63 {
                        issues.bds_out_of_range += 1;
                    }
                }
                Constellation::Navic => {
                    if prn > 14 {
                        issues.navic_out_of_range += 1;
                    }
                }
                Constellation::Qzss => {
                    if prn > 10 {
                        issues.qzss_out_of_range += 1;
                    }
                }
            }

            if let Some(el) = sat.elevation {
                if el < 0.0 {
                    issues.elevation_negative += 1;
                } else if el > 90.0 {
                    issues.elevation_above_90 += 1;
                }
            }

            if let Some(az) = sat.azimuth
                && !(0.0..360.0).contains(&az)
            {
                issues.azimuth_out_of_range += 1;
            }

            if let Some(snr) = sat.snr {
                if snr < 0.0 {
                    issues.snr_negative += 1;
                } else if (snr - 99.0).abs() < 0.5 {
                    // 99 dB-Hz is a common firmware sentinel for "no data". Callers
                    // should pass `None` for unavailable SNR rather than a sentinel value.
                    issues.snr_sentinel_99 += 1;
                } else if snr > 60.0 {
                    issues.snr_above_60 += 1;
                }
            }
        }

        if has_duplicate {
            issues.reports_with_duplicate_prn += 1;
        }
    }

    issues
}

fn collect_satellite_issues(reports: &[InternalSatReport]) -> SatelliteIssues {
    collect_satellite_issues_inner(reports.iter().map(|r| r.tracked.as_slice()))
}

fn validate_satellite_data(reports: &[InternalSatReport]) {
    for msg in collect_satellite_issues(reports).to_warning_strings() {
        log::warn!("{msg}");
    }
}

/// Resolve the best available timestamp for a fix as a [`GpsTime`].
///
/// Uses `gps_time` when the receiver had an active lock, otherwise falls back
/// to `sys_time` (treated as a GPS-domain estimate), then to the Unix epoch.
/// All internal builder logic uses this instead of accessing `fix.gps_time` directly.
#[inline]
fn effective_time(fix: &InternalFix) -> GpsTime {
    fix.gps_time
        .unwrap_or_else(|| GpsTime::from_utc(fix.sys_time.map(SysTime::utc).unwrap_or_default()))
}

pub(crate) fn datetime_to_micros(dt: DateTime<Utc>) -> i64 {
    dt.timestamp_micros()
}

pub(crate) fn micros_to_datetime(us: i64) -> DateTime<Utc> {
    DateTime::from_timestamp_micros(us).unwrap_or_default()
}

/// Encode an optional `DateTime<Utc>` as u64 microseconds since Unix epoch.
///
/// `u64::MAX` is used as the sentinel for `None`. It corresponds to year ~584,542
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

#[cfg(test)]
mod validation_tests {
    use super::*;
    use crate::time_types::GpsTime;
    use crate::types::{Constellation, Satellite};
    use chrono::DateTime;

    /// `to_records` drives off a table with one row per `SatelliteIssues` field.
    /// Setting every field non-zero must surface exactly one record per field:
    /// the all-fields struct literal is a compile error if a field is added
    /// without updating it, and this length check fails if such a field has no
    /// matching table row (a silently-unreported issue category).
    #[test]
    fn every_issue_field_maps_to_a_record() {
        let issues = SatelliteIssues {
            prn_zero: 1,
            gps_sbas_range: 1,
            gps_out_of_range: 1,
            glo_offset_range: 1,
            glo_out_of_range: 1,
            gal_out_of_range: 1,
            bds_out_of_range: 1,
            navic_out_of_range: 1,
            qzss_out_of_range: 1,
            elevation_negative: 1,
            elevation_above_90: 1,
            azimuth_out_of_range: 1,
            snr_sentinel_99: 1,
            snr_above_60: 1,
            snr_negative: 1,
            reports_with_duplicate_prn: 1,
        };
        let records = issues.to_records();
        assert_eq!(records.len(), 16);
        assert!(records.iter().all(|r| r.count == 1));
    }

    fn gps_time() -> GpsTime {
        GpsTime::from_utc(DateTime::from_timestamp(1_748_000_000, 0).expect("valid"))
    }

    fn report(sats: Vec<Satellite>) -> InternalSatReport {
        InternalSatReport {
            gps_time: Some(gps_time()),
            sys_time: None,
            tracked: sats,
        }
    }

    fn sat(constellation: Constellation, prn: u32) -> Satellite {
        Satellite::builder()
            .constellation(constellation)
            .prn(prn)
            .build()
    }

    fn sat_el(constellation: Constellation, prn: u32, elevation: f32) -> Satellite {
        Satellite::builder()
            .constellation(constellation)
            .prn(prn)
            .elevation(elevation)
            .build()
    }

    fn sat_az(constellation: Constellation, prn: u32, azimuth: f32) -> Satellite {
        Satellite::builder()
            .constellation(constellation)
            .prn(prn)
            .azimuth(azimuth)
            .build()
    }

    fn sat_snr(constellation: Constellation, prn: u32, snr: f32) -> Satellite {
        Satellite::builder()
            .constellation(constellation)
            .prn(prn)
            .snr(snr)
            .build()
    }

    #[test]
    fn clean_data_produces_no_issues() {
        let reports = vec![report(vec![
            sat(Constellation::Gps, 1),
            sat(Constellation::Gps, 32),
            sat(Constellation::Glonass, 1),
            sat(Constellation::Glonass, 32),
            sat(Constellation::Galileo, 1),
            sat(Constellation::Galileo, 36),
            sat(Constellation::Beidou, 1),
            sat(Constellation::Beidou, 63),
        ])];
        assert_eq!(
            collect_satellite_issues(&reports),
            SatelliteIssues::default()
        );
    }

    #[test]
    fn prn_zero_detected() {
        let reports = vec![report(vec![sat(Constellation::Gps, 0)])];
        let issues = collect_satellite_issues(&reports);
        assert_eq!(issues.prn_zero, 1);
    }

    #[test]
    fn gps_sbas_range_prn_33_to_64() {
        let reports = vec![report(vec![
            sat(Constellation::Gps, 33),
            sat(Constellation::Gps, 64),
        ])];
        let issues = collect_satellite_issues(&reports);
        assert_eq!(issues.gps_sbas_range, 2);
        assert_eq!(issues.gps_out_of_range, 0);
    }

    #[test]
    fn gps_prn_above_64_is_out_of_range() {
        let reports = vec![report(vec![sat(Constellation::Gps, 65)])];
        let issues = collect_satellite_issues(&reports);
        assert_eq!(issues.gps_out_of_range, 1);
        assert_eq!(issues.gps_sbas_range, 0);
    }

    #[test]
    fn glonass_offset_range_65_to_96() {
        let reports = vec![report(vec![
            sat(Constellation::Glonass, 65),
            sat(Constellation::Glonass, 96),
        ])];
        let issues = collect_satellite_issues(&reports);
        assert_eq!(issues.glo_offset_range, 2);
        assert_eq!(issues.glo_out_of_range, 0);
    }

    #[test]
    fn glonass_prn_33_to_64_is_out_of_range() {
        let reports = vec![report(vec![sat(Constellation::Glonass, 33)])];
        let issues = collect_satellite_issues(&reports);
        assert_eq!(issues.glo_out_of_range, 1);
        assert_eq!(issues.glo_offset_range, 0);
    }

    #[test]
    fn galileo_prn_above_36_is_out_of_range() {
        let reports = vec![report(vec![sat(Constellation::Galileo, 37)])];
        let issues = collect_satellite_issues(&reports);
        assert_eq!(issues.gal_out_of_range, 1);
    }

    #[test]
    fn beidou_prn_above_63_is_out_of_range() {
        let reports = vec![report(vec![sat(Constellation::Beidou, 64)])];
        let issues = collect_satellite_issues(&reports);
        assert_eq!(issues.bds_out_of_range, 1);
    }

    #[test]
    fn negative_elevation_detected() {
        let reports = vec![report(vec![sat_el(Constellation::Gps, 1, -1.0)])];
        let issues = collect_satellite_issues(&reports);
        assert_eq!(issues.elevation_negative, 1);
    }

    #[test]
    fn elevation_above_90_detected() {
        let reports = vec![report(vec![sat_el(Constellation::Gps, 1, 91.0)])];
        let issues = collect_satellite_issues(&reports);
        assert_eq!(issues.elevation_above_90, 1);
    }

    #[test]
    fn azimuth_out_of_range_detected() {
        let reports = vec![report(vec![
            sat_az(Constellation::Gps, 1, -1.0),
            sat_az(Constellation::Gps, 2, 360.0),
        ])];
        let issues = collect_satellite_issues(&reports);
        assert_eq!(issues.azimuth_out_of_range, 2);
    }

    #[test]
    fn azimuth_boundary_values_are_valid() {
        let reports = vec![report(vec![
            sat_az(Constellation::Gps, 1, 0.0),
            sat_az(Constellation::Gps, 2, 359.9),
        ])];
        assert_eq!(
            collect_satellite_issues(&reports),
            SatelliteIssues::default()
        );
    }

    #[test]
    fn snr_sentinel_99_detected() {
        let reports = vec![report(vec![sat_snr(Constellation::Gps, 1, 99.0)])];
        let issues = collect_satellite_issues(&reports);
        assert_eq!(issues.snr_sentinel_99, 1);
        assert_eq!(issues.snr_above_60, 0);
    }

    #[test]
    fn snr_above_60_but_not_sentinel_detected() {
        let reports = vec![report(vec![sat_snr(Constellation::Gps, 1, 70.0)])];
        let issues = collect_satellite_issues(&reports);
        assert_eq!(issues.snr_above_60, 1);
        assert_eq!(issues.snr_sentinel_99, 0);
    }

    #[test]
    fn snr_negative_detected() {
        let reports = vec![report(vec![sat_snr(Constellation::Gps, 1, -1.0)])];
        let issues = collect_satellite_issues(&reports);
        assert_eq!(issues.snr_negative, 1);
    }

    #[test]
    fn duplicate_prn_in_same_report_detected() {
        let reports = vec![report(vec![
            sat(Constellation::Gps, 5),
            sat(Constellation::Gps, 5),
        ])];
        let issues = collect_satellite_issues(&reports);
        assert_eq!(issues.reports_with_duplicate_prn, 1);
    }

    #[test]
    fn same_prn_in_different_constellations_is_not_duplicate() {
        let reports = vec![report(vec![
            sat(Constellation::Gps, 5),
            sat(Constellation::Galileo, 5),
        ])];
        assert_eq!(
            collect_satellite_issues(&reports),
            SatelliteIssues::default()
        );
    }

    #[test]
    fn duplicate_counted_once_per_report_not_per_satellite() {
        let reports = vec![report(vec![
            sat(Constellation::Gps, 1),
            sat(Constellation::Gps, 1),
            sat(Constellation::Gps, 1),
        ])];
        let issues = collect_satellite_issues(&reports);
        assert_eq!(issues.reports_with_duplicate_prn, 1);
    }

    #[test]
    fn issues_aggregated_across_multiple_reports() {
        let reports = vec![
            report(vec![sat(Constellation::Gps, 0)]),
            report(vec![sat(Constellation::Gps, 0)]),
            report(vec![sat(Constellation::Gps, 0)]),
        ];
        let issues = collect_satellite_issues(&reports);
        assert_eq!(issues.prn_zero, 3);
    }

    #[test]
    fn prn_zero_skips_range_check() {
        // PRN 0 with Glonass: should only count as prn_zero, not glo_out_of_range.
        let reports = vec![report(vec![sat(Constellation::Glonass, 0)])];
        let issues = collect_satellite_issues(&reports);
        assert_eq!(issues.prn_zero, 1);
        assert_eq!(issues.glo_out_of_range, 0);
    }
}

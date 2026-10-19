use std::collections::BTreeMap;
use std::time::Duration;

use chrono::{DateTime, Utc};
use geotrace_sdk_units::snr;

use crate::error::{
    self, BuildError, Error, FieldLocation, MetaField, MetaStringWithNul, UnplacedRecordCounts,
    VariantPathError,
};
use crate::time_types::{GpsTime, SysTime};
use crate::types::{
    Annotation, Channel, Constellation, EventMarker, EventMarkerColor, EventMarkerIconChoice,
    EventMarkerPoint, EventMarkerStyle, Marker, Meta, NavFile, NavFix, NavFixTime, NavPoint,
    SatelliteReport, TravelMode,
};
use crate::variant_path::EventKind;
use crate::{
    Angle, PositionProvenance, SatelliteEvidence, SatelliteObservation, SatelliteObservationReport,
    SatelliteUsageReport,
};

use self::satellite_association::InternalSatReport;

mod satellite_association;

impl NavFix {
    fn ghost(gps_time: GpsTime, lat: Angle, lon: Angle, heading: Option<Angle>) -> Self {
        Self::builder()
            .time(NavFixTime::Receiver(gps_time.utc()))
            .lat(lat)
            .lon(lon)
            .maybe_heading(heading)
            .position_provenance(Some(PositionProvenance::SdkSynthesized))
            .build()
    }

    fn gps_sys_clock_delta_us(&self) -> Option<i64> {
        match self.time {
            NavFixTime::Both { gps, sys } => {
                gps.timestamp_micros().checked_sub(sys.timestamp_micros())
            }
            NavFixTime::Receiver(_) | NavFixTime::Host(_) => None,
        }
    }

    fn host_clock_time(&self) -> Option<SysTime> {
        self.sys_time().map(SysTime::from_utc)
    }

    fn effective_time(&self) -> GpsTime {
        GpsTime::from_utc(self.time.effective())
    }

    /// Annotation and event timestamps use the host clock for placement.
    fn timeline_time(&self) -> DateTime<Utc> {
        self.time
            .sys_time()
            .unwrap_or_else(|| self.time.effective())
    }
}

struct InternalPoint {
    fix: NavFix,
    evidence: SatelliteEvidence,
}

impl InternalPoint {
    fn into_nav_point(self) -> NavPoint {
        NavPoint::new(self.fix, self.evidence)
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
///     .with_meta(Meta::builder().title("My Track").build()?)
///     .open();
///
/// // Add data to `recorder`, then call `recorder.finish()`.
/// # Ok(())
/// # }
/// ```
#[derive(Clone)]
pub struct NavFileBuilder {
    meta: Option<Meta>,
    satellite_window: Duration,
    continue_on_error: bool,
    scrubbed_provenance: bool,
}

impl NavFileBuilder {
    /// Create a new builder with default settings.
    ///
    /// Defaults: satellite window = 500 ms, strict mode (errors on dropped data).
    pub fn new() -> Self {
        Self {
            meta: None,
            satellite_window: Duration::from_millis(500),
            continue_on_error: false,
            scrubbed_provenance: false,
        }
    }

    /// Attach file-level metadata (title, device, notes).
    pub fn with_meta(mut self, meta: Meta) -> Self {
        self.meta = Some(meta);
        self
    }

    /// Set the file title.
    ///
    /// Returns `Err` for a title with a nul byte.
    pub fn with_title(mut self, title: impl Into<String>) -> Result<Self, MetaStringWithNul> {
        let title = title.into();
        MetaField::Title.reject_nul_byte(&title)?;
        self.meta.get_or_insert_with(Meta::default).title = Some(title);
        Ok(self)
    }

    /// Set the device or sensor name.
    ///
    /// Returns `Err` for a device name with a nul byte.
    pub fn with_device(mut self, device: impl Into<String>) -> Result<Self, MetaStringWithNul> {
        let device = device.into();
        MetaField::Device.reject_nul_byte(&device)?;
        self.meta.get_or_insert_with(Meta::default).device = Some(device);
        Ok(self)
    }

    /// Set free-text notes for the file.
    ///
    /// Returns `Err` for notes with a nul byte.
    pub fn with_notes(mut self, notes: impl Into<String>) -> Result<Self, MetaStringWithNul> {
        let notes = notes.into();
        MetaField::Notes.reject_nul_byte(&notes)?;
        self.meta.get_or_insert_with(Meta::default).notes = Some(notes);
        Ok(self)
    }

    /// Set the stable identity key used by the app's history database.
    ///
    /// All recordings with the same identity are stored under the same group
    /// and appear together in the History window. The string should be stable
    /// across re-recordings - for example a device serial number or route name.
    ///
    /// Returns `Err` for an identity with a nul byte.
    pub fn with_identity(mut self, id: impl Into<String>) -> Result<Self, MetaStringWithNul> {
        let id = id.into();
        MetaField::Identity.reject_nul_byte(&id)?;
        self.meta.get_or_insert_with(Meta::default).identity = Some(id);
        Ok(self)
    }

    /// Declare the platform the recording was made on.
    ///
    /// Returns `Err` for a [`TravelMode::Unknown`] wire value with a nul byte.
    pub fn with_travel_mode(mut self, mode: TravelMode) -> Result<Self, MetaStringWithNul> {
        MetaField::TravelMode.reject_nul_byte(mode.name())?;
        self.meta.get_or_insert_with(Meta::default).travel_mode = Some(mode);
        Ok(self)
    }

    /// Override the maximum time gap for associating a satellite report to a nav fix.
    ///
    /// A window longer than `i64::MAX` microseconds, about 292,000 years, is
    /// capped there: every report is associated with its nearest fix.
    pub fn with_satellite_window(mut self, window: Duration) -> Self {
        self.satellite_window = window;
        self
    }

    /// Clamp an out-of-range time and log a warning. Drop an event with an invalid variant path
    /// and log an error.
    ///
    /// The strict build fails with [`BuildError::AnnotationsOutsideRange`],
    /// [`BuildError::EventMarkersOutsideRange`] or
    /// [`BuildError::InvalidEventMarkerVariantPath`]. After this call, an annotation
    /// or an event marker outside the nav fix time range is clamped to the
    /// nearest endpoint. An event from [`NavRecorder::add_event`] or
    /// [`NavRecorder::add_event_with_note`] whose variant path
    /// [`EventMarker::builder`] rejects is dropped.
    pub fn with_lenient_errors(mut self) -> Self {
        self.continue_on_error = true;
        self
    }

    /// Stamp `<scrubbed>` as the SDK version and no build commit at all.
    ///
    /// For a fixture or any other `.gtd` kept in version control: regenerating
    /// it from a different SDK build then writes the same bytes. The file does
    /// not record which SDK build wrote it.
    pub fn with_scrubbed_provenance(mut self) -> Self {
        self.scrubbed_provenance = true;
        self
    }

    /// Consume the configuration and return a [`NavRecorder`] ready for data.
    pub fn open(self) -> NavRecorder {
        NavRecorder {
            fixes: Vec::new(),
            satellite_reports: Vec::new(),
            annotations: Vec::new(),
            pending_event_markers: Vec::new(),
            event_marker_styles_by_variant_path: BTreeMap::new(),
            channels: Vec::new(),
            first_event_variant_path_rejection: None,
            meta: self.meta,
            satellite_window: self.satellite_window,
            continue_on_error: self.continue_on_error,
            scrubbed_provenance: self.scrubbed_provenance,
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
    fixes: Vec<NavFix>,
    satellite_reports: Vec<InternalSatReport>,
    annotations: Vec<Annotation>,
    pending_event_markers: Vec<(String, chrono::DateTime<chrono::Utc>, Option<String>)>,
    event_marker_styles_by_variant_path: BTreeMap<String, EventMarkerStyle>,
    channels: Vec<Channel>,
    first_event_variant_path_rejection: Option<VariantPathError>,
    meta: Option<Meta>,
    satellite_window: Duration,
    continue_on_error: bool,
    scrubbed_provenance: bool,
}

/// A timeline object that [`NavRecorder::add`] dispatches on.
///
/// Implemented for [`NavFix`], [`SatelliteReport`], [`SatelliteUsageReport`],
/// [`SatelliteObservationReport`], [`Annotation`],
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

impl NavRecord for SatelliteUsageReport {
    fn add_to(self, recorder: &mut NavRecorder) {
        recorder.add_satellite_usage_report(self);
    }
}

impl NavRecord for SatelliteObservationReport {
    fn add_to(self, recorder: &mut NavRecorder) {
        recorder.add_satellite_observation_report(self);
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

    /// Append a navigation fix with its source clocks and metadata.
    pub fn add_nav_fix(&mut self, fix: impl Into<NavFix>) -> &mut Self {
        self.fixes.push(fix.into());
        self
    }

    /// Convert a combined report into complete global usage and observation samples.
    pub fn add_satellite_report(&mut self, report: impl Into<SatelliteReport>) -> &mut Self {
        self.satellite_reports
            .push(InternalSatReport::from_sat_report(&report.into()));
        self
    }

    /// Append an independent usage sample for association during [`Self::finish`].
    pub fn add_satellite_usage_report(
        &mut self,
        report: impl Into<SatelliteUsageReport>,
    ) -> &mut Self {
        self.satellite_reports
            .push(InternalSatReport::from_usage(report.into()));
        self
    }

    /// Append an independent observation sample for association during [`Self::finish`].
    pub fn add_satellite_observation_report(
        &mut self,
        report: impl Into<SatelliteObservationReport>,
    ) -> &mut Self {
        self.satellite_reports
            .push(InternalSatReport::from_observation(report.into()));
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
    /// The channel keeps its own sample timestamps. It is correlated with the
    /// nav points by time at query time, not resampled here. Build one with
    /// [`Channel::builder`].
    pub fn add_channel(&mut self, channel: Channel) -> &mut Self {
        self.channels.push(channel);
        self
    }

    /// Add a typed event marker derived from an [`EventKind`] implementation.
    ///
    /// If `event.variant_path()` returns `None` (e.g. a `#[event_kind(skip)]`
    /// variant), the call is a silent no-op. The recorder drops an event whose variant path
    /// [`EventMarker::builder`] rejects. In strict mode [`finish`](Self::finish) then fails with
    /// [`BuildError::InvalidEventMarkerVariantPath`]. In lenient mode this call logs an error
    /// with the path and the rule it breaks.
    pub fn add_event(
        &mut self,
        event: &impl EventKind,
        sys_time: impl Into<chrono::DateTime<chrono::Utc>>,
    ) -> &mut Self {
        let Some(path) = event.variant_path() else {
            return self;
        };
        self.add_typed_event_marker(path, event, sys_time.into(), event.event_note());
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
        self.add_typed_event_marker(path, event, sys_time.into(), Some(note.into()));
        self
    }

    fn add_typed_event_marker(
        &mut self,
        path: String,
        event: &impl EventKind,
        sys_time: DateTime<Utc>,
        annotation: Option<String>,
    ) {
        if let Err(rejection) = error::validate_variant_path(&path) {
            if self.continue_on_error {
                log::error!("Event marker at {sys_time} dropped: {rejection:#}");
            } else {
                self.first_event_variant_path_rejection
                    .get_or_insert(rejection);
            }
            return;
        }
        self.register_derived_icon_for_unstyled_path(&path, event);
        self.pending_event_markers
            .push((path, sys_time, annotation));
    }

    fn register_derived_icon_for_unstyled_path(&mut self, path: &str, event: &impl EventKind) {
        let Some(icon) = event.marker_icon() else {
            return;
        };
        // `BTreeMap::entry` takes an owned key and costs one `String` allocation per event.
        // `contains_key` takes a `&str`, and the insert below runs for the first event of a path.
        if self.event_marker_styles_by_variant_path.contains_key(path) {
            return;
        }
        self.event_marker_styles_by_variant_path.insert(
            path.to_owned(),
            EventMarkerStyle {
                variant_path: path.to_owned(),
                icon: EventMarkerIconChoice::Icon(icon),
                color: EventMarkerColor::Auto,
            },
        );
    }

    /// Register an icon and color for a variant path.
    ///
    /// Variants with no registered style use the fallback hash color and Pin icon.
    /// The recorder keeps one style per variant path, the style of the last call for that path.
    /// It keeps that style over an `#[event_kind(icon = …)]` icon, before or after
    /// [`add_event`](Self::add_event) records the variant. [`finish`](Self::finish) returns the
    /// styles in variant path order.
    pub fn add_event_marker_style(&mut self, style: EventMarkerStyle) -> &mut Self {
        self.event_marker_styles_by_variant_path
            .insert(style.variant_path.clone(), style);
        self
    }

    /// Associate each report with its nearest fix within the configured window.
    /// Receiver times use the receiver clock. Host-only reports use a shared host
    /// clock when available, then the nearest recorded clock delta.
    /// Multiple reports remain independent. Orphan reports with equal normalized
    /// times share one synthesized position. Original report clocks are retained.
    pub fn finish(mut self) -> Result<NavFile, BuildError> {
        if let Some(source) = self.first_event_variant_path_rejection {
            return Err(BuildError::InvalidEventMarkerVariantPath { source });
        }
        validate_satellite_data(&self.satellite_reports);

        self.fixes.sort_by_key(NavFix::effective_time);
        self.annotations.sort_by_key(|a| a.time);

        let records_need_a_position = !self.satellite_reports.is_empty()
            || !self.annotations.is_empty()
            || !self.pending_event_markers.is_empty();
        if self.fixes.is_empty() && records_need_a_position {
            return Err(BuildError::NoNavFixes(UnplacedRecordCounts {
                satellite_reports: self.satellite_reports.len(),
                annotations: self.annotations.len(),
                event_markers: self.pending_event_markers.len(),
            }));
        }

        let (sat_assignments, unassociated) = satellite_association::associate_satellites(
            &self.fixes,
            self.satellite_reports,
            self.satellite_window,
        )?;

        // Build ghost nav fixes for orphaned satellite reports.
        let ghost_points = satellite_association::ghost_nav_points_for(&self.fixes, unassociated)?;
        if !ghost_points.is_empty() {
            log::debug!(
                "{} ghost nav fix(es) created for satellite reports outside the {}ms association window",
                ghost_points.len(),
                self.satellite_window.as_millis(),
            );
        }

        let (resolved_markers, out_of_range) =
            interpolate_annotations(&self.fixes, self.annotations, self.continue_on_error);
        let out_of_range_count = out_of_range.len();

        if out_of_range_count > 0 {
            return Err(BuildError::AnnotationsOutsideRange {
                count: out_of_range_count,
            });
        }

        // Merge real nav points and ghost nav points, sorted by time.
        let mut internal_points: Vec<InternalPoint> = self
            .fixes
            .into_iter()
            .zip(sat_assignments)
            .map(|(fix, evidence)| InternalPoint { fix, evidence })
            .collect();
        internal_points.extend(ghost_points);
        internal_points.sort_by_key(|p| p.fix.effective_time());

        let placed_event_markers = interpolate_event_markers(
            &internal_points,
            self.pending_event_markers,
            self.continue_on_error,
        );
        if placed_event_markers.outside_the_fix_time_range > 0 {
            return Err(BuildError::EventMarkersOutsideRange {
                count: placed_event_markers.outside_the_fix_time_range,
            });
        }

        // Convert to public output types at the output boundary.
        let nav_points: Vec<NavPoint> = internal_points
            .into_iter()
            .map(InternalPoint::into_nav_point)
            .collect();

        let markers = resolved_markers
            .into_iter()
            .map(|(annotation, position)| Marker {
                annotation,
                lat: position.lat,
                lon: position.lon,
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

        let mut meta = self.meta.unwrap_or_default();
        if self.scrubbed_provenance {
            meta.stamp_scrubbed_provenance();
        } else {
            meta.stamp_this_build();
        }

        Ok(NavFile {
            meta,
            debug_time_repair: None,
            nav_points,
            markers,
            event_markers: placed_event_markers.markers,
            event_marker_styles: self
                .event_marker_styles_by_variant_path
                .into_values()
                .collect(),
            channels,
        })
    }
}

/// A fix reduced to what placing an external event on the nav timeline needs.
#[derive(Clone, Copy)]
struct TimelineFix {
    time: DateTime<Utc>,
    position: TimelinePosition,
}

impl TimelineFix {
    fn from_internal_fix(fix: &NavFix) -> Self {
        Self {
            time: fix.timeline_time(),
            position: TimelinePosition::from_internal_fix(fix),
        }
    }
}

#[derive(Clone, Copy)]
struct TimelinePosition {
    lat: Angle,
    lon: Angle,
}

impl TimelinePosition {
    fn from_internal_fix(fix: &NavFix) -> Self {
        Self {
            lat: fix.lat,
            lon: fix.lon,
        }
    }

    /// The position `fraction` of the way from `self` to `other`, with the
    /// longitude taken along the shortest arc and wrapped into [-180, 180).
    /// Two longitudes exactly 180° apart have two arcs of equal length. This
    /// takes the westward one.
    fn interpolated_to(self, other: Self, fraction: f64) -> Self {
        let lat_deg = self.lat.as_degrees();
        let lon_deg = self.lon.as_degrees();
        Self {
            lat: Angle::degrees(lat_deg + fraction * (other.lat.as_degrees() - lat_deg)),
            lon: Angle::degrees(
                lon_deg + fraction * self.lon.signed_arc_to(other.lon).as_degrees(),
            )
            .wrapped_to_plus_minus_180_degrees(),
        }
    }

    /// Spherical forward bearing from `self` to `other`, in [0, 360) degrees.
    fn bearing_to(self, other: Self) -> Angle {
        let lat0 = self.lat.as_radians();
        let lat1 = other.lat.as_radians();
        let dlon = self.lon.signed_arc_to(other.lon).as_radians();
        let y = dlon.sin() * lat1.cos();
        let x = lat0.cos() * lat1.sin() - lat0.sin() * lat1.cos() * dlon.cos();
        Angle::degrees((y.atan2(x).to_degrees() + 360.0) % 360.0)
    }

    /// This position moved `distance_m` metres along `heading` over a sphere,
    /// with the longitude wrapped into [-180, 180).
    fn advanced_along_great_circle(self, heading: Angle, distance_m: f64) -> Self {
        const EARTH_RADIUS_M: f64 = 6_371_000.0;
        let angular_distance = distance_m / EARTH_RADIUS_M;
        let heading = heading.as_radians();
        let lat = self.lat.as_radians();
        let new_lat = (lat.sin() * angular_distance.cos()
            + lat.cos() * angular_distance.sin() * heading.cos())
        .asin();
        let new_lon = self.lon.as_radians()
            + (heading.sin() * angular_distance.sin() * lat.cos())
                .atan2(angular_distance.cos() - lat.sin() * new_lat.sin());
        Self {
            lat: Angle::radians(new_lat),
            lon: Angle::radians(new_lon).wrapped_to_plus_minus_180_degrees(),
        }
    }
}

/// Where an external event's time sits on the nav timeline, with the position
/// to give the event: the interpolated position inside the fix time span, and
/// the position of the first or the last fix outside it.
enum TimelinePlacement {
    AfterLastFix(TimelinePosition),
    BeforeFirstFix(TimelinePosition),
    NoFixes,
    WithinFixTimeSpan(TimelinePosition),
}

impl TimelinePlacement {
    /// The position to give the event, `None` where it is outside the fix time
    /// span and the build is strict. A lenient build takes the position of the
    /// nearest endpoint and logs a warning naming `subject`.
    fn resolved_position(
        self,
        lenient: bool,
        subject: impl std::fmt::Display,
    ) -> Option<TimelinePosition> {
        match self {
            Self::WithinFixTimeSpan(position) => Some(position),
            Self::BeforeFirstFix(position) if lenient => {
                log::warn!("{subject} is before the first nav fix: clamped to the first position");
                Some(position)
            }
            Self::AfterLastFix(position) if lenient => {
                log::warn!("{subject} is after the last nav fix: clamped to the last position");
                Some(position)
            }
            Self::BeforeFirstFix(_) | Self::AfterLastFix(_) | Self::NoFixes => None,
        }
    }
}

/// The fixes an external event is placed against, sorted by
/// [`NavFix::timeline_time`], the key [`FixTimeline::place`] searches.
/// The nav points stay in [`NavFix::effective_time`] order, and the two
/// orders differ where a fix has both a receiver and a host timestamp.
struct FixTimeline(Vec<TimelineFix>);

impl FixTimeline {
    fn from_fixes<'a>(fixes: impl Iterator<Item = &'a NavFix>) -> Self {
        let mut timeline: Vec<TimelineFix> = fixes.map(TimelineFix::from_internal_fix).collect();
        timeline.sort_by_key(|fix| fix.time);
        Self(timeline)
    }

    /// Place an external event's time on the nav timeline.
    ///
    /// A time equal to a fix's time is placed at that fix, and at the first of
    /// them in [`NavFix::effective_time`] order where several fixes share
    /// that time.
    fn place(&self, time: DateTime<Utc>) -> TimelinePlacement {
        let pos = self.0.partition_point(|fix| fix.time < time);
        let before = pos.checked_sub(1).and_then(|index| self.0.get(index));

        match (before, self.0.get(pos)) {
            (_, Some(at)) if at.time == time => TimelinePlacement::WithinFixTimeSpan(at.position),
            (Some(before), Some(after)) => {
                let before_us = before.time.timestamp_micros();
                let span_us = after.time.timestamp_micros() - before_us;
                let fraction = if span_us == 0 {
                    0.0_f64
                } else {
                    (time.timestamp_micros() - before_us) as f64 / span_us as f64
                };
                TimelinePlacement::WithinFixTimeSpan(
                    before.position.interpolated_to(after.position, fraction),
                )
            }
            (Some(last), None) => TimelinePlacement::AfterLastFix(last.position),
            (None, Some(first)) => TimelinePlacement::BeforeFirstFix(first.position),
            (None, None) => TimelinePlacement::NoFixes,
        }
    }
}

/// Interpolate positions for each annotation.
///
/// Returns `(resolved, out_of_range)`. In lenient mode `out_of_range`
/// is always empty (positions are clamped and a warning is logged). In strict
/// mode, out-of-range annotations go into `out_of_range`.
///
/// An annotation's timestamp is a host system-clock time, as are the timestamps
/// of every external event source (log files, user annotations).
/// [`FixTimeline`] places it against each fix's `sys_time`, or against the
/// `gps_time` of a fix that has no `sys_time`.
fn interpolate_annotations(
    fixes: &[NavFix],
    annotations: Vec<Annotation>,
    lenient: bool,
) -> (Vec<(Annotation, TimelinePosition)>, Vec<Annotation>) {
    let timeline = FixTimeline::from_fixes(fixes.iter());
    let mut resolved = Vec::new();
    let mut out_of_range = Vec::new();

    for annotation in annotations {
        let ann_time = annotation.time;

        let position = timeline
            .place(ann_time)
            .resolved_position(lenient, format_args!("Annotation at {ann_time}"));

        match position {
            Some(position) => resolved.push((annotation, position)),
            None => out_of_range.push(annotation),
        }
    }

    (resolved, out_of_range)
}

/// The event markers [`interpolate_event_markers`] placed, and a count of the
/// markers it left unplaced. `outside_the_fix_time_range` is 0 in lenient mode.
struct PlacedEventMarkers {
    markers: Vec<EventMarkerPoint>,
    outside_the_fix_time_range: usize,
}

/// Interpolate geographic positions for event markers from the built nav track.
///
/// [`FixTimeline::place`] places the `sys_time` on the nav timeline.
/// In lenient mode a marker before the first fix or after the last fix is
/// clamped to that endpoint and logged as a warning. In strict mode it is
/// counted in `outside_the_fix_time_range` and left out of `markers`.
fn interpolate_event_markers(
    points: &[InternalPoint],
    pending: Vec<(String, DateTime<Utc>, Option<String>)>,
    lenient: bool,
) -> PlacedEventMarkers {
    let timeline = FixTimeline::from_fixes(points.iter().map(|p| &p.fix));

    let mut markers = Vec::new();
    let mut outside_the_fix_time_range = 0;

    for (variant_path, sys_time, annotation) in pending {
        let position = timeline.place(sys_time).resolved_position(
            lenient,
            format_args!("Event marker {variant_path:?} at {sys_time}"),
        );

        match position {
            Some(position) => markers.push(EventMarkerPoint {
                variant_path,
                sys_time,
                lat: position.lat,
                lon: position.lon,
                annotation,
            }),
            None => outside_the_fix_time_range += 1,
        }
    }

    PlacedEventMarkers {
        markers,
        outside_the_fix_time_range,
    }
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
    /// Satellites with SNR ≈ 99 dB-Hz (common firmware sentinel value for "no data").
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
        // Driving the records off a table keeps each kind a single line: a new
        // issue (e.g. another constellation's PRN range) is one more entry.
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
    collect_satellite_issues_inner(reports.into_iter().map(|report| {
        report
            .tracked
            .iter()
            .map(SatelliteObservation::from)
            .collect::<Vec<_>>()
    }))
    .to_records()
}

/// Inner implementation shared by the public API and the internal builder path.
#[expect(
    clippy::cognitive_complexity,
    reason = "one branch per validation rule; splitting would obscure rather than clarify"
)]
fn collect_satellite_issues_inner(
    reports: impl Iterator<Item = impl AsRef<[SatelliteObservation]>>,
) -> SatelliteIssues {
    let mut issues = SatelliteIssues::default();

    for report in reports {
        let tracked = report.as_ref();
        let mut has_duplicate = false;

        for (i, sat) in tracked.iter().enumerate() {
            let prn = sat.identity.prn;

            // Scanning the rows before this one allocates nothing, and costs about as
            // much as hashing them: a report has at most a few dozen rows.
            if tracked
                .iter()
                .take(i)
                .any(|earlier| earlier.identity == sat.identity)
            {
                has_duplicate = true;
            }

            if prn == 0 {
                issues.prn_zero += 1;
                // Skip further range checks. PRN 0 is invalid for all constellations.
                continue;
            }

            match sat.identity.constellation {
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
                } else if sat.snr.is_some_and(snr::is_no_data_sentinel) {
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
    collect_satellite_issues_inner(reports.iter().map(InternalSatReport::diagnostic_rows))
}

fn validate_satellite_data(reports: &[InternalSatReport]) {
    for msg in collect_satellite_issues(reports).to_warning_strings() {
        log::warn!("{msg}");
    }
}

pub(crate) fn datetime_to_micros(dt: DateTime<Utc>) -> i64 {
    dt.timestamp_micros()
}

/// The writer stores a timestamp as the two's complement bits of its microsecond
/// count. The one count whose bits equal [`ABSENT_TIMESTAMP_MICROS`] is rejected,
/// since the reader takes that value for an absent timestamp.
pub(crate) fn datetime_to_u64(
    dt: DateTime<Utc>,
    location: FieldLocation,
    record: usize,
) -> Result<u64, Error> {
    let micros = dt.timestamp_micros().cast_unsigned();
    if micros == ABSENT_TIMESTAMP_MICROS {
        return Err(Error::TimestampIsTheAbsentValue {
            group: location.group,
            dataset: location.dataset,
            record,
        });
    }
    Ok(micros)
}

pub(crate) fn opt_datetime_to_u64(
    dt: Option<DateTime<Utc>>,
    location: FieldLocation,
    record: usize,
) -> Result<u64, Error> {
    dt.map_or(Ok(ABSENT_TIMESTAMP_MICROS), |dt| {
        datetime_to_u64(dt, location, record)
    })
}

/// The value the optional timestamp datasets store for an absent timestamp.
pub(crate) const ABSENT_TIMESTAMP_MICROS: u64 = u64::MAX;

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use crate::Satellite;

    use super::*;

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

    fn report(tracked: Vec<Satellite>) -> InternalSatReport {
        InternalSatReport::from_sat_report(&SatelliteReport {
            time: NavFixTime::Receiver(DateTime::from_timestamp(1_748_000_000, 0).expect("valid")),
            tracked,
        })
    }

    fn sat(constellation: Constellation, prn: u32) -> Satellite {
        Satellite::builder()
            .constellation(constellation)
            .prn(prn)
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
            sat(Constellation::Navic, 1),
            sat(Constellation::Navic, 14),
            sat(Constellation::Qzss, 1),
            sat(Constellation::Qzss, 10),
        ])];
        assert_eq!(
            collect_satellite_issues(&reports),
            SatelliteIssues::default()
        );
    }

    #[rstest]
    #[case::prn_0_skips_the_constellation_range_check(
        vec![sat(Constellation::Glonass, 0)],
        SatelliteIssues { prn_zero: 1, ..SatelliteIssues::default() },
    )]
    #[case::gps_prn_33_to_64_is_the_sbas_range(
        vec![sat(Constellation::Gps, 33), sat(Constellation::Gps, 64)],
        SatelliteIssues { gps_sbas_range: 2, ..SatelliteIssues::default() },
    )]
    #[case::gps_prn_above_64_is_out_of_range(
        vec![sat(Constellation::Gps, 65)],
        SatelliteIssues { gps_out_of_range: 1, ..SatelliteIssues::default() },
    )]
    #[case::glonass_prn_65_to_96_is_the_gngsv_offset_range(
        vec![sat(Constellation::Glonass, 65), sat(Constellation::Glonass, 96)],
        SatelliteIssues { glo_offset_range: 2, ..SatelliteIssues::default() },
    )]
    #[case::glonass_prn_33_to_64_is_out_of_range(
        vec![sat(Constellation::Glonass, 33)],
        SatelliteIssues { glo_out_of_range: 1, ..SatelliteIssues::default() },
    )]
    #[case::galileo_prn_above_36_is_out_of_range(
        vec![sat(Constellation::Galileo, 37)],
        SatelliteIssues { gal_out_of_range: 1, ..SatelliteIssues::default() },
    )]
    #[case::beidou_prn_above_63_is_out_of_range(
        vec![sat(Constellation::Beidou, 64)],
        SatelliteIssues { bds_out_of_range: 1, ..SatelliteIssues::default() },
    )]
    #[case::navic_prn_above_14_is_out_of_range(
        vec![sat(Constellation::Navic, 15)],
        SatelliteIssues { navic_out_of_range: 1, ..SatelliteIssues::default() },
    )]
    #[case::qzss_prn_above_10_is_out_of_range(
        vec![sat(Constellation::Qzss, 11)],
        SatelliteIssues { qzss_out_of_range: 1, ..SatelliteIssues::default() },
    )]
    #[case::elevation_below_the_horizon(
        vec![Satellite { elevation: Some(-1.0), ..sat(Constellation::Gps, 1) }],
        SatelliteIssues { elevation_negative: 1, ..SatelliteIssues::default() },
    )]
    #[case::elevation_above_the_zenith(
        vec![Satellite { elevation: Some(91.0), ..sat(Constellation::Gps, 1) }],
        SatelliteIssues { elevation_above_90: 1, ..SatelliteIssues::default() },
    )]
    #[case::azimuth_below_0_and_at_360(
        vec![
            Satellite { azimuth: Some(-1.0), ..sat(Constellation::Gps, 1) },
            Satellite { azimuth: Some(360.0), ..sat(Constellation::Gps, 2) },
        ],
        SatelliteIssues { azimuth_out_of_range: 2, ..SatelliteIssues::default() },
    )]
    #[case::snr_at_the_99_no_data_value(
        vec![Satellite { snr: Some(99.0), ..sat(Constellation::Gps, 1) }],
        SatelliteIssues { snr_sentinel_99: 1, ..SatelliteIssues::default() },
    )]
    #[case::snr_above_60(
        vec![Satellite { snr: Some(70.0), ..sat(Constellation::Gps, 1) }],
        SatelliteIssues { snr_above_60: 1, ..SatelliteIssues::default() },
    )]
    #[case::snr_below_0(
        vec![Satellite { snr: Some(-1.0), ..sat(Constellation::Gps, 1) }],
        SatelliteIssues { snr_negative: 1, ..SatelliteIssues::default() },
    )]
    #[case::a_prn_twice_two_rows_apart(
        vec![sat(Constellation::Gps, 5), sat(Constellation::Gps, 12), sat(Constellation::Gps, 5)],
        SatelliteIssues { reports_with_duplicate_prn: 1, ..SatelliteIssues::default() },
    )]
    #[case::a_prn_three_times_counts_the_report_once(
        vec![sat(Constellation::Gps, 1), sat(Constellation::Gps, 1), sat(Constellation::Gps, 1)],
        SatelliteIssues { reports_with_duplicate_prn: 1, ..SatelliteIssues::default() },
    )]
    fn each_issue_is_counted_in_its_own_field(
        #[case] tracked: Vec<Satellite>,
        #[case] expected_issues: SatelliteIssues,
    ) {
        assert_eq!(
            collect_satellite_issues(&[report(tracked)]),
            expected_issues
        );
    }

    #[test]
    fn azimuth_boundary_values_are_valid() {
        let reports = vec![report(vec![
            Satellite {
                azimuth: Some(0.0),
                ..sat(Constellation::Gps, 1)
            },
            Satellite {
                azimuth: Some(359.9),
                ..sat(Constellation::Gps, 2)
            },
        ])];
        assert_eq!(
            collect_satellite_issues(&reports),
            SatelliteIssues::default()
        );
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
    fn ghost_nav_points_for_fails_on_reports_without_a_nav_fix() {
        let orphan_reports = vec![
            report(vec![sat(Constellation::Gps, 1)]),
            report(vec![sat(Constellation::Gps, 2)]),
        ];

        let result = satellite_association::ghost_nav_points_for(&[], orphan_reports);

        assert!(matches!(
            result,
            Err(BuildError::NoNavFixes(UnplacedRecordCounts {
                satellite_reports: 2,
                annotations: 0,
                event_markers: 0,
            }))
        ));
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
}

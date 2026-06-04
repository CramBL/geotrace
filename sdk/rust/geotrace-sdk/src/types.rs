use std::{fs::File, io, path::Path};

use crate::{Angle, Velocity};
use chrono::{DateTime, Utc};

use crate::error::{Error, EventMarkerError};

/// A single GPS/GNSS fix: position, heading, and optional speed at a point in time.
///
/// `heading` is `None` for synthetic/ghost fixes where the actual direction is
/// unknown (e.g., dead-reckoned positions emitted only to carry satellite reports).
/// The app renders those as circles rather than directional arrows.
///
/// `gps_time` is the GPS-receiver timestamp; it is `None` when the receiver had no
/// lock at the time of this record.
/// Do not substitute `sys_time` for `gps_time` on the client side - pass `None`
/// and let the SDK resolve the effective time from `sys_time` internally.
///
/// `sys_time` is the system-clock timestamp recorded alongside the GPS fix.
/// Providing it allows the builder to compute the GPS/system-clock delta, which
/// is used to convert satellite report system-clock timestamps into the GPS time
/// domain for accurate ghost-fix interpolation during no-fix periods.
#[derive(bon::Builder, Debug, Clone, Copy)]
pub struct NavFix {
    /// GPS-receiver timestamp; `None` when the receiver had no active lock.
    #[builder(into)]
    pub gps_time: Option<DateTime<Utc>>,
    /// System-clock time at the moment of this fix, if recorded.
    #[builder(into)]
    pub sys_time: Option<DateTime<Utc>>,
    pub lat: Angle,
    pub lon: Angle,
    /// Compass heading in \[0°, 360°). `None` = unknown direction (ghost fix).
    pub heading: Option<Angle>,
    pub speed: Option<Velocity>,
    /// Estimated horizontal position accuracy in metres, as reported by the GPS receiver.
    ///
    /// The app renders a translucent blue circle of this radius around the point when
    /// present. `None` when the receiver did not report an accuracy estimate.
    pub eph_m: Option<f64>,
}

/// A satellite visibility report captured at a point in time.
///
/// Supply at least one of `gps_time` or `sys_time`. When neither is present the
/// builder logs a warning and drops the report.
///
/// - `gps_time`: the GPS-receiver timestamp, available when the receiver had an
///   active fix at the time of capture.
/// - `sys_time`: the system-clock timestamp, available whenever the host OS can
///   read the clock. This is used together with the GPS/system-clock delta derived
///   from surrounding NavFixes to place orphan reports in the GPS time domain.
#[derive(bon::Builder, Debug, Clone)]
pub struct SatelliteReport {
    /// GPS-domain timestamp; present when the receiver had an active fix.
    #[builder(into)]
    pub gps_time: Option<DateTime<Utc>>,
    /// System-clock timestamp at capture time; optional but strongly recommended.
    #[builder(into)]
    pub sys_time: Option<DateTime<Utc>>,
    /// All satellites currently tracked (may include satellites not in the fix).
    pub tracked: Vec<Satellite>,
}

/// One tracked satellite with optional signal metrics.
#[derive(bon::Builder, Debug, Clone, Copy)]
pub struct Satellite {
    #[builder(into)]
    pub constellation: Constellation,
    pub prn: u32,
    /// Whether this satellite is contributing to the current positional fix.
    #[builder(default)]
    pub in_fix: bool,
    /// Elevation above horizon in degrees; `None` if unavailable.
    pub elevation: Option<f32>,
    /// Azimuth from true north in degrees; `None` if unavailable.
    pub azimuth: Option<f32>,
    /// Signal-to-noise ratio in dB-Hz; `None` if unavailable.
    pub snr: Option<f32>,
}

/// GNSS constellation identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Constellation {
    Gps,
    Glonass,
    Galileo,
    Beidou,
}

impl Constellation {
    /// Stable u8 encoding written to the `tracked_sats/constellation` dataset.
    pub(crate) fn to_u8(self) -> u8 {
        match self {
            Constellation::Gps => 0,
            Constellation::Glonass => 1,
            Constellation::Galileo => 2,
            Constellation::Beidou => 3,
        }
    }

    pub(crate) fn from_u8(code: u8, dataset: &'static str) -> Result<Self, Error> {
        match code {
            0 => Ok(Constellation::Gps),
            1 => Ok(Constellation::Glonass),
            2 => Ok(Constellation::Galileo),
            3 => Ok(Constellation::Beidou),
            _ => Err(Error::UnknownConstellation {
                code: i16::from(code),
                dataset,
            }),
        }
    }

    pub fn try_from_lower_case(s: impl AsRef<str>) -> Result<Self, Error> {
        let s = s.as_ref();
        match s {
            "gps" => Ok(Constellation::Gps),
            "glonass" => Ok(Constellation::Glonass),
            "galileo" => Ok(Constellation::Galileo),
            "beidou" => Ok(Constellation::Beidou),
            _ => Err(Error::UnknownConstellationName { name: s.to_owned() }),
        }
    }
}

/// A user-defined map annotation with an optional label and icon.
#[derive(Debug, Clone)]
pub struct Annotation {
    /// Display label; `None` or empty string renders as unlabelled.
    pub label: Option<String>,
    /// Visual icon; `None` defaults to `MarkerIcon::Pin` when loaded.
    pub icon: Option<MarkerIcon>,
    pub time: DateTime<Utc>,
}

#[bon::bon]
impl Annotation {
    /// Build a new [`Annotation`].
    ///
    /// Empty or whitespace-only labels are automatically converted to `None`.
    #[builder(finish_fn = build)]
    pub fn new(
        #[builder(into)] time: DateTime<Utc>,
        #[builder(into)] label: Option<String>,
        icon: Option<MarkerIcon>,
    ) -> Self {
        Self {
            time,
            label: label.filter(|s| !s.trim().is_empty()),
            icon,
        }
    }
}

/// Icon displayed for a map marker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarkerIcon {
    Pin,
    Cross,
    Circle,
    Lightning,
    Warning,
    Error,
    Check,
    Satellite,
    SatelliteLost,
    Gear,
    Refresh,
    Download,
    Upload,
    Wrench,
}

impl MarkerIcon {
    /// Stable u8 encoding written to the `markers/icon` dataset.
    pub(crate) fn to_u8(self) -> u8 {
        match self {
            MarkerIcon::Pin => 0,
            MarkerIcon::Cross => 1,
            MarkerIcon::Circle => 2,
            MarkerIcon::Lightning => 3,
            MarkerIcon::Warning => 4,
            MarkerIcon::Error => 5,
            MarkerIcon::Check => 6,
            MarkerIcon::Satellite => 7,
            MarkerIcon::SatelliteLost => 8,
            MarkerIcon::Gear => 9,
            MarkerIcon::Refresh => 10,
            MarkerIcon::Download => 11,
            MarkerIcon::Upload => 12,
            MarkerIcon::Wrench => 13,
        }
    }

    pub(crate) fn from_u8(code: u8) -> Self {
        match code {
            1 => MarkerIcon::Cross,
            2 => MarkerIcon::Circle,
            3 => MarkerIcon::Lightning,
            4 => MarkerIcon::Warning,
            5 => MarkerIcon::Error,
            6 => MarkerIcon::Check,
            7 => MarkerIcon::Satellite,
            8 => MarkerIcon::SatelliteLost,
            9 => MarkerIcon::Gear,
            10 => MarkerIcon::Refresh,
            11 => MarkerIcon::Download,
            12 => MarkerIcon::Upload,
            13 => MarkerIcon::Wrench,
            _ => MarkerIcon::Pin,
        }
    }

    pub(crate) fn name(self) -> &'static str {
        match self {
            MarkerIcon::Pin => "pin",
            MarkerIcon::Cross => "cross",
            MarkerIcon::Circle => "circle",
            MarkerIcon::Lightning => "lightning",
            MarkerIcon::Warning => "warning",
            MarkerIcon::Error => "error",
            MarkerIcon::Check => "check",
            MarkerIcon::Satellite => "satellite",
            MarkerIcon::SatelliteLost => "satellite_lost",
            MarkerIcon::Gear => "gear",
            MarkerIcon::Refresh => "refresh",
            MarkerIcon::Download => "download",
            MarkerIcon::Upload => "upload",
            MarkerIcon::Wrench => "wrench",
        }
    }

    pub fn try_from_lower_case(s: impl AsRef<str>) -> Result<Self, Error> {
        let s = s.as_ref();
        match s {
            "pin" => Ok(MarkerIcon::Pin),
            "cross" => Ok(MarkerIcon::Cross),
            "circle" => Ok(MarkerIcon::Circle),
            "lightning" => Ok(MarkerIcon::Lightning),
            "warning" => Ok(MarkerIcon::Warning),
            "error" => Ok(MarkerIcon::Error),
            "check" => Ok(MarkerIcon::Check),
            "satellite" => Ok(MarkerIcon::Satellite),
            "satellite_lost" => Ok(MarkerIcon::SatelliteLost),
            "gear" => Ok(MarkerIcon::Gear),
            "refresh" => Ok(MarkerIcon::Refresh),
            "download" => Ok(MarkerIcon::Download),
            "upload" => Ok(MarkerIcon::Upload),
            "wrench" => Ok(MarkerIcon::Wrench),
            _ => Err(Error::UnknownMarkerIcon { name: s.to_owned() }),
        }
    }
}

/// Optional file-level metadata.
#[derive(Debug, Clone, Default)]
pub struct Meta {
    pub title: Option<String>,
    /// Sensor or device that produced the data.
    pub device: Option<String>,
    /// Free-text notes.
    pub notes: Option<String>,
    /// Stable grouping key used by the app's history database.
    ///
    /// When set, all recordings with the same identity string are stored under
    /// the same group in the database and appear together in the History window.
    pub identity: Option<String>,
}

#[bon::bon]
impl Meta {
    /// Build a new [`Meta`] object.
    ///
    /// Empty or whitespace-only strings are automatically converted to `None`.
    #[builder(finish_fn = build)]
    pub fn new(
        #[builder(into)] title: Option<String>,
        #[builder(into)] device: Option<String>,
        #[builder(into)] notes: Option<String>,
        #[builder(into)] identity: Option<String>,
    ) -> Self {
        Self {
            title: title.filter(|s| !s.trim().is_empty()),
            device: device.filter(|s| !s.trim().is_empty()),
            notes: notes.filter(|s| !s.trim().is_empty()),
            identity: identity.filter(|s| !s.trim().is_empty()),
        }
    }
}

impl NavFix {
    /// The best available timestamp for this fix.
    ///
    /// Returns `gps_time` when the receiver had an active lock, otherwise falls
    /// back to `sys_time`, then to the Unix epoch as a last resort.
    /// Use this instead of accessing `gps_time` directly when you need a
    /// concrete timestamp regardless of whether a GPS lock was present.
    pub fn effective_gps_time(&self) -> DateTime<Utc> {
        self.gps_time.or(self.sys_time).unwrap_or_default()
    }
}

/// A nav fix combined with its associated satellite report (if any).
#[derive(Debug, Clone)]
pub struct NavPoint {
    pub fix: NavFix,
    pub satellites: Option<SatelliteReport>,
}

/// A map annotation with its interpolated position on the nav track.
#[derive(Debug, Clone)]
pub struct Marker {
    pub annotation: Annotation,
    /// Latitude interpolated from the surrounding nav fixes.
    pub lat: Angle,
    /// Longitude interpolated from the surrounding nav fixes.
    pub lon: Angle,
}

/// An event marker to add to the nav file.
///
/// The builder computes the geographic position by interpolating surrounding
/// nav fixes; producers supply only a timestamp.
///
/// Construct via `EventMarker::builder().build()` - it validates the variant path and returns
/// `Err` immediately if it is malformed.
#[derive(Debug, Clone)]
pub struct EventMarker {
    pub(crate) variant_path: String,
    pub(crate) sys_time: chrono::DateTime<chrono::Utc>,
    pub(crate) annotation: Option<String>,
}

#[bon::bon]
impl EventMarker {
    /// Build a validated [`EventMarker`].
    ///
    /// Returns `Err` if `variant_path` is malformed (empty, leading/trailing slash,
    /// consecutive slashes, non-ASCII-alphanumeric/hyphen/underscore, or > 256 bytes).
    #[builder(finish_fn = build)]
    pub fn new(
        #[builder(into)] variant_path: String,
        #[builder(into)] sys_time: chrono::DateTime<chrono::Utc>,
        #[builder(into)] annotation: Option<String>,
    ) -> Result<Self, EventMarkerError> {
        crate::error::validate_variant_path(&variant_path)?;
        Ok(Self {
            variant_path,
            sys_time,
            annotation,
        })
    }
}

impl EventMarker {
    pub fn variant_path(&self) -> &str {
        &self.variant_path
    }

    pub fn sys_time(&self) -> chrono::DateTime<chrono::Utc> {
        self.sys_time
    }

    pub fn annotation(&self) -> Option<&str> {
        self.annotation.as_deref()
    }
}

/// Fill color for an event marker variant.
///
/// `Auto` resolves to a deterministic hash color derived from the variant path,
/// so unstyled variants still get visually distinct, consistent colors.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum EventMarkerColor {
    /// Use the deterministic djb2 hash color for this variant path.
    #[default]
    Auto,
    /// Explicit `#RRGGBB` hex color, e.g. `"#FF9900"`.
    Hex(String),
}

impl EventMarkerColor {
    /// Construct an explicit hex color without requiring `.to_owned()` at the call site.
    pub fn hex(s: impl Into<String>) -> Self {
        Self::Hex(s.into())
    }
}

impl TryFrom<String> for EventMarkerColor {
    type Error = Error;

    fn try_from(s: String) -> Result<Self, Self::Error> {
        if s.is_empty() {
            return Ok(Self::Auto);
        }
        if s.len() == 7 && s.starts_with('#') && s.chars().skip(1).all(|c| c.is_ascii_hexdigit()) {
            Ok(Self::Hex(s))
        } else {
            Err(Error::ParseError {
                unit: "EventMarkerColor (hex)",
                input: s,
                reason: "expected #RRGGBB format".to_owned(),
            })
        }
    }
}

/// Icon shape for an event marker variant.
///
/// `Auto` resolves to `MarkerIcon::Pin` when the file is loaded.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum EventMarkerIconChoice {
    /// Use the application default (currently `MarkerIcon::Pin`).
    #[default]
    Auto,
    /// Explicit icon choice.
    Icon(MarkerIcon),
}

impl From<MarkerIcon> for EventMarkerIconChoice {
    fn from(icon: MarkerIcon) -> Self {
        Self::Icon(icon)
    }
}

impl From<Option<MarkerIcon>> for EventMarkerIconChoice {
    fn from(icon: Option<MarkerIcon>) -> Self {
        icon.map_or(Self::Auto, Self::Icon)
    }
}

/// Per-variant icon and color override stored in the file.
#[derive(Debug, Clone)]
pub struct EventMarkerStyle {
    /// Must exactly match a `variant_path` used in the event markers.
    pub variant_path: String,
    /// Icon shape; defaults to `Auto` (Pin).
    pub icon: EventMarkerIconChoice,
    /// Fill color; defaults to `Auto` (hash-derived from the variant path).
    pub color: EventMarkerColor,
}

#[bon::bon]
impl EventMarkerStyle {
    /// Build a new [`EventMarkerStyle`].
    ///
    /// Empty or whitespace-only colors are automatically converted to `Auto`.
    #[builder(finish_fn = build)]
    pub fn new(
        #[builder(into)] variant_path: String,
        icon: Option<EventMarkerIconChoice>,
        #[builder(into)] color: Option<String>,
    ) -> Result<Self, Error> {
        Ok(Self {
            variant_path,
            icon: icon.unwrap_or_default(),
            color: color
                .filter(|s| !s.trim().is_empty())
                .map(EventMarkerColor::try_from)
                .transpose()?
                .unwrap_or_default(),
        })
    }
}

/// An event marker after position interpolation, stored in [`NavFile`].
#[derive(Debug, Clone)]
pub struct EventMarkerPoint {
    pub variant_path: String,
    pub sys_time: chrono::DateTime<chrono::Utc>,
    pub lat: Angle,
    pub lon: Angle,
    pub annotation: Option<String>,
}

/// A complete, validated GeoTrace data file ready for serialisation.
///
/// Construct via [`NavFileSink::finish`](crate::NavFileSink::finish).
#[derive(Debug, Clone)]
pub struct NavFile {
    pub(crate) meta: Meta,
    pub(crate) nav_points: Vec<NavPoint>,
    pub(crate) markers: Vec<Marker>,
    pub(crate) event_markers: Vec<EventMarkerPoint>,
    pub(crate) event_marker_styles: Vec<EventMarkerStyle>,
}

impl NavFile {
    pub fn meta(&self) -> &Meta {
        &self.meta
    }

    pub fn nav_points(&self) -> &[NavPoint] {
        &self.nav_points
    }

    pub fn markers(&self) -> &[Marker] {
        &self.markers
    }

    pub fn event_markers(&self) -> &[EventMarkerPoint] {
        &self.event_markers
    }

    pub fn event_marker_styles(&self) -> &[EventMarkerStyle] {
        &self.event_marker_styles
    }

    /// Serialise the file to the provided writer.
    pub fn write<W: io::Write>(&self, mut writer: W) -> Result<(), crate::error::Error> {
        let bytes = crate::write::build_hdf5(self)?;
        writer.write_all(&bytes)?;
        Ok(())
    }

    /// Write to a file at `path`. Appends `.gtd` if `path` has no extension.
    pub fn write_to_file(&self, path: impl AsRef<Path>) -> Result<(), crate::error::Error> {
        let path = path.as_ref();
        let dest = if path.extension().is_none() {
            path.with_extension("gtd")
        } else {
            path.to_path_buf()
        };
        self.write(File::create(dest)?)
    }

    /// Read a `.gtd` file from the provided reader.
    pub fn read<R: io::Read>(mut reader: R) -> Result<Self, crate::error::Error> {
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes)?;
        crate::read::parse_hdf5(bytes)
    }

    /// Open a `.gtd` file at `path` and parse it.
    pub fn open(path: impl AsRef<std::path::Path>) -> Result<Self, crate::error::Error> {
        Self::read(File::open(path)?)
    }

    /// Pretty-print a summary of a `.gtd` file at the given path.
    pub fn inspect(path: impl AsRef<std::path::Path>) -> Result<String, crate::error::Error> {
        crate::read::inspect_path(path.as_ref())
    }
}

use std::{fs::File, io, path::Path};

use chrono::{DateTime, Utc};
use geotrace_sdk_units::{ChannelUnit, PhysicalQuantity};

use crate::error::{ChannelError, Error, EventMarkerError, MARKER_LABEL_LOCATION};
use crate::fixed_width_string::{AnnotationField, MarkerLabelField};
use crate::provenance;
use crate::{Angle, Velocity};

/// The clock or clocks that stamped a nav fix or a satellite report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavFixTime {
    /// The receiver's timestamp, with no host clock recorded.
    Receiver(DateTime<Utc>),
    /// The host clock's timestamp, taken while the receiver had no lock.
    Host(DateTime<Utc>),
    /// Both timestamps, recorded under lock on a host that also stamped it.
    Both {
        gps: DateTime<Utc>,
        sys: DateTime<Utc>,
    },
}

/// The two timestamps a recorder holds for one fix or satellite report, either
/// of which may be absent. A caller cannot transpose the two clocks: each has
/// its own field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecordedFixTimestamps {
    pub gps: Option<DateTime<Utc>>,
    pub sys: Option<DateTime<Utc>>,
}

impl NavFixTime {
    /// `None` when the recorder holds neither timestamp.
    pub fn from_recorded(
        RecordedFixTimestamps { gps, sys }: RecordedFixTimestamps,
    ) -> Option<Self> {
        match (gps, sys) {
            (Some(gps), Some(sys)) => Some(Self::Both { gps, sys }),
            (Some(gps), None) => Some(Self::Receiver(gps)),
            (None, Some(sys)) => Some(Self::Host(sys)),
            (None, None) => None,
        }
    }

    pub fn gps_time(self) -> Option<DateTime<Utc>> {
        match self {
            Self::Receiver(gps) | Self::Both { gps, .. } => Some(gps),
            Self::Host(_) => None,
        }
    }

    pub fn sys_time(self) -> Option<DateTime<Utc>> {
        match self {
            Self::Host(sys) | Self::Both { sys, .. } => Some(sys),
            Self::Receiver(_) => None,
        }
    }

    /// The receiver's timestamp where the fix has one, else the host clock's.
    pub fn effective(self) -> DateTime<Utc> {
        match self {
            Self::Receiver(gps) | Self::Both { gps, .. } => gps,
            Self::Host(sys) => sys,
        }
    }
}

/// A single GPS/GNSS fix: position, heading, and optional speed at a point in time.
///
/// `heading` is `None` for synthetic/ghost fixes where the actual direction is
/// unknown (e.g., dead-reckoned positions emitted only to carry satellite reports).
/// The app renders those as circles.
///
/// The builder measures the GPS/system-clock delta from a [`NavFixTime::Both`]
/// fix. With that delta it converts a satellite report's system-clock timestamp
/// into the GPS time domain, and interpolates ghost fixes through a no-fix
/// period.
///
/// The ranges stated on the fields below are data quality expectations, not
/// parse rules.
/// [`NavFile::read`] returns a latitude or longitude the file holds unchanged,
/// NaN included.
/// A NaN `heading`, `speed` or `eph_m` reads back as `None`: NaN is how the
/// write path stores an absent one.
/// The builder writes every value it is given: a recorder that captured bad
/// data must be able to write it.
/// Checking a value against its range is the consumer's job.
///
/// A fix without a timestamp does not compile:
///
/// ```compile_fail
/// use geotrace_sdk::{Angle, NavFix};
///
/// let fix = NavFix::builder()
///     .lat(Angle::degrees(51.5))
///     .lon(Angle::degrees(-0.1))
///     .build();
/// ```
#[derive(bon::Builder, Debug, Clone, Copy, PartialEq)]
pub struct NavFix {
    pub time: NavFixTime,
    /// WGS-84 latitude, expected in \[-90°, 90°].
    pub lat: Angle,
    /// WGS-84 longitude, expected in \[-180°, 180°].
    pub lon: Angle,
    /// Compass heading, expected in \[0°, 360°). `None` = unknown direction
    /// (ghost fix).
    pub heading: Option<Angle>,
    /// Ground speed, expected to be non-negative.
    pub speed: Option<Velocity>,
    /// Estimated horizontal position accuracy in metres, as reported by the GPS
    /// receiver, expected to be non-negative.
    ///
    /// The app renders a translucent blue circle of this radius around the point when
    /// present. `None` when the receiver did not report an accuracy estimate.
    pub eph_m: Option<f64>,
}

/// A satellite visibility report captured at a point in time.
///
/// The builder places a [`NavFixTime::Host`] report in the GPS time domain with
/// the GPS/system-clock delta it measures from the surrounding nav fixes.
///
/// A report without a timestamp does not compile:
///
/// ```compile_fail
/// use geotrace_sdk::SatelliteReport;
///
/// let report = SatelliteReport::builder().tracked(Vec::new()).build();
/// ```
#[derive(bon::Builder, Debug, Clone, PartialEq)]
pub struct SatelliteReport {
    pub time: NavFixTime,
    /// All satellites currently tracked (may include satellites not in the fix).
    pub tracked: Vec<Satellite>,
}

/// One tracked satellite with optional signal metrics.
#[derive(bon::Builder, Debug, Clone, Copy, PartialEq)]
pub struct Satellite {
    #[builder(into)]
    pub constellation: Constellation,
    pub prn: u32,
    /// Whether this satellite is contributing to the current positional fix.
    #[builder(default)]
    pub in_fix: bool,
    /// Elevation above horizon in degrees. `None` if unavailable.
    pub elevation: Option<f32>,
    /// Azimuth from true north in degrees. `None` if unavailable.
    pub azimuth: Option<f32>,
    /// Signal-to-noise ratio in dB-Hz.
    ///
    /// An unavailable SNR is `None`. Never encode one as `0.0`, which readers
    /// take as a measured 0 dB-Hz, nor as a sentinel value such as 99 dB-Hz.
    pub snr: Option<f32>,
}

/// The SNR some receiver firmware reports in place of a measurement, in dB-Hz.
const SNR_NO_DATA_SENTINEL_DB_HZ: f32 = 99.0;

/// How far from [`SNR_NO_DATA_SENTINEL_DB_HZ`] a reported SNR still counts as
/// the sentinel value, in dB-Hz.
const SNR_NO_DATA_SENTINEL_TOLERANCE_DB_HZ: f32 = 0.5;

impl Satellite {
    /// Whether `snr` holds ≈99 dB-Hz, the sentinel value firmware reports for "no data".
    ///
    /// The SDK reads and writes the value unchanged and only counts it among
    /// [`crate::NavFileBuilder`]'s satellite warnings: interpreting it is left
    /// to the caller.
    pub fn snr_is_no_data_sentinel(&self) -> bool {
        self.snr.is_some_and(|snr| {
            (snr - SNR_NO_DATA_SENTINEL_DB_HZ).abs() < SNR_NO_DATA_SENTINEL_TOLERANCE_DB_HZ
        })
    }
}

/// GNSS constellation identifier.
///
/// `EnumString` (via `strum`) gives the lowercase wire form used by
/// [`Constellation::try_from_lower_case`], derived from the variant names so it
/// can't desync from them. [`Constellation::display_name`] is a separate,
/// deliberately-exhaustive `match` - it is the single place that fixes the
/// "BeiDou" vs "Beidou" vs "BEIDOU" spelling, and the compiler forces
/// it to be updated whenever a variant is added, unlike a derived
/// `#[strum(message = ...)]` (which would silently fall back to `None` instead).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, strum::EnumString, strum::EnumIter, strum::EnumCount,
)]
#[strum(serialize_all = "lowercase")]
pub enum Constellation {
    Gps,
    Glonass,
    Galileo,
    Beidou,
    Navic,
    Qzss,
}

impl Constellation {
    /// Stable u8 encoding written to the `tracked_sats/constellation` dataset.
    pub(crate) fn to_u8(self) -> u8 {
        match self {
            Constellation::Gps => 0,
            Constellation::Glonass => 1,
            Constellation::Galileo => 2,
            Constellation::Beidou => 3,
            Constellation::Navic => 4,
            Constellation::Qzss => 5,
        }
    }

    pub(crate) fn from_u8(code: u8, dataset: &'static str) -> Result<Self, Error> {
        match code {
            0 => Ok(Constellation::Gps),
            1 => Ok(Constellation::Glonass),
            2 => Ok(Constellation::Galileo),
            3 => Ok(Constellation::Beidou),
            4 => Ok(Constellation::Navic),
            5 => Ok(Constellation::Qzss),
            _ => Err(Error::UnknownConstellation {
                code: i16::from(code),
                dataset,
            }),
        }
    }

    /// Canonical human-readable name, e.g. `Constellation::Beidou.display_name() == "BeiDou"`.
    ///
    /// Single source of truth for the constellation's display spelling - every
    /// other call site (UI labels, `read::constellation_names`, Python bindings)
    /// formats through this.
    pub fn display_name(self) -> &'static str {
        match self {
            Constellation::Gps => "GPS",
            Constellation::Glonass => "GLONASS",
            Constellation::Galileo => "Galileo",
            Constellation::Beidou => "BeiDou",
            Constellation::Navic => "NavIC",
            Constellation::Qzss => "QZSS",
        }
    }

    /// Parses the lowercase wire form, e.g. `"beidou"`.
    pub fn try_from_lower_case(s: impl AsRef<str>) -> Result<Self, Error> {
        let s = s.as_ref();
        // strum::ParseError carries no information beyond "no variant matched" -
        // not worth threading through as a `source`.
        s.parse()
            .map_err(|_err: strum::ParseError| Error::UnknownConstellationName {
                name: s.to_owned(),
            })
    }
}

/// A user-defined map annotation with an optional label and an icon.
///
/// `Annotation::builder().build()` is the only way in, and it rejects a label
/// past the capacity of `markers/label`.
#[derive(Debug, Clone, PartialEq)]
pub struct Annotation {
    pub(crate) label: Option<String>,
    pub(crate) icon: MarkerIcon,
    pub(crate) time: DateTime<Utc>,
}

#[bon::bon]
impl Annotation {
    /// Build a validated [`Annotation`].
    ///
    /// An empty or whitespace-only `label` becomes `None`. Returns `Err` if
    /// `label` does not fit a [`MarkerLabelField`](crate::MarkerLabelField).
    #[builder(finish_fn = build)]
    pub fn new(
        #[builder(into)] time: DateTime<Utc>,
        #[builder(into)] label: Option<String>,
        #[builder(default = MarkerIcon::Pin)] icon: MarkerIcon,
    ) -> Result<Self, Error> {
        let label = label.filter(|s| !s.trim().is_empty());
        if let Some(label) = label.as_deref() {
            MarkerLabelField::new(label)
                .map_err(|source| Error::unwritable_field(MARKER_LABEL_LOCATION, source))?;
        }
        Ok(Self { time, label, icon })
    }
}

impl Annotation {
    /// `None` for an annotation the map renders unlabelled.
    pub fn label(&self) -> Option<&str> {
        self.label.as_deref()
    }

    pub fn icon(&self) -> MarkerIcon {
        self.icon
    }

    pub fn time(&self) -> DateTime<Utc> {
        self.time
    }
}

/// Icon displayed for a map marker.
///
/// `Display`/`FromStr` (via `strum`) give the lower `snake_case` wire form used by
/// `MarkerIcon::name` and [`MarkerIcon::try_from_lower_case`] - the variant name
/// and its string form are derived from a single definition, so adding, renaming,
/// or removing a variant cannot desync the two.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    strum::Display,
    strum::EnumString,
    strum::IntoStaticStr,
    strum::EnumCount,
    strum::EnumIter,
)]
#[strum(serialize_all = "snake_case")]
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

    /// Lower `snake_case` wire form, e.g. `MarkerIcon::SatelliteLost.name() == "satellite_lost"`.
    ///
    /// Inverse of [`MarkerIcon::try_from_lower_case`]. Both are derived from the
    /// variant names via `strum`, so they always agree.
    pub(crate) fn name(self) -> &'static str {
        self.into()
    }

    /// Parses the lower `snake_case` wire form produced by `MarkerIcon::name`.
    pub fn try_from_lower_case(s: impl AsRef<str>) -> Result<Self, Error> {
        let s = s.as_ref();
        // strum::ParseError carries no information beyond "no variant matched" -
        // not worth threading through as a `source`.
        s.parse()
            .map_err(|_err: strum::ParseError| Error::UnknownMarkerIcon { name: s.to_owned() })
    }
}

#[cfg(test)]
mod marker_icon_tests {
    use strum::IntoEnumIterator;

    use super::*;

    /// `name()`/`try_from_lower_case` are derived from the same variant list, so
    /// every variant must round-trip through its wire form back to itself.
    #[test]
    fn name_and_try_from_lower_case_round_trip() {
        for icon in MarkerIcon::iter() {
            let wire = icon.name();
            let parsed = MarkerIcon::try_from_lower_case(wire)
                .unwrap_or_else(|err| panic!("{wire:?} should parse back to {icon:?}: {err}"));
            assert_eq!(parsed, icon);
        }
    }

    /// The wire form is part of the on-disk `.gtd` format
    /// (`Annotation::icon`/`EventMarkerIconChoice::Icon`). Pin it down so a
    /// rename of a variant - which would silently change `strum`'s derived
    /// `snake_case` form - is caught by this test.
    #[test]
    fn name_is_stable_wire_form() {
        assert_eq!(MarkerIcon::Pin.name(), "pin");
        assert_eq!(MarkerIcon::SatelliteLost.name(), "satellite_lost");
        assert_eq!(MarkerIcon::Wrench.name(), "wrench");
    }

    #[test]
    fn try_from_lower_case_rejects_unknown_strings() {
        let err = MarkerIcon::try_from_lower_case("not_an_icon").unwrap_err();
        assert!(matches!(err, Error::UnknownMarkerIcon { name } if name == "not_an_icon"));
    }
}

#[cfg(test)]
mod travel_mode_tests {
    use strum::{EnumCount, IntoEnumIterator};

    use super::*;

    /// `name()` is a hand-written match (it must borrow from `Unknown`), while
    /// `Display` and `from_lower_case` are strum-derived. Round-tripping every
    /// variant through both pins the three representations together.
    #[test]
    fn name_display_and_from_lower_case_agree() {
        for mode in TravelMode::iter() {
            assert_eq!(mode.name(), mode.to_string());
            assert_eq!(TravelMode::from_lower_case(mode.name()), mode);
        }
    }

    /// The wire form is part of the on-disk `.gtd` format (`meta_travel_mode`).
    /// Pin it down so a variant rename - which would silently change `strum`'s
    /// derived `snake_case` form - is caught by this test.
    #[test]
    fn name_is_stable_wire_form() {
        let known = [
            (TravelMode::Car, "car"),
            (TravelMode::Motorcycle, "motorcycle"),
            (TravelMode::Bicycle, "bicycle"),
            (TravelMode::Pedestrian, "pedestrian"),
            (TravelMode::Boat, "boat"),
            (TravelMode::Rail, "rail"),
            (TravelMode::Aircraft, "aircraft"),
        ];
        // Every variant except the `Unknown` carrier must appear in the table.
        assert_eq!(known.len(), TravelMode::COUNT - 1);
        for (mode, wire) in known {
            assert_eq!(mode.name(), wire);
        }
    }

    #[test]
    fn unknown_values_are_preserved_verbatim() {
        let mode = TravelMode::from_lower_case("hovercraft");
        assert_eq!(mode, TravelMode::Unknown("hovercraft".into()));
        assert_eq!(mode.name(), "hovercraft");
    }
}

#[cfg(test)]
mod constellation_tests {
    use strum::IntoEnumIterator;

    use super::*;

    /// `try_from_lower_case` is derived from the variant list (via `EnumString`),
    /// so every variant's lowercase wire form must parse back to itself.
    #[test]
    fn try_from_lower_case_round_trips_through_display_name() {
        for c in Constellation::iter() {
            let lower = c.display_name().to_lowercase();
            let parsed = Constellation::try_from_lower_case(&lower)
                .unwrap_or_else(|err| panic!("{lower:?} should parse back to {c:?}: {err}"));
            assert_eq!(parsed, c);
        }
    }

    /// The display name is the single source of truth for the "BeiDou" vs
    /// "Beidou" vs "BEIDOU" spelling that was previously re-typed independently
    /// at every call site (UI labels, `read::constellation_names`, Python
    /// bindings). Pin it down so a future edit has to change it here.
    #[test]
    fn display_name_is_canonical_spelling() {
        assert_eq!(Constellation::Gps.display_name(), "GPS");
        assert_eq!(Constellation::Glonass.display_name(), "GLONASS");
        assert_eq!(Constellation::Galileo.display_name(), "Galileo");
        assert_eq!(Constellation::Beidou.display_name(), "BeiDou");
        assert_eq!(Constellation::Navic.display_name(), "NavIC");
        assert_eq!(Constellation::Qzss.display_name(), "QZSS");
    }

    /// The lowercase wire form is part of the on-disk `.gtd` format
    /// (`tracked_sats/constellation` group attributes). Pin it down so a rename
    /// of a variant - which would silently change `strum`'s derived lowercase
    /// form - is caught by this test.
    #[test]
    fn try_from_lower_case_accepts_stable_wire_form() {
        for (lower, expected) in [
            ("gps", Constellation::Gps),
            ("glonass", Constellation::Glonass),
            ("galileo", Constellation::Galileo),
            ("beidou", Constellation::Beidou),
            ("navic", Constellation::Navic),
            ("qzss", Constellation::Qzss),
        ] {
            assert_eq!(Constellation::try_from_lower_case(lower).unwrap(), expected);
        }
    }

    #[test]
    fn try_from_lower_case_rejects_unknown_strings() {
        let err = Constellation::try_from_lower_case("not_a_constellation").unwrap_err();
        assert!(
            matches!(err, Error::UnknownConstellationName { name } if name == "not_a_constellation")
        );
    }

    /// `to_u8`/`from_u8` are the on-disk binary codes in the `.gtd`
    /// `tracked_sats/constellation` dataset - the highest-consequence mapping
    /// for this type. Pin the exact codes (a wrong number silently corrupts
    /// files) and assert the table is exhaustive against `COUNT`, then check
    /// every variant roundtrips so no new variant can lack a `from_u8` arm.
    #[test]
    fn u8_wire_codes_are_stable_and_round_trip() {
        use strum::{EnumCount, IntoEnumIterator};
        let expected = [
            (Constellation::Gps, 0u8),
            (Constellation::Glonass, 1),
            (Constellation::Galileo, 2),
            (Constellation::Beidou, 3),
            (Constellation::Navic, 4),
            (Constellation::Qzss, 5),
        ];
        assert_eq!(expected.len(), Constellation::COUNT);
        for (c, code) in expected {
            assert_eq!(c.to_u8(), code, "{c:?} wire code");
            assert_eq!(Constellation::from_u8(code, "test").unwrap(), c);
        }
        for c in Constellation::iter() {
            assert_eq!(Constellation::from_u8(c.to_u8(), "test").unwrap(), c);
        }
    }
}

/// Platform a recording was made on, declared by the recorder.
///
/// The field describes what carried the receiver, not how an application
/// should process the data - consumers derive their own behavior from it
/// (the GeoTrace app, for example, picks a snap-to-road costing).
///
/// `Display`/`FromStr` (via `strum`) give the lower `snake_case` wire form used
/// by [`TravelMode::name`] and [`TravelMode::from_lower_case`]. Wire values
/// outside the known set parse into [`TravelMode::Unknown`] so they survive a
/// read-write round trip.
#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    strum::Display,
    strum::EnumString,
    strum::EnumCount,
    strum::EnumIter,
)]
#[strum(serialize_all = "snake_case")]
pub enum TravelMode {
    Car,
    Motorcycle,
    Bicycle,
    Pedestrian,
    Boat,
    Rail,
    Aircraft,
    /// A wire value not in the known set, preserved verbatim.
    #[strum(default)]
    Unknown(String),
}

impl TravelMode {
    /// Lower `snake_case` wire form, e.g. `TravelMode::Car.name() == "car"`.
    ///
    /// For [`TravelMode::Unknown`] this is the preserved original wire value.
    /// Inverse of [`TravelMode::from_lower_case`].
    pub fn name(&self) -> &str {
        match self {
            TravelMode::Car => "car",
            TravelMode::Motorcycle => "motorcycle",
            TravelMode::Bicycle => "bicycle",
            TravelMode::Pedestrian => "pedestrian",
            TravelMode::Boat => "boat",
            TravelMode::Rail => "rail",
            TravelMode::Aircraft => "aircraft",
            TravelMode::Unknown(raw) => raw,
        }
    }

    /// Parses the lower `snake_case` wire form produced by [`TravelMode::name`].
    ///
    /// Never fails: values outside the known set become
    /// [`TravelMode::Unknown`], preserving the input verbatim.
    pub fn from_lower_case(s: impl AsRef<str>) -> Self {
        let s = s.as_ref();
        match s.parse() {
            Ok(mode) => mode,
            // `#[strum(default)]` makes parsing infallible. The explicit
            // fallback keeps that true if the default is ever removed.
            Err(_) => TravelMode::Unknown(s.to_owned()),
        }
    }
}

/// Optional file-level metadata.
#[derive(Debug, Clone, Default, PartialEq)]
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
    /// Platform the recording was made on.
    pub travel_mode: Option<TravelMode>,
    pub(crate) sdk_version: Option<String>,
    pub(crate) sdk_git_commit: Option<String>,
    pub(crate) sdk_commit_time: Option<DateTime<Utc>>,
}

impl Meta {
    /// Version of the SDK build that produced the file.
    pub fn sdk_version(&self) -> Option<&str> {
        self.sdk_version.as_deref()
    }

    /// Commit of the GeoTrace repository the writing SDK was built from.
    pub fn sdk_git_commit(&self) -> Option<&str> {
        self.sdk_git_commit.as_deref()
    }

    /// Committer timestamp of [`Meta::sdk_git_commit`], in UTC.
    pub fn sdk_commit_time(&self) -> Option<DateTime<Utc>> {
        self.sdk_commit_time
    }

    pub(crate) fn stamp_this_build(&mut self) {
        self.sdk_version = Some(crate::VERSION.to_owned());
        self.sdk_git_commit = provenance::PROVENANCE.map(|p| p.commit.to_owned());
        self.sdk_commit_time = provenance::commit_time();
    }

    pub(crate) fn stamp_scrubbed_provenance(&mut self) {
        self.sdk_version = Some(provenance::SCRUBBED_SDK_VERSION.to_owned());
        self.sdk_git_commit = None;
        self.sdk_commit_time = None;
    }

    fn equals_ignoring_build_provenance(&self, other: &Self) -> bool {
        let Self {
            title,
            device,
            notes,
            identity,
            travel_mode,
            sdk_version: _,
            sdk_git_commit: _,
            sdk_commit_time: _,
        } = self;
        *title == other.title
            && *device == other.device
            && *notes == other.notes
            && *identity == other.identity
            && *travel_mode == other.travel_mode
    }
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
        travel_mode: Option<TravelMode>,
    ) -> Self {
        Self {
            title: title.filter(|s| !s.trim().is_empty()),
            device: device.filter(|s| !s.trim().is_empty()),
            notes: notes.filter(|s| !s.trim().is_empty()),
            identity: identity.filter(|s| !s.trim().is_empty()),
            travel_mode,
            sdk_version: None,
            sdk_git_commit: None,
            sdk_commit_time: None,
        }
    }
}

impl NavFix {
    pub fn gps_time(&self) -> Option<DateTime<Utc>> {
        self.time.gps_time()
    }

    pub fn sys_time(&self) -> Option<DateTime<Utc>> {
        self.time.sys_time()
    }

    /// The receiver's timestamp where the fix has one, else the host clock's.
    pub fn effective_gps_time(&self) -> DateTime<Utc> {
        self.time.effective()
    }
}

impl SatelliteReport {
    pub fn gps_time(&self) -> Option<DateTime<Utc>> {
        self.time.gps_time()
    }

    pub fn sys_time(&self) -> Option<DateTime<Utc>> {
        self.time.sys_time()
    }
}

/// A nav fix combined with its associated satellite report (if any).
#[derive(Debug, Clone, PartialEq)]
pub struct NavPoint {
    pub fix: NavFix,
    pub satellites: Option<SatelliteReport>,
}

/// A map annotation with its interpolated position on the nav track.
#[derive(Debug, Clone, PartialEq)]
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
/// nav fixes. Producers supply only a timestamp.
///
/// Construct via `EventMarker::builder().build()` - it returns `Err` immediately
/// for a malformed variant path and for an annotation its field cannot hold.
#[derive(Debug, Clone, PartialEq)]
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
    /// consecutive slashes, non-ASCII-alphanumeric/hyphen/underscore, or longer than
    /// [`VariantPathField::CONTENT_CAPACITY`](crate::VariantPathField::CONTENT_CAPACITY)
    /// bytes), and if `annotation` does not fit an [`AnnotationField`].
    #[builder(finish_fn = build)]
    pub fn new(
        #[builder(into)] variant_path: String,
        #[builder(into)] sys_time: chrono::DateTime<chrono::Utc>,
        #[builder(into)] annotation: Option<String>,
    ) -> Result<Self, EventMarkerError> {
        crate::error::validate_variant_path(&variant_path)?;
        if let Some(annotation) = annotation.as_deref() {
            AnnotationField::new(annotation)
                .map_err(|source| EventMarkerError::UnwritableAnnotation { source })?;
        }
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
    /// A `color_hex` wire value that is not the `#RRGGBB` form, read from a
    /// file and preserved verbatim.
    Unrecognized(String),
}

impl EventMarkerColor {
    /// Construct an explicit hex color without requiring `.to_owned()` at the call site.
    pub fn hex(s: impl Into<String>) -> Self {
        Self::Hex(s.into())
    }

    /// Reads a `color_hex` wire value: an empty value gives
    /// [`EventMarkerColor::Auto`], the `#RRGGBB` form
    /// [`EventMarkerColor::Hex`], and any other value
    /// [`EventMarkerColor::Unrecognized`].
    pub fn from_wire_value(s: impl Into<String>) -> Self {
        let s = s.into();
        if s.is_empty() {
            Self::Auto
        } else if s.len() == 7
            && s.starts_with('#')
            && s.chars().skip(1).all(|c| c.is_ascii_hexdigit())
        {
            Self::Hex(s)
        } else {
            Self::Unrecognized(s)
        }
    }
}

impl TryFrom<String> for EventMarkerColor {
    type Error = Error;

    fn try_from(s: String) -> Result<Self, Self::Error> {
        match Self::from_wire_value(s) {
            Self::Unrecognized(input) => Err(Error::ParseError {
                unit: "EventMarkerColor (hex)",
                input,
                reason: "expected #RRGGBB format".to_owned(),
            }),
            color => Ok(color),
        }
    }
}

/// Icon shape for an event marker variant.
///
/// `Auto` resolves to `MarkerIcon::Pin` when the file is loaded.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum EventMarkerIconChoice {
    /// Use the application default (currently `MarkerIcon::Pin`).
    #[default]
    Auto,
    /// Explicit icon choice.
    Icon(MarkerIcon),
    /// An `icon_name` wire value outside the [`MarkerIcon`] set, read from a
    /// file and preserved verbatim.
    Unrecognized(String),
}

impl EventMarkerIconChoice {
    /// The `icon_name` wire value this choice writes: the empty name for
    /// [`EventMarkerIconChoice::Auto`]. Inverse of
    /// [`EventMarkerIconChoice::from_wire_name`].
    pub fn wire_name(&self) -> &str {
        match self {
            Self::Auto => "",
            Self::Icon(icon) => icon.name(),
            Self::Unrecognized(name) => name,
        }
    }

    /// Reads an `icon_name` wire value: an empty name gives
    /// [`EventMarkerIconChoice::Auto`], a name in the [`MarkerIcon`] set
    /// [`EventMarkerIconChoice::Icon`], and any other name
    /// [`EventMarkerIconChoice::Unrecognized`].
    pub fn from_wire_name(name: impl Into<String>) -> Self {
        let name = name.into();
        if name.is_empty() {
            return Self::Auto;
        }
        match MarkerIcon::try_from_lower_case(&name) {
            Ok(icon) => Self::Icon(icon),
            Err(_) => Self::Unrecognized(name),
        }
    }
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

#[cfg(test)]
mod event_marker_style_wire_tests {
    use rstest::rstest;

    use super::*;

    #[rstest]
    #[case("", EventMarkerColor::Auto)]
    #[case("#ff9900", EventMarkerColor::Hex("#ff9900".to_owned()))]
    #[case("#FF990", EventMarkerColor::Unrecognized("#FF990".to_owned()))]
    #[case("#GG9900", EventMarkerColor::Unrecognized("#GG9900".to_owned()))]
    #[case("FF9900", EventMarkerColor::Unrecognized("FF9900".to_owned()))]
    fn from_wire_value_reads_a_color_hex_field(
        #[case] wire: &str,
        #[case] expected: EventMarkerColor,
    ) {
        assert_eq!(EventMarkerColor::from_wire_value(wire), expected);
    }

    #[rstest]
    #[case("", EventMarkerIconChoice::Auto)]
    #[case("wrench", EventMarkerIconChoice::Icon(MarkerIcon::Wrench))]
    #[case("Wrench", EventMarkerIconChoice::Unrecognized("Wrench".to_owned()))]
    #[case("hovercraft", EventMarkerIconChoice::Unrecognized("hovercraft".to_owned()))]
    fn from_wire_name_reads_an_icon_name_field(
        #[case] wire: &str,
        #[case] expected: EventMarkerIconChoice,
    ) {
        assert_eq!(EventMarkerIconChoice::from_wire_name(wire), expected);
    }

    #[rstest]
    #[case(EventMarkerIconChoice::Auto, "")]
    #[case(
        EventMarkerIconChoice::Icon(MarkerIcon::SatelliteLost),
        "satellite_lost"
    )]
    #[case(EventMarkerIconChoice::Unrecognized("hovercraft".to_owned()), "hovercraft")]
    fn wire_name_writes_an_icon_name_field(
        #[case] choice: EventMarkerIconChoice,
        #[case] expected: &str,
    ) {
        assert_eq!(choice.wire_name(), expected);
    }

    #[test]
    fn try_from_rejects_a_value_that_is_not_rrggbb() {
        let err = EventMarkerColor::try_from("FF9900".to_owned())
            .expect_err("a value that is not #RRGGBB is rejected");
        assert_eq!(
            err.to_string(),
            "failed to parse EventMarkerColor (hex) from \"FF9900\": expected #RRGGBB format"
        );
    }
}

/// Per-variant icon and color override stored in the file.
#[derive(Debug, Clone, PartialEq)]
pub struct EventMarkerStyle {
    /// Must exactly match a `variant_path` used in the event markers.
    pub variant_path: String,
    /// Icon shape. Defaults to `Auto` (Pin).
    pub icon: EventMarkerIconChoice,
    /// Fill color. Defaults to `Auto` (hash-derived from the variant path).
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
#[derive(Debug, Clone, PartialEq)]
pub struct EventMarkerPoint {
    pub variant_path: String,
    pub sys_time: chrono::DateTime<chrono::Utc>,
    pub lat: Angle,
    pub lon: Angle,
    pub annotation: Option<String>,
}

/// A named scalar or vector time series recorded alongside the nav track: an
/// ad-hoc sensor metric such as an accelerometer's x/y/z axes or an inclinometer
/// angle, sampled at its own rate and correlated with the track by timestamp.
///
/// Stored under `channels/<name>/` as a `time` (µs) dataset and a `value`
/// dataset (1-D for a scalar channel, 2-D `[n, k]` for a vector channel), with
/// the unit, wrap period, description, and component labels carried as
/// attributes on the channel's group. Build one with [`Channel::builder`] and
/// attach it via [`NavRecorder::add_channel`](crate::NavRecorder::add_channel).
#[derive(Debug, Clone, PartialEq)]
pub struct Channel {
    pub(crate) name: String,
    pub(crate) unit: Option<ChannelUnit>,
    pub(crate) period: Option<Angle>,
    pub(crate) description: Option<String>,
    /// Vector component labels (`["x", "y", "z"]`), or empty for a scalar
    /// channel. When non-empty, `values` holds one column per component.
    pub(crate) components: Vec<String>,
    pub(crate) times: Vec<DateTime<Utc>>,
    /// Sample values in row-major order: `times.len()` rows of
    /// `component_count()` columns each. A scalar channel has one column, so
    /// this is one value per timestamp.
    pub(crate) values: Vec<f64>,
}

#[bon::bon]
impl Channel {
    /// Build a validated [`Channel`], scalar or vector.
    ///
    /// `name` must be a lowercase identifier, since queries reference it as
    /// `@name`. Pass `components` (e.g. `["x", "y", "z"]`) to make a vector
    /// channel whose `value` dataset has one column per component. Each label
    /// must be a unique identifier, referenced as `@name.label`. Omit it for a
    /// scalar channel. Use a recognized [`ChannelUnit`] when GeoTrace should
    /// scale and dimension-check the values. Use [`ChannelUnit::custom`] for a
    /// deliberate display-only label whose values remain dimensionless.
    ///
    /// `values` is row-major: `times.len()` rows of one column (scalar) or
    /// `components.len()` columns (vector). Returns `Err` if the name or a
    /// component label is malformed, `values` is not `times.len() × columns`
    /// long, or a wrap period is invalid or paired with a non-angular unit.
    #[builder(finish_fn = build)]
    pub fn new(
        #[builder(into)] name: String,
        #[builder(into)] unit: Option<ChannelUnit>,
        period: Option<Angle>,
        #[builder(into)] description: Option<String>,
        // Any iterable of stringlike labels: `["x", "y", "z"]`, a `vec!` of
        // `&str`, or an owned `Vec<String>` all work.
        #[builder(with = |labels: impl IntoIterator<Item: Into<String>>| {
            labels.into_iter().map(Into::into).collect()
        })]
        components: Option<Vec<String>>,
        times: Vec<DateTime<Utc>>,
        values: Vec<f64>,
    ) -> Result<Self, ChannelError> {
        crate::error::validate_channel_name(&name)?;
        if let Some(unit) = unit.as_ref().filter(|unit| !unit.is_writable()) {
            return Err(ChannelError::UnwritableUnit {
                name,
                unit: unit.to_string(),
            });
        }
        // `None` is a scalar channel. `Some(list)` is a vector channel, and an
        // explicitly empty list is rejected by `validate_components`.
        let components = match components {
            None => Vec::new(),
            Some(list) => {
                crate::error::validate_components(&name, &list)?;
                list
            }
        };
        let columns = components.len().max(1);
        let expected = times.len() * columns;
        if values.len() != expected {
            return Err(ChannelError::LengthMismatch {
                name,
                expected,
                actual: values.len(),
            });
        }
        if let Some(period) = period {
            let degrees = period.as_degrees();
            if !(degrees.is_finite() && degrees > 0.0) {
                return Err(ChannelError::InvalidPeriod { name });
            }
            let angular = unit
                .as_ref()
                .and_then(ChannelUnit::as_recognized)
                .is_some_and(|unit| unit.quantity() == PhysicalQuantity::Angle);
            if !angular {
                return Err(ChannelError::PeriodNeedsAngularUnit { name });
            }
        }
        Ok(Self {
            name,
            unit,
            period,
            description: description.filter(|s| !s.trim().is_empty()),
            components,
            times,
            values,
        })
    }
}

impl Channel {
    /// The channel's identifier, referenced as `@name` in queries.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The recognized or custom unit of the channel's values, when declared.
    pub fn unit(&self) -> Option<&ChannelUnit> {
        self.unit.as_ref()
    }

    /// The wrap period of an angular channel (`360°` for a heading), or `None`
    /// for a linear value.
    ///
    /// A `deg` channel without a period holds an unbounded angle: only a
    /// channel with a period wraps.
    pub fn period(&self) -> Option<Angle> {
        self.period
    }

    /// A human description of what the channel records, when declared.
    pub fn description(&self) -> Option<&str> {
        self.description.as_deref()
    }

    /// The vector component labels (`["x", "y", "z"]`), or empty for a scalar
    /// channel. Each is referenced as `@name.label` in queries.
    pub fn components(&self) -> &[String] {
        &self.components
    }

    /// Whether this is a vector channel (has named components).
    pub fn is_vector(&self) -> bool {
        !self.components.is_empty()
    }

    /// Value columns per sample: the component count for a vector channel, or 1
    /// for a scalar channel.
    pub fn component_count(&self) -> usize {
        self.components.len().max(1)
    }

    /// The sample timestamps, one per row of [`values`](Self::values).
    pub fn times(&self) -> &[DateTime<Utc>] {
        &self.times
    }

    /// The sample values, in [`unit`](Self::unit), in row-major order:
    /// `times.len()` rows of [`component_count`](Self::component_count) columns.
    /// For a scalar channel this is one value per timestamp. See
    /// [`rows`](Self::rows) to iterate per sample.
    pub fn values(&self) -> &[f64] {
        &self.values
    }

    /// The samples as rows, each holding one value per column
    /// ([`component_count`](Self::component_count) values). A scalar channel
    /// yields one-element rows.
    pub fn rows(&self) -> impl Iterator<Item = &[f64]> {
        self.values.chunks(self.component_count())
    }
}

/// A complete, validated GeoTrace data file ready for serialisation.
///
/// Construct via [`NavRecorder::finish`](crate::NavRecorder::finish).
#[derive(Debug, Clone, PartialEq)]
pub struct NavFile {
    pub(crate) meta: Meta,
    pub(crate) nav_points: Vec<NavPoint>,
    pub(crate) markers: Vec<Marker>,
    pub(crate) event_markers: Vec<EventMarkerPoint>,
    pub(crate) event_marker_styles: Vec<EventMarkerStyle>,
    pub(crate) channels: Vec<Channel>,
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

    pub fn channels(&self) -> &[Channel] {
        &self.channels
    }

    /// Compares two files' recorded content, ignoring their SDK build stamp.
    ///
    /// [`Meta::sdk_version`], [`Meta::sdk_git_commit`] and
    /// [`Meta::sdk_commit_time`] name the SDK build that wrote a file, so `==`
    /// reports two files of the same recording as unequal when different builds
    /// wrote them.
    pub fn equals_ignoring_build_provenance(&self, other: &Self) -> bool {
        let Self {
            meta,
            nav_points,
            markers,
            event_markers,
            event_marker_styles,
            channels,
        } = self;
        meta.equals_ignoring_build_provenance(&other.meta)
            && *nav_points == other.nav_points
            && *markers == other.markers
            && *event_markers == other.event_markers
            && *event_marker_styles == other.event_marker_styles
            && *channels == other.channels
    }

    /// Serialise the file to `writer`.
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

    /// Read a `.gtd` file from `reader`.
    pub fn read<R: io::Read>(mut reader: R) -> Result<Self, crate::error::Error> {
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes)?;
        crate::read::parse_hdf5(bytes)
    }

    /// Open a `.gtd` file at `path` and parse it.
    pub fn open(path: impl AsRef<std::path::Path>) -> Result<Self, crate::error::Error> {
        Self::read(File::open(path)?)
    }

    /// Pretty-print a summary of the `.gtd` file at `path`.
    pub fn inspect(path: impl AsRef<std::path::Path>) -> Result<String, crate::error::Error> {
        crate::read::inspect_path(path.as_ref())
    }
}

#[cfg(test)]
mod nav_file_comparison_tests {
    use rstest::rstest;

    use super::*;

    fn a_nav_file() -> NavFile {
        let time = DateTime::from_timestamp(1_748_000_000, 0).expect("a valid timestamp");
        let lat = Angle::degrees(51.5);
        let lon = Angle::degrees(-0.1);
        NavFile {
            meta: Meta::builder().title("A recording").build(),
            nav_points: vec![NavPoint {
                fix: NavFix::builder()
                    .time(NavFixTime::Receiver(time))
                    .lat(lat)
                    .lon(lon)
                    .build(),
                satellites: None,
            }],
            markers: vec![Marker {
                annotation: Annotation::builder()
                    .time(time)
                    .label("Coffee stop")
                    .build()
                    .expect("the marker label fits its field"),
                lat,
                lon,
            }],
            event_markers: vec![EventMarkerPoint {
                variant_path: "power/boot".to_owned(),
                sys_time: time,
                lat,
                lon,
                annotation: None,
            }],
            event_marker_styles: vec![
                EventMarkerStyle::builder()
                    .variant_path("power/boot")
                    .build()
                    .expect("the variant path fits its field"),
            ],
            channels: vec![
                Channel::builder()
                    .name("speed")
                    .times(vec![time])
                    .values(vec![3.0])
                    .build()
                    .expect("one value per timestamp"),
            ],
        }
    }

    #[test]
    fn two_files_differing_only_in_their_build_stamp_are_equal_ignoring_it() {
        let mut stamped = a_nav_file();
        stamped.meta.stamp_this_build();
        let mut scrubbed = a_nav_file();
        scrubbed.meta.stamp_scrubbed_provenance();

        assert!(stamped.equals_ignoring_build_provenance(&scrubbed));
        assert_ne!(stamped, scrubbed);
    }

    #[rstest]
    #[case::title(|file: &mut NavFile| file.meta.title = Some("Another recording".to_owned()))]
    #[case::device(|file: &mut NavFile| file.meta.device = Some("Another device".to_owned()))]
    #[case::notes(|file: &mut NavFile| file.meta.notes = Some("A note".to_owned()))]
    #[case::identity(|file: &mut NavFile| file.meta.identity = Some("another-key".to_owned()))]
    #[case::travel_mode(|file: &mut NavFile| file.meta.travel_mode = Some(TravelMode::Boat))]
    #[case::nav_points(|file: &mut NavFile| file.nav_points.clear())]
    #[case::markers(|file: &mut NavFile| file.markers.clear())]
    #[case::event_markers(|file: &mut NavFile| file.event_markers.clear())]
    #[case::event_marker_styles(|file: &mut NavFile| file.event_marker_styles.clear())]
    #[case::channels(|file: &mut NavFile| file.channels.clear())]
    fn two_files_differing_in_one_field_are_unequal_ignoring_the_build_stamp(
        #[case] change: impl FnOnce(&mut NavFile),
    ) {
        let file = a_nav_file();
        let mut changed = a_nav_file();
        change(&mut changed);

        assert!(!file.equals_ignoring_build_provenance(&changed));
    }
}

use crate::time_types::{GpsTime, SysTime};
use chrono::{DateTime, Utc};
use geotrace_sdk_units::snr;
use std::cmp::Ordering;
use std::fmt;

/// Pseudo-Random Noise code number that uniquely identifies a satellite within its constellation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Prn(u32);

impl Prn {
    pub fn new(value: u32) -> Self {
        Self(value)
    }

    pub fn value(self) -> u32 {
        self.0
    }
}

impl fmt::Display for Prn {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl PartialEq<u32> for Prn {
    fn eq(&self, other: &u32) -> bool {
        self.0 == *other
    }
}

/// Signal quality tier derived from an [`Snr`] value. The measured tiers are
/// declared strongest first, and [`SignalQuality::NoDataSentinel`] last: it
/// classifies a reading the receiver made no measurement for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, strum::EnumIter)]
pub enum SignalQuality {
    /// ≥ 40 dB-Hz - excellent lock.
    Excellent,
    /// 35–40 dB-Hz - good.
    Good,
    /// 30–35 dB-Hz - moderate.
    Moderate,
    /// 25–30 dB-Hz - weak.
    Weak,
    /// < 25 dB-Hz - very weak / marginal.
    VeryWeak,
    /// [`NO_DATA_SENTINEL_DB_HZ`] - some receiver firmware sends this value
    /// when it has no measurement.
    NoDataSentinel,
}

/// The hover text beside a reading of [`SignalQuality::NoDataSentinel`], for
/// every surface that shows one.
pub const NO_DATA_SNR_EXPLANATION: &str =
    "Some receivers send this value when they have no measurement";

/// The SNR in dB-Hz some receiver firmware sends when it has no measurement. The
/// `.gtd` format defines it, and the SDKs and the application classify a
/// reading with the one definition.
pub use geotrace_sdk_units::snr::NO_DATA_SENTINEL_DB_HZ;

/// Signal-to-Noise Ratio for a satellite signal, in dB-Hz.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub struct Snr(f32);

impl Snr {
    pub fn new(value: f32) -> Self {
        Self(value)
    }

    pub fn value(self) -> f32 {
        self.0
    }

    /// Whether the receiver sent [`NO_DATA_SENTINEL_DB_HZ`], which it sends
    /// when it has no measurement.
    pub fn is_no_data_sentinel(self) -> bool {
        snr::is_no_data_sentinel(self.0)
    }

    pub fn quality(self) -> SignalQuality {
        if self.is_no_data_sentinel() {
            SignalQuality::NoDataSentinel
        } else if self.0 >= 40.0 {
            SignalQuality::Excellent
        } else if self.0 >= 35.0 {
            SignalQuality::Good
        } else if self.0 >= 30.0 {
            SignalQuality::Moderate
        } else if self.0 >= 25.0 {
            SignalQuality::Weak
        } else {
            SignalQuality::VeryWeak
        }
    }
}

/// Variant declaration order also defines the `Ord` sort order (GPS first,
/// then GLONASS, Galileo, BeiDou, NavIC, QZSS), used to group satellites by
/// constellation in tables.
///
/// Serialized by name (`"gps"`, `"glonass"`, ...), pinned by
/// `constellation_wire_names_are_stable`.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    PartialOrd,
    Ord,
    strum::EnumCount,
    strum::EnumIter,
    serde::Serialize,
    serde::Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum Constellation {
    /// United States' Global Positioning System
    Gps,
    /// Russia's Global Navigation Satellite System
    Glonass,
    /// European Union's Galileo system
    Galileo,
    /// China's BeiDou Navigation Satellite System
    Beidou,
    /// India's Navigation with Indian Constellation (NavIC / IRNSS)
    Navic,
    /// Japan's Quasi-Zenith Satellite System
    Qzss,
}

impl Constellation {
    /// Canonical human-readable name, e.g. `Constellation::Beidou.display_name() == "BeiDou"`.
    ///
    /// Matches `geotrace_sdk::Constellation::display_name` for the
    /// structurally-identical SDK/wire-format type. Keep the two in sync.
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

    /// RINEX single-letter satellite prefix, e.g. `G` for GPS PRN labels
    /// ("G05"). Single source for every per-PRN table and label.
    pub fn prn_prefix(self) -> &'static str {
        match self {
            Constellation::Gps => "G",
            Constellation::Glonass => "R",
            Constellation::Galileo => "E",
            Constellation::Beidou => "C",
            Constellation::Navic => "I",
            Constellation::Qzss => "J",
        }
    }
}

crate::enum_bitset! {
    /// A set of GNSS constellations, one bit each, e.g. which constellations a
    /// query covers.
    pub struct ConstellationSet(u8) for Constellation;
}

#[derive(Debug, Clone, Copy)]
pub struct Satellite {
    constellation: Constellation,
    prn: Prn,
    in_fix: bool,
    elevation: Option<f32>,
    azimuth: Option<f32>,
    snr: Option<Snr>,
}

impl Satellite {
    pub fn new(
        constellation: Constellation,
        prn: u32,
        elevation: Option<f32>,
        azimuth: Option<f32>,
        snr: Option<f32>,
        in_fix: bool,
    ) -> Self {
        Self {
            constellation,
            prn: Prn::new(prn),
            in_fix,
            elevation,
            azimuth,
            snr: snr.map(Snr::new),
        }
    }

    pub fn constellation(&self) -> Constellation {
        self.constellation
    }
    pub fn prn(&self) -> Prn {
        self.prn
    }
    pub fn in_fix(&self) -> bool {
        self.in_fix
    }
    pub fn elevation(&self) -> Option<f32> {
        self.elevation
    }
    pub fn azimuth(&self) -> Option<f32> {
        self.azimuth
    }
    pub fn snr(&self) -> Option<Snr> {
        self.snr
    }

    /// The SNR the receiver measured for this satellite. `None` for a
    /// satellite with a missing SNR, and for one with the no-data value.
    pub fn measured_snr(&self) -> Option<Snr> {
        self.snr.filter(|snr| !snr.is_no_data_sentinel())
    }

    /// Merges another row of the same report for this satellite into this one.
    ///
    /// This satellite is in the fix when any of its rows was, and takes the
    /// highest SNR measured on its rows: the strongest signal measured for the
    /// satellite. It takes the no-data value only where no row measured an SNR.
    /// Taking the highest keeps the result independent of row order, which
    /// `gt_analysis::loss_of_lock` reads when it compares a satellite's SNR
    /// between epochs. Elevation and azimuth are properties of the satellite's
    /// geometry, not of the signal: this satellite keeps the first value any of
    /// its rows holds for each of them.
    pub fn absorb_repeated_row(&mut self, row: Satellite) {
        self.in_fix |= row.in_fix;
        self.elevation = self.elevation.or(row.elevation);
        self.azimuth = self.azimuth.or(row.azimuth);
        self.snr = match (self.measured_snr(), row.measured_snr()) {
            (Some(highest_so_far), Some(row_snr)) => {
                Some(Snr::new(highest_so_far.value().max(row_snr.value())))
            }
            (Some(measured), None) | (None, Some(measured)) => Some(measured),
            (None, None) => self.snr.or(row.snr),
        };
    }
}

/// Why a [`Slip`] was recorded for a satellite.
///
/// The detection algorithm lives in the `gt-analysis` crate. This type is shared
/// so both the slip-rate plot and the generated-marker pipeline describe a slip
/// the same way.
#[derive(Debug, Clone, Copy, PartialEq, Eq, strum::EnumCount, strum::EnumIter)]
pub enum SlipCause {
    /// Satellite disappeared while above the mask - the receiver lost lock.
    LostLock,
    /// Satellite stayed above the mask but its SNR dropped sharply between epochs.
    SnrDrop,
}

impl SlipCause {
    /// Short human-readable cause, e.g. `"lost lock"`. Single source for
    /// every slip marker and tooltip.
    pub fn label(self) -> &'static str {
        match self {
            SlipCause::LostLock => "lost lock",
            SlipCause::SnrDrop => "SNR drop",
        }
    }
}

/// A satellite's tracked geometry and signal at one epoch - the before/after
/// payload of a [`Slip`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SatSample {
    pub elevation: Option<f32>,
    pub azimuth: Option<f32>,
    pub snr: Option<Snr>,
}

impl SatSample {
    /// Snapshot the tracked geometry and signal of `sat`.
    pub fn of(sat: &Satellite) -> Self {
        Self {
            elevation: sat.elevation,
            azimuth: sat.azimuth,
            snr: sat.snr,
        }
    }
}

/// A loss-of-lock (cycle slip) detected for one satellite at one epoch, relative
/// to the previous one.  Produced by `gt_analysis::loss_of_lock`.
///
/// Carries the satellite's state on both sides of the transition so a marker can
/// show what changed: `to` is `None` for a [`SlipCause::LostLock`] (the satellite
/// is no longer reported this epoch).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Slip {
    pub constellation: Constellation,
    pub prn: Prn,
    pub cause: SlipCause,
    /// The satellite at the previous epoch (before the slip).
    pub from: SatSample,
    /// The satellite at the current epoch, or `None` when it dropped out.
    pub to: Option<SatSample>,
}

/// All satellites that slipped at one epoch (one satellite report).
///
/// Slips detected at the same epoch are grouped into one event, so the map shows
/// one marker per epoch listing every affected satellite.
#[derive(Debug, Clone, PartialEq)]
pub struct SlipEvent {
    pub slips: Vec<Slip>,
}

#[derive(Debug, Clone)]
pub struct Satellites {
    /// GPS receiver clock timestamp, if the original report had `gps_time`.
    gps_time: Option<GpsTime>,
    /// Host system-clock timestamp, if the original report had `sys_time`.
    sys_time: Option<SysTime>,
    fix_count: u32,
    satellite_count: u32,
    satellites: Vec<Satellite>,
}

impl Satellites {
    /// Construct a satellite report.
    ///
    /// At least one of `gps_time` / `sys_time` should be `Some`. The builder
    /// guarantees this in practice, but it is not enforced here.
    pub fn new(
        gps_time: Option<GpsTime>,
        sys_time: Option<SysTime>,
        satellites: Vec<Satellite>,
    ) -> Self {
        let fix_count = satellites.iter().filter(|s| s.in_fix).count() as u32;
        let satellite_count = satellites.len() as u32;
        Self {
            gps_time,
            sys_time,
            fix_count,
            satellite_count,
            satellites,
        }
    }

    /// GPS receiver clock timestamp, if the report was GPS-timestamped.
    pub fn gps_time(&self) -> Option<GpsTime> {
        self.gps_time
    }

    /// Host system-clock timestamp, if the report was system-clock-timestamped.
    pub fn sys_time(&self) -> Option<SysTime> {
        self.sys_time
    }

    /// Best available timestamp for display (GPS time preferred over system
    /// time).  Returns `None` only when both clocks are absent, which should
    /// not occur for any report that passed `finish()`.
    pub fn best_time(&self) -> Option<DateTime<Utc>> {
        self.gps_time
            .map(GpsTime::utc)
            .or_else(|| self.sys_time.map(SysTime::utc))
    }

    /// `true` when this report has a GPS receiver clock timestamp.
    pub fn time_from_gps(&self) -> bool {
        self.gps_time.is_some()
    }

    /// The number of satellites actively contributing to the positional fix.
    pub fn fix_count(&self) -> u32 {
        self.fix_count
    }

    /// The total number of satellites currently being tracked, regardless of their fix status.
    pub fn satellite_count(&self) -> u32 {
        self.satellite_count
    }

    pub fn satellites(&self) -> impl Iterator<Item = &Satellite> {
        self.satellites.iter()
    }

    pub fn satellites_with_fix(&self) -> impl Iterator<Item = &Satellite> {
        self.satellites.iter().filter(|s| s.in_fix)
    }

    pub fn by_constellation(
        &self,
        constellation: Constellation,
    ) -> impl Iterator<Item = &Satellite> {
        self.satellites
            .iter()
            .filter(move |s| s.constellation == constellation)
    }

    /// The strongest Signal-to-Noise Ratio (SNR) across all tracked satellites.
    ///
    /// Returns `None` if no SNR data is available.
    pub fn max_snr(&self) -> Option<Snr> {
        self.satellites
            .iter()
            .filter_map(|s| s.snr)
            .max_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal))
    }

    /// The strongest Signal-to-Noise Ratio (SNR) for a specific constellation.
    ///
    /// Returns `None` if no satellites in the constellation have SNR data.
    pub fn max_snr_by_constellation(&self, constellation: Constellation) -> Option<Snr> {
        self.by_constellation(constellation)
            .filter_map(|s| s.snr)
            .max_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal))
    }

    pub fn is_in_fix(&self, constellation: Constellation, prn: Prn) -> bool {
        self.satellites
            .iter()
            .any(|s| s.in_fix && s.constellation == constellation && s.prn == prn)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    mod constellation {
        use super::*;

        /// The persisted names for each constellation. Pinned so a rename or a
        /// reorder cannot silently invalidate saved settings that list folded
        /// constellations by name.
        #[test]
        fn constellation_wire_names_are_stable() {
            use serde::Deserialize as _;
            use serde::de::IntoDeserializer as _;
            use serde::de::value::{Error as DeError, StrDeserializer};
            use strum::EnumCount as _;

            let expected = [
                (Constellation::Gps, "gps"),
                (Constellation::Glonass, "glonass"),
                (Constellation::Galileo, "galileo"),
                (Constellation::Beidou, "beidou"),
                (Constellation::Navic, "navic"),
                (Constellation::Qzss, "qzss"),
            ];
            assert_eq!(expected.len(), Constellation::COUNT);
            for (constellation, wire) in expected {
                let de: StrDeserializer<'_, DeError> = wire.into_deserializer();
                assert_eq!(
                    Constellation::deserialize(de),
                    Ok(constellation),
                    "deserializing {wire:?}"
                );
            }
        }

        /// Pins the canonical constellation display spelling. Keep in sync with
        /// `geotrace_sdk::Constellation::display_name`'s identical assertions.
        #[test]
        fn display_name_is_canonical_spelling() {
            use strum::EnumCount;
            let expected = [
                (Constellation::Gps, "GPS"),
                (Constellation::Glonass, "GLONASS"),
                (Constellation::Galileo, "Galileo"),
                (Constellation::Beidou, "BeiDou"),
                (Constellation::Navic, "NavIC"),
                (Constellation::Qzss, "QZSS"),
            ];
            // Length-vs-COUNT guard: a new variant without a name entry fails here.
            assert_eq!(expected.len(), Constellation::COUNT);
            for (c, name) in expected {
                assert_eq!(c.display_name(), name);
            }
        }

        /// Single source of truth for RINEX PRN prefixes, COUNT-guarded like
        /// `display_name_is_canonical_spelling`.
        #[test]
        fn prn_prefix_is_canonical() {
            use strum::EnumCount;
            let expected = [
                (Constellation::Gps, "G"),
                (Constellation::Glonass, "R"),
                (Constellation::Galileo, "E"),
                (Constellation::Beidou, "C"),
                (Constellation::Navic, "I"),
                (Constellation::Qzss, "J"),
            ];
            assert_eq!(expected.len(), Constellation::COUNT);
            for (c, prefix) in expected {
                assert_eq!(c.prn_prefix(), prefix);
            }
        }

        #[test]
        fn slip_cause_label_is_canonical() {
            use strum::{EnumCount, IntoEnumIterator};
            let expected = [
                (SlipCause::LostLock, "lost lock"),
                (SlipCause::SnrDrop, "SNR drop"),
            ];
            assert_eq!(expected.len(), SlipCause::COUNT);
            for (cause, label) in expected {
                assert_eq!(cause.label(), label);
            }
            // Every cause has a non-empty label.
            assert!(
                SlipCause::iter()
                    .map(SlipCause::label)
                    .all(|l| !l.is_empty())
            );
        }
    }

    mod snr {
        use rstest::rstest;

        use super::{Constellation, NO_DATA_SENTINEL_DB_HZ, Satellite, SignalQuality, Snr};

        #[rstest]
        #[case::excellent(44.0, SignalQuality::Excellent)]
        #[case::at_the_excellent_threshold(40.0, SignalQuality::Excellent)]
        #[case::good(37.0, SignalQuality::Good)]
        #[case::moderate(32.0, SignalQuality::Moderate)]
        #[case::weak(27.0, SignalQuality::Weak)]
        #[case::very_weak(10.0, SignalQuality::VeryWeak)]
        #[case::zero_is_a_measurement(0.0, SignalQuality::VeryWeak)]
        #[case::the_no_data_value(99.0, SignalQuality::NoDataSentinel)]
        #[case::inside_the_no_data_band(99.4, SignalQuality::NoDataSentinel)]
        #[case::just_below_the_no_data_band(98.5, SignalQuality::Excellent)]
        #[case::just_above_the_no_data_band(99.5, SignalQuality::Excellent)]
        fn quality_classifies_a_reading(#[case] snr_db: f32, #[case] expected: SignalQuality) {
            assert_eq!(Snr::new(snr_db).quality(), expected);
        }

        #[rstest]
        #[case::the_no_data_value(NO_DATA_SENTINEL_DB_HZ, None)]
        #[case::a_measurement(40.0, Some(40.0))]
        fn measured_snr_drops_the_no_data_value(
            #[case] snr_db: f32,
            #[case] expected_db: Option<f32>,
        ) {
            let satellite = Satellite::new(Constellation::Gps, 7, None, None, Some(snr_db), false);

            assert_eq!(satellite.measured_snr().map(Snr::value), expected_db);
            assert_eq!(satellite.snr().map(Snr::value), Some(snr_db));
        }
    }

    mod satellite {
        use rstest::rstest;

        use super::{Constellation, NO_DATA_SENTINEL_DB_HZ, Satellite};

        const PRN: u32 = 7;

        const FIRST_ELEVATION_DEG: f32 = 40.0;

        const FIRST_AZIMUTH_DEG: f32 = 90.0;

        const SECOND_ELEVATION_DEG: f32 = 10.0;

        const SECOND_AZIMUTH_DEG: f32 = 200.0;

        fn row_with_snr(snr_db: Option<f32>) -> Satellite {
            Satellite::new(Constellation::Gps, PRN, None, None, snr_db, false)
        }

        fn row_with_geometry(elevation_deg: Option<f32>, azimuth_deg: Option<f32>) -> Satellite {
            Satellite::new(
                Constellation::Gps,
                PRN,
                elevation_deg,
                azimuth_deg,
                None,
                false,
            )
        }

        #[rstest]
        #[case::the_higher_snr_first(Some(45.0), Some(30.0), Some(45.0))]
        #[case::the_higher_snr_second(Some(30.0), Some(45.0), Some(45.0))]
        #[case::only_the_first_row_reports_an_snr(Some(45.0), None, Some(45.0))]
        #[case::only_the_second_row_reports_an_snr(None, Some(45.0), Some(45.0))]
        #[case::neither_row_reports_an_snr(None, None, None)]
        #[case::the_first_row_holds_the_no_data_value(
            Some(NO_DATA_SENTINEL_DB_HZ),
            Some(30.0),
            Some(30.0)
        )]
        #[case::the_second_row_holds_the_no_data_value(
            Some(30.0),
            Some(NO_DATA_SENTINEL_DB_HZ),
            Some(30.0)
        )]
        #[case::both_rows_hold_the_no_data_value(
            Some(NO_DATA_SENTINEL_DB_HZ),
            Some(NO_DATA_SENTINEL_DB_HZ),
            Some(NO_DATA_SENTINEL_DB_HZ)
        )]
        #[case::the_no_data_value_and_no_snr(
            Some(NO_DATA_SENTINEL_DB_HZ),
            None,
            Some(NO_DATA_SENTINEL_DB_HZ)
        )]
        #[case::no_snr_and_the_no_data_value(
            None,
            Some(NO_DATA_SENTINEL_DB_HZ),
            Some(NO_DATA_SENTINEL_DB_HZ)
        )]
        fn absorb_repeated_row_keeps_the_highest_snr_measured_on_the_two_rows(
            #[case] snr_db: Option<f32>,
            #[case] absorbed_snr_db: Option<f32>,
            #[case] expected_snr_db: Option<f32>,
        ) {
            let mut satellite = row_with_snr(snr_db);

            satellite.absorb_repeated_row(row_with_snr(absorbed_snr_db));

            assert_eq!(satellite.snr().map(|snr| snr.value()), expected_snr_db);
        }

        #[test]
        fn absorb_repeated_row_keeps_the_elevation_and_azimuth_of_the_row_reporting_them_first() {
            let mut satellite =
                row_with_geometry(Some(FIRST_ELEVATION_DEG), Some(FIRST_AZIMUTH_DEG));

            satellite.absorb_repeated_row(row_with_geometry(
                Some(SECOND_ELEVATION_DEG),
                Some(SECOND_AZIMUTH_DEG),
            ));

            assert_eq!(
                (satellite.elevation(), satellite.azimuth()),
                (Some(FIRST_ELEVATION_DEG), Some(FIRST_AZIMUTH_DEG))
            );
        }

        #[test]
        fn absorb_repeated_row_takes_an_elevation_and_azimuth_the_satellite_has_none_of() {
            let mut satellite = row_with_geometry(None, None);

            satellite.absorb_repeated_row(row_with_geometry(
                Some(SECOND_ELEVATION_DEG),
                Some(SECOND_AZIMUTH_DEG),
            ));

            assert_eq!(
                (satellite.elevation(), satellite.azimuth()),
                (Some(SECOND_ELEVATION_DEG), Some(SECOND_AZIMUTH_DEG))
            );
        }

        #[rstest]
        #[case::the_satellite_is_in_the_fix(true, false)]
        #[case::the_absorbed_row_is_in_the_fix(false, true)]
        fn absorb_repeated_row_puts_the_satellite_in_the_fix_when_either_row_was(
            #[case] in_fix: bool,
            #[case] absorbed_in_fix: bool,
        ) {
            let mut satellite = Satellite::new(Constellation::Gps, PRN, None, None, None, in_fix);

            satellite.absorb_repeated_row(Satellite::new(
                Constellation::Gps,
                PRN,
                None,
                None,
                None,
                absorbed_in_fix,
            ));

            assert!(satellite.in_fix());
        }
    }
}

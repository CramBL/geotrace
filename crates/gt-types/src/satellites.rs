use crate::time_types::{GpsTime, SysTime};
use chrono::{DateTime, Utc};
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

/// Signal quality tier derived from an [`Snr`] value.
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
}

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

    pub fn quality(self) -> SignalQuality {
        if self.0 >= 40.0 {
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
/// Serialized by name (`"gps"`, `"glonass"`, ...) rather than by index, so a
/// persisted list stays readable and survives variants being added or
/// reordered. The wire names are pinned by `constellation_wire_names_are_stable`.
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
    /// Single source of truth for this type's display spelling - call sites
    /// (e.g. `gt-map`'s satellite panel) should format through this rather than
    /// re-typing the name. Mirrors `geotrace_sdk::Constellation::display_name`,
    /// which answers the same spelling question for the structurally-identical
    /// SDK/wire-format type. Keep the two in sync.
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
    /// A set of GNSS constellations, one bit each - cheap to pass and combine
    /// where a `HashSet` would allocate. Used to describe "which constellations"
    /// a query covers, from a single one up to all of them.
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
}

/// Why a [`Slip`] was recorded for a satellite.
///
/// The detection algorithm lives in the `gt-analysis` crate; this type is shared
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
/// Slips detected at the same epoch are grouped into a single event so the map
/// shows one marker per epoch listing every affected satellite, rather than a
/// stack of overlapping markers.
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

    /// `true` when this report carries a GPS receiver clock timestamp.
    ///
    /// When `false` the report was timestamped by the host system clock only.
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

    /// All currently tracked satellites.
    pub fn satellites(&self) -> impl Iterator<Item = &Satellite> {
        self.satellites.iter()
    }

    /// Satellites that are actively contributing to the current positional fix.
    pub fn satellites_with_fix(&self) -> impl Iterator<Item = &Satellite> {
        self.satellites.iter().filter(|s| s.in_fix)
    }

    /// Tracked satellites belonging to a specific GNSS constellation.
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

    /// The total number of valid fixes resolved by the receiver.
    pub fn total_fix(&self) -> usize {
        self.fix_count as usize
    }

    /// Checks if a specific satellite is currently contributing to the positional fix.
    pub fn is_in_fix(&self, constellation: Constellation, prn: Prn) -> bool {
        self.satellites
            .iter()
            .any(|s| s.in_fix && s.constellation == constellation && s.prn == prn)
    }
}

#[cfg(test)]
mod constellation_tests {
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

    /// Single source of truth for constellation display spelling. Pin it down
    /// so a future edit has to change it here. Keep in sync with
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
    fn constellation_set_membership() {
        use strum::IntoEnumIterator;

        let empty = ConstellationSet::empty();
        assert!(empty.is_empty());
        assert!(Constellation::iter().all(|c| !empty.contains(c)));

        let one = ConstellationSet::single(Constellation::Galileo);
        assert!(!one.is_empty());
        for c in Constellation::iter() {
            assert_eq!(one.contains(c), c == Constellation::Galileo);
        }

        let two = one.with(Constellation::Gps);
        assert!(two.contains(Constellation::Gps));
        assert!(two.contains(Constellation::Galileo));
        assert!(!two.contains(Constellation::Beidou));

        let mut built = ConstellationSet::empty();
        built.insert(Constellation::Gps);
        built.insert(Constellation::Gps);
        assert_eq!(built, ConstellationSet::single(Constellation::Gps));

        built.set(Constellation::Galileo, true);
        assert!(built.contains(Constellation::Galileo));
        built.set(Constellation::Gps, false);
        assert!(!built.contains(Constellation::Gps));
        built.remove(Constellation::Galileo);
        assert!(built.is_empty());

        assert_eq!(
            ConstellationSet::single(Constellation::Gps)
                .union(ConstellationSet::single(Constellation::Galileo)),
            two
        );

        let all = ConstellationSet::all();
        assert!(Constellation::iter().all(|c| all.contains(c)));
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
        // Distinct labels, so no two causes collide.
        assert!(
            SlipCause::iter()
                .map(SlipCause::label)
                .all(|l| !l.is_empty())
        );
    }
}

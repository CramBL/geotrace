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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Constellation {
    /// United States' Global Positioning System
    Gps,
    /// Russia's Global Navigation Satellite System
    Glonass,
    /// European Union's Galileo system
    Galileo,
    /// China's BeiDou Navigation Satellite System
    Beidou,
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
        }
    }
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

    /// Single source of truth for constellation display spelling. Pin it down
    /// so a future edit has to change it here. Keep in sync with
    /// `geotrace_sdk::Constellation::display_name`'s identical assertions.
    #[test]
    fn display_name_is_canonical_spelling() {
        assert_eq!(Constellation::Gps.display_name(), "GPS");
        assert_eq!(Constellation::Glonass.display_name(), "GLONASS");
        assert_eq!(Constellation::Galileo.display_name(), "Galileo");
        assert_eq!(Constellation::Beidou.display_name(), "BeiDou");
    }
}

use crate::time_types::{GpsTime, SysTime};
use chrono::{DateTime, Utc};
use std::cmp::Ordering;

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

#[derive(Debug, Clone, Copy)]
pub struct Satellite {
    constellation: Constellation,
    prn: u32,
    in_fix: bool,
    elevation: Option<f32>,
    azimuth: Option<f32>,
    snr: Option<f32>,
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
            prn,
            in_fix,
            elevation,
            azimuth,
            snr,
        }
    }

    pub fn constellation(&self) -> Constellation {
        self.constellation
    }
    pub fn prn(&self) -> u32 {
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
    pub fn snr(&self) -> Option<f32> {
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
    /// At least one of `gps_time` / `sys_time` should be `Some`; the builder
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
    pub fn max_snr(&self) -> Option<f32> {
        self.satellites
            .iter()
            .filter_map(|s| s.snr)
            .max_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal))
    }

    /// The strongest Signal-to-Noise Ratio (SNR) for a specific constellation.
    ///
    /// Returns `None` if no satellites in the constellation have SNR data.
    pub fn max_snr_by_constellation(&self, constellation: Constellation) -> Option<f32> {
        self.by_constellation(constellation)
            .filter_map(|s| s.snr)
            .max_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal))
    }

    /// The total number of valid fixes resolved by the receiver.
    pub fn total_fix(&self) -> usize {
        self.fix_count as usize
    }

    /// Checks if a specific satellite is currently contributing to the positional fix.
    pub fn is_in_fix(&self, constellation: Constellation, prn: u32) -> bool {
        self.satellites
            .iter()
            .any(|s| s.in_fix && s.constellation == constellation && s.prn == prn)
    }
}

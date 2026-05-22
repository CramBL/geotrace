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
    time: DateTime<Utc>,
    fix_count: u32,
    satellite_count: u32,
    satellites: Vec<Satellite>,
}

impl Satellites {
    pub fn new(time: DateTime<Utc>, satellites: Vec<Satellite>) -> Self {
        let fix_count = satellites.iter().filter(|s| s.in_fix).count() as u32;
        let satellite_count = satellites.len() as u32;
        Self {
            fix_count,
            satellite_count,
            time,
            satellites,
        }
    }

    /// The timestamp when this satellite data was recorded.
    pub fn time(&self) -> DateTime<Utc> {
        self.time
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

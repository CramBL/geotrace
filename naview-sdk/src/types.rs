use chrono::{DateTime, Utc};
use uom::si::f64::{Angle, Velocity};

use crate::error::Error;

/// A single GPS/GNSS fix: position, heading, and optional speed at a point in time.
#[derive(bon::Builder, Debug, Clone, Copy)]
pub struct NavFix {
    pub time: DateTime<Utc>,
    pub lat: Angle,
    pub lon: Angle,
    pub heading: Angle,
    pub speed: Option<Velocity>,
}

/// A satellite visibility report captured at a specific time.
#[derive(bon::Builder, Debug, Clone)]
pub struct SatelliteReport {
    pub time: DateTime<Utc>,
    /// All satellites currently tracked (may include satellites not in the fix).
    pub tracked: Vec<Satellite>,
}

/// One tracked satellite with optional signal metrics.
#[derive(bon::Builder, Debug, Clone, Copy)]
pub struct Satellite {
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
}

/// A user-defined map annotation with an optional label and icon.
#[derive(bon::Builder, Debug, Clone)]
pub struct Annotation {
    pub time: DateTime<Utc>,
    /// Display label; `None` or empty string renders as unlabelled.
    pub label: Option<String>,
    /// Visual icon; `None` defaults to `MarkerIcon::Pin` when loaded.
    pub icon: Option<MarkerIcon>,
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
        }
    }
}

/// Optional file-level metadata.
#[derive(bon::Builder, Debug, Clone, Default)]
pub struct Meta {
    pub title: Option<String>,
    /// Sensor or device that produced the data.
    pub device: Option<String>,
    /// Free-text notes.
    pub notes: Option<String>,
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

/// A complete, validated naview data file ready for serialisation.
///
/// Construct via [`NavFileBuilder::finish`](crate::NavFileBuilder::finish).
#[derive(Debug, Clone)]
pub struct NavFile {
    pub meta: Meta,
    pub(crate) nav_points: Vec<NavPoint>,
    pub(crate) markers: Vec<Marker>,
}

impl NavFile {
    pub fn nav_points(&self) -> &[NavPoint] {
        &self.nav_points
    }

    pub fn markers(&self) -> &[Marker] {
        &self.markers
    }

    /// Serialise the file to the provided writer.
    pub fn write<W: std::io::Write>(&self, mut writer: W) -> Result<(), crate::error::Error> {
        let bytes = crate::write::build_hdf5(self)?;
        writer.write_all(&bytes)?;
        Ok(())
    }

    /// Write to a file at `path`. Appends `.nvd` if `path` has no extension.
    pub fn write_to_file(
        &self,
        path: impl AsRef<std::path::Path>,
    ) -> Result<(), crate::error::Error> {
        let path = path.as_ref();
        let dest = if path.extension().is_none() {
            path.with_extension("nvd")
        } else {
            path.to_path_buf()
        };
        self.write(std::fs::File::create(dest)?)
    }

    /// Read a `.nvd` file from the provided reader.
    pub fn read<R: std::io::Read>(mut reader: R) -> Result<Self, crate::error::Error> {
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes)?;
        crate::read::parse_hdf5(bytes)
    }

    /// Open a `.nvd` file at `path` and parse it.
    pub fn open(path: impl AsRef<std::path::Path>) -> Result<Self, crate::error::Error> {
        Self::read(std::fs::File::open(path)?)
    }

    /// Pretty-print a summary of a `.nvd` file at the given path.
    pub fn inspect(path: impl AsRef<std::path::Path>) -> Result<String, crate::error::Error> {
        crate::read::inspect_path(path.as_ref())
    }
}

use std::{collections::HashMap, path::PathBuf};

/// All user settings that survive restarts.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Schema version — always written as 1; reserved for future migrations.
    pub version: u32,
    pub plot: PlotSettings,
    pub map: MapSettings,
    pub ui: UiSettings,
    pub processing: ProcessingSettings,
    pub storage: StorageSettings,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            version: 1,
            plot: PlotSettings::default(),
            map: MapSettings::default(),
            ui: UiSettings::default(),
            processing: ProcessingSettings::default(),
            storage: StorageSettings::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct StorageSettings {
    /// When `false`, NVD files are not automatically stored in the history
    /// database on load.  Existing data is not affected.
    pub enabled: bool,
}

impl Default for StorageSettings {
    fn default() -> Self {
        Self { enabled: true }
    }
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct PlotSettings {
    pub show_grid: bool,
    pub panel_visible: bool,
    /// Fraction of the window width given to the map panel (0.0–1.0).
    pub split_ratio: f32,
    /// Per-metric visibility. A missing key means the metric is visible (default `true`).
    pub metric: HashMap<MetricKind, bool>,
}

impl Default for PlotSettings {
    fn default() -> Self {
        Self {
            show_grid: true,
            panel_visible: true,
            split_ratio: 0.6,
            metric: HashMap::new(),
        }
    }
}

/// One variant per plot metric.
///
/// New variants can be added freely; old config files simply won't have the key,
/// and the apply step treats a missing entry as `true` (the default).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MetricKind {
    SatsSeen,
    SatsFix,
    GpsSeen,
    GpsFix,
    GlonassSeen,
    GlonassFix,
    GalileoSeen,
    GalileoFix,
    BeidouSeen,
    BeidouFix,
    Velocity,
    Eph,
    HeadingDeg,
    ClockDeltaMs,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct MapSettings {
    pub layer: MapLayerSetting,
    pub mapbox_token: String,
    pub sync_to_map: bool,
}

impl Default for MapSettings {
    fn default() -> Self {
        Self {
            layer: MapLayerSetting::Osm,
            mapbox_token: String::new(),
            sync_to_map: true,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MapLayerSetting {
    #[default]
    Osm,
    Satellite,
}

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct UiSettings {
    pub theme: ThemeSetting,
}

impl Default for UiSettings {
    fn default() -> Self {
        Self {
            theme: ThemeSetting::System,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeSetting {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct ProcessingSettings {
    /// Gap between consecutive GPS points that triggers a new trip segment, in seconds.
    pub track_split_gap_seconds: u64,
    /// Max seconds between a log entry timestamp and the nearest GPS fix for association.
    pub log_marker_window_s: u64,
}

impl Default for ProcessingSettings {
    fn default() -> Self {
        Self {
            track_split_gap_seconds: 300,
            log_marker_window_s: 60,
        }
    }
}

/// Returns the path to the GeoTrace config file, or `None` when the platform
/// config directory is unavailable (e.g. `HOME` unset on Linux).
pub fn settings_path() -> Option<PathBuf> {
    Some(dirs::config_dir()?.join("geotrace").join("config.toml"))
}

/// Load settings from disk, falling back to defaults on any error.
pub fn load_settings() -> Settings {
    let Some(path) = settings_path() else {
        log::warn!("Config directory unavailable — using defaults");
        return Settings::default();
    };
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Settings::default(); // absent on first run; not an error
    };
    match toml::from_str::<Settings>(&text) {
        Ok(s) => s,
        Err(e) => {
            log::warn!("Config parse error in {path:?}: {e:#} — using defaults");
            Settings::default()
        }
    }
}

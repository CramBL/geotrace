use std::{collections::HashMap, path::PathBuf};

/// All user settings that survive restarts.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Schema version - always written as 1. Reserved for future migrations.
    pub version: u32,
    pub plot: PlotSettings,
    pub map: MapSettings,
    pub ui: UiSettings,
    pub processing: ProcessingSettings,
    pub storage: StorageSettings,
    pub update: UpdateSettings,
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
            update: UpdateSettings::default(),
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct UpdateSettings {
    /// When `true`, GeoTrace checks for a newer release on startup and prompts
    /// the user. The check is also skipped in debug builds and when
    /// `GEOTRACE_OFFLINE` is set.
    pub check_on_startup: bool,
    /// A specific version the user chose to skip. The prompt stays hidden for
    /// exactly this version. Cleared automatically once a newer version appears.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skipped_version: Option<String>,
}

impl Default for UpdateSettings {
    fn default() -> Self {
        Self {
            check_on_startup: true,
            skipped_version: None,
        }
    }
}

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct StorageSettings {
    /// When `false`, GTD files are not automatically stored in the history
    /// database on load.  Existing data is not affected.
    pub enabled: bool,
    /// When `true`, the oldest recordings are automatically pruned after each
    /// import to keep the total stored size at or below `auto_prune_max_bytes`.
    pub auto_prune_enabled: bool,
    /// Maximum total size (in bytes) before automatic pruning kicks in.
    /// Default: 10 GiB.
    pub auto_prune_max_bytes: u64,
    /// When `true`, the user is prompted to confirm before pruning happens.
    pub auto_prune_confirm: bool,
}

impl Default for StorageSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            auto_prune_enabled: false,
            auto_prune_max_bytes: 10 * 1024 * 1024 * 1024,
            auto_prune_confirm: true,
        }
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
/// New variants can be added freely. Old config files simply won't have the key,
/// and the apply step treats a missing entry as `true` (the default).
///
/// Re-exported from `gt_types` rather than defined here, so the persisted
/// settings and the plot widget (`gt_plot::plot_widget`) share one definition
/// instead of maintaining matching copies by hand.
pub use gt_types::MetricKind;

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
    /// Gap between consecutive GPS points that triggers a new track segment, in seconds.
    pub track_split_gap_seconds: u64,
    /// Max seconds between a log entry timestamp and the nearest GPS fix for association.
    pub log_marker_window_s: u64,
    /// Whether to flag abrupt GPS/system clock-offset jumps as clock-discontinuity markers.
    pub detect_clock_discontinuities: bool,
    /// Sensitivity of the clock-discontinuity test, in robust standard deviations
    /// from the track's median step.  Lower is more sensitive.
    pub clock_discontinuity_sigmas: f64,
}

impl Default for ProcessingSettings {
    fn default() -> Self {
        Self {
            track_split_gap_seconds: 300,
            log_marker_window_s: 60,
            detect_clock_discontinuities: true,
            clock_discontinuity_sigmas: gt_track_builder::DEFAULT_CLOCK_OUTLIER_SIGMAS,
        }
    }
}

/// Returns the path to the GeoTrace config file, or `None` when the platform
/// config directory is unavailable (e.g. `HOME` unset on Linux).
pub fn settings_path() -> Option<PathBuf> {
    Some(dirs::config_dir()?.join("geotrace").join("config.toml"))
}

/// Load settings from a specific file path, falling back to defaults on any error.
pub fn load_settings_from(path: &std::path::Path) -> Settings {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Settings::default(); // absent on first run or read error; not an error
    };
    match toml::from_str::<Settings>(&text) {
        Ok(s) => s,
        Err(e) => {
            log::warn!("Config parse error in {path:?}: {e:#} - using defaults");
            Settings::default()
        }
    }
}

/// Load settings from disk, falling back to defaults on any error.
pub fn load_settings() -> Settings {
    let Some(path) = settings_path() else {
        log::warn!("Config directory unavailable - using defaults");
        return Settings::default();
    };
    load_settings_from(&path)
}

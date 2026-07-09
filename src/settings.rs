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
    pub analysis: AnalysisSettings,
    pub storage: StorageSettings,
    pub update: UpdateSettings,
    pub query: QuerySettings,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            version: 1,
            plot: PlotSettings::default(),
            map: MapSettings::default(),
            ui: UiSettings::default(),
            processing: ProcessingSettings::default(),
            analysis: AnalysisSettings::default(),
            storage: StorageSettings::default(),
            update: UpdateSettings::default(),
            query: QuerySettings::default(),
        }
    }
}

/// Persisted state for the query window: the history of run queries.
/// Examples are embedded in the binary, not persisted.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct QuerySettings {
    /// Previously run queries, newest first. See `app::query` for the
    /// dedup/pin/cap rules that maintain this list.
    pub history: Vec<QueryHistoryEntry>,
}

/// One remembered query: its text, whether the user pinned it against
/// eviction, and when it last ran.
///
/// Stored verbatim - an entry that no longer parses after a language change
/// simply shows a parse error when loaded, it is never dropped on load.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct QueryHistoryEntry {
    pub text: String,
    pub pinned: bool,
    /// When this query last ran, as Unix milliseconds. Stored as a plain
    /// integer because chrono's serde support is off workspace-wide.
    pub last_run_unix_ms: i64,
}

/// Parameters for the derived satellite-analysis plots (utilization rate and
/// loss-of-lock slip rate).
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct AnalysisSettings {
    /// Elevation mask, in degrees, applied to the "in view" baseline of the
    /// satellite utilization rate and to slip detection.
    pub elevation_mask_deg: f32,
    /// Whether to mark epochs where a used satellite falls below the mask.
    pub mark_masked_fix: bool,
    /// SNR drop, in dB-Hz between consecutive epochs, above which a still-tracked
    /// satellite counts as having slipped.
    pub snr_drop_db: f32,
    /// Trailing window, in minutes, over which the slip rate is averaged.
    pub slip_window_min: f32,
}

impl Default for AnalysisSettings {
    fn default() -> Self {
        Self {
            elevation_mask_deg: gt_plot::DEFAULT_ELEVATION_MASK_DEG,
            mark_masked_fix: true,
            snr_drop_db: gt_plot::DEFAULT_SNR_DROP_DB,
            slip_window_min: gt_plot::DEFAULT_SLIP_WINDOW_MIN,
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
    /// Stroke width of the plot lines. Clamped to
    /// `gt_plot::PLOT_LINE_WIDTH_RANGE` when applied.
    pub line_width: f32,
    pub panel_visible: bool,
    /// Fraction of the window width given to the map panel (0.0–1.0).
    pub split_ratio: f32,
    /// Per-metric visibility. A missing key means the metric is visible (default `true`).
    pub metric: HashMap<MetricKind, bool>,
    /// Per-channel visibility, keyed by channel name. Missing means visible,
    /// like `metric`. Names are global across files: an `accel` hidden once
    /// stays hidden in the next recording carrying an `accel`.
    pub channel: HashMap<String, bool>,
    /// Whether the advanced analysis chips (satellite utilization) are revealed.
    /// Off by default - those metrics are hidden until the user opts in.
    pub show_advanced_metrics: bool,
    /// Whether the ad-hoc channel chips are revealed. Off by default, like
    /// the advanced section.
    pub show_channels: bool,
}

impl Default for PlotSettings {
    fn default() -> Self {
        Self {
            show_grid: true,
            line_width: gt_plot::DEFAULT_PLOT_LINE_WIDTH,
            panel_visible: true,
            split_ratio: 0.6,
            metric: HashMap::new(),
            channel: HashMap::new(),
            show_advanced_metrics: false,
            show_channels: false,
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
    /// The display toggles' per-category visibility. Serialized as the
    /// list of hidden categories: an absent key or empty list shows
    /// everything, and categories added later default to visible on old
    /// config files.
    pub display_mask: gt_ui_types::DisplayMask,
}

impl Default for MapSettings {
    fn default() -> Self {
        Self {
            layer: MapLayerSetting::Osm,
            mapbox_token: String::new(),
            sync_to_map: true,
            display_mask: gt_ui_types::DisplayMask::default(),
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
    /// Whether to emit a marker when the GNSS fix drops.
    pub detect_gnss_fix_lost: bool,
    /// Whether to emit a marker when the GNSS fix returns.
    pub detect_gnss_fix_regained: bool,
    /// Whether to flag abrupt GPS/system clock-offset jumps as clock-discontinuity markers.
    pub detect_clock_discontinuities: bool,
    /// Sensitivity of the clock-discontinuity test, in robust standard deviations
    /// from the track's median step.  Lower is more sensitive.
    pub clock_discontinuity_sigmas: f64,
    /// Whether to flag loss-of-lock (cycle slip) events as markers.  Slip
    /// detection reuses the elevation mask and SNR-drop threshold from
    /// [`AnalysisSettings`], so markers and the slip-rate plot stay consistent.
    pub detect_slips: bool,
}

impl Default for ProcessingSettings {
    fn default() -> Self {
        Self {
            track_split_gap_seconds: 300,
            log_marker_window_s: 60,
            detect_gnss_fix_lost: true,
            detect_gnss_fix_regained: true,
            detect_clock_discontinuities: true,
            clock_discontinuity_sigmas: gt_track_builder::DEFAULT_CLOCK_OUTLIER_SIGMAS,
            detect_slips: true,
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

use std::time::{Duration, Instant};

/// `f32` stored as its bit pattern so `AppSnapshot` can derive `PartialEq`
/// without triggering the `float_cmp` lint.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct StableF32(u32);

impl From<f32> for StableF32 {
    fn from(v: f32) -> Self {
        Self(v.to_bits())
    }
}

impl From<StableF32> for f32 {
    fn from(s: StableF32) -> Self {
        f32::from_bits(s.0)
    }
}

/// `f64` stored as its bit pattern so `AppSnapshot` can derive `PartialEq`
/// without triggering the `float_cmp` lint.
///
/// Dirty-check helper only: it compares raw bits, so it does not treat
/// `+0.0`/`-0.0` as equal or canonicalise `NaN`.  Inputs here are range-clamped
/// finite settings values, so neither case arises.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct StableF64(u64);

impl From<f64> for StableF64 {
    fn from(v: f64) -> Self {
        Self(v.to_bits())
    }
}

/// Compact snapshot of all settings-relevant app state.
#[derive(PartialEq)]
pub(super) struct AppSnapshot {
    pub show_grid: bool,
    pub panel_visible: bool,
    pub split_ratio: StableF32,
    pub metric_sats_seen: bool,
    pub metric_sats_fix: bool,
    pub metric_gps_seen: bool,
    pub metric_gps_fix: bool,
    pub metric_glonass_seen: bool,
    pub metric_glonass_fix: bool,
    pub metric_galileo_seen: bool,
    pub metric_galileo_fix: bool,
    pub metric_beidou_seen: bool,
    pub metric_beidou_fix: bool,
    pub metric_velocity: bool,
    pub metric_eph: bool,
    pub metric_heading_deg: bool,
    pub metric_clock_delta_ms: bool,
    pub layer: crate::settings::MapLayerSetting,
    pub mapbox_token: String,
    pub sync_to_map: bool,
    pub theme: crate::settings::ThemeSetting,
    pub track_split_gap_seconds: u64,
    pub log_marker_window_s: u64,
    pub detect_clock_discontinuities: bool,
    pub clock_discontinuity_sigmas: StableF64,
    pub storage_enabled: bool,
    pub auto_prune_enabled: bool,
    pub auto_prune_max_bytes: u64,
    pub auto_prune_confirm: bool,
    pub update_check_on_startup: bool,
    pub skipped_version: Option<String>,
}

impl Default for AppSnapshot {
    fn default() -> Self {
        // The clock-discontinuity defaults are sourced from the persisted
        // settings so this dirty-check baseline cannot drift from what is loaded.
        let processing = crate::settings::ProcessingSettings::default();
        Self {
            show_grid: true,
            panel_visible: true,
            split_ratio: StableF32::from(0.6_f32),
            metric_sats_seen: true,
            metric_sats_fix: true,
            metric_gps_seen: true,
            metric_gps_fix: true,
            metric_glonass_seen: true,
            metric_glonass_fix: true,
            metric_galileo_seen: true,
            metric_galileo_fix: true,
            metric_beidou_seen: true,
            metric_beidou_fix: true,
            metric_velocity: true,
            metric_eph: true,
            metric_heading_deg: true,
            metric_clock_delta_ms: true,
            layer: crate::settings::MapLayerSetting::Osm,
            mapbox_token: String::new(),
            sync_to_map: true,
            theme: crate::settings::ThemeSetting::System,
            track_split_gap_seconds: 300,
            log_marker_window_s: 60,
            detect_clock_discontinuities: processing.detect_clock_discontinuities,
            clock_discontinuity_sigmas: StableF64::from(processing.clock_discontinuity_sigmas),
            storage_enabled: true,
            auto_prune_enabled: false,
            auto_prune_max_bytes: 10 * 1024 * 1024 * 1024,
            auto_prune_confirm: true,
            update_check_on_startup: true,
            skipped_version: None,
        }
    }
}

/// Detects settings changes and drives debounced write-through to disk.
pub(super) struct ConfigManager {
    dirty: bool,
    last_changed: Option<Instant>,
    prev_snapshot: AppSnapshot,
}

impl ConfigManager {
    pub fn new(initial: AppSnapshot) -> Self {
        Self {
            dirty: false,
            last_changed: None,
            prev_snapshot: initial,
        }
    }

    /// Compare `current` against the previous snapshot, mark dirty if changed.
    pub fn sync(&mut self, current: AppSnapshot) {
        if current != self.prev_snapshot {
            self.prev_snapshot = current;
            self.dirty = true;
            self.last_changed = Some(Instant::now());
        }
    }

    /// Returns `true` and clears the dirty flag if a flush is due.
    ///
    /// The caller is responsible for performing the actual flush.
    pub fn take_flush(&mut self) -> bool {
        const DEBOUNCE: Duration = Duration::from_millis(500);
        if self.dirty && self.last_changed.is_some_and(|t| t.elapsed() >= DEBOUNCE) {
            self.dirty = false;
            true
        } else {
            false
        }
    }
}

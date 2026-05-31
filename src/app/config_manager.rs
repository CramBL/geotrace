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
    pub layer: crate::settings::MapLayerSetting,
    pub mapbox_token: String,
    pub sync_to_map: bool,
    pub theme: crate::settings::ThemeSetting,
    pub track_split_gap_seconds: u64,
    pub log_marker_window_s: u64,
}

impl Default for AppSnapshot {
    fn default() -> Self {
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
            layer: crate::settings::MapLayerSetting::Osm,
            mapbox_token: String::new(),
            sync_to_map: true,
            theme: crate::settings::ThemeSetting::System,
            track_split_gap_seconds: 300,
            log_marker_window_s: 60,
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

    /// Compare `current` against the previous snapshot; mark dirty if changed.
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

use std::time::{Duration, Instant};
use strum::EnumCount;

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
    pub line_width: StableF32,
    pub panel_visible: bool,
    pub split_ratio: StableF32,
    /// Per-metric visibility flags in `MetricKind::iter()` order, sized by
    /// `MetricKind::COUNT` so a new variant is picked up automatically with no
    /// edit here.  This is purely an in-run dirty-check key: on-disk settings are
    /// keyed by stable wire names, never by position, so tracking the enum's
    /// order and length here is safe across add/remove/rename/reorder.
    pub metrics: [bool; crate::settings::MetricKind::COUNT],
    pub show_advanced_metrics: bool,
    /// The toggled channel visibilities, sorted by name. Channels are dynamic
    /// per-file names, so unlike `metrics` this cannot be a fixed array.
    pub channels: Vec<(String, bool)>,
    pub show_channels: bool,
    pub layer: crate::settings::MapLayerSetting,
    pub mapbox_token: String,
    pub sync_to_map: bool,
    pub theme: crate::settings::ThemeSetting,
    pub track_split_gap_seconds: u64,
    pub log_marker_window_s: u64,
    pub detect_gnss_fix_lost: bool,
    pub detect_gnss_fix_regained: bool,
    pub detect_clock_discontinuities: bool,
    pub clock_discontinuity_sigmas: StableF64,
    pub detect_slips: bool,
    pub elevation_mask_deg: StableF32,
    pub mark_masked_fix: bool,
    pub snr_drop_db: StableF32,
    pub slip_window_min: StableF32,
    pub storage_enabled: bool,
    pub auto_prune_enabled: bool,
    pub auto_prune_max_bytes: u64,
    pub auto_prune_confirm: bool,
    pub update_check_on_startup: bool,
    pub skipped_version: Option<String>,
    /// Query-history mutation counter. The history is a growing `Vec` that
    /// this flat snapshot cannot compare directly against, so the window
    /// bumps this counter on every change and the dirty-check watches the
    /// number.
    pub query_history_revision: u64,
}

impl Default for AppSnapshot {
    fn default() -> Self {
        // The clock-discontinuity and analysis defaults are sourced from the
        // persisted settings so this dirty-check baseline cannot drift from what
        // is loaded.
        let processing = crate::settings::ProcessingSettings::default();
        let analysis = crate::settings::AnalysisSettings::default();
        Self {
            show_grid: true,
            line_width: StableF32::from(gt_plot::DEFAULT_PLOT_LINE_WIDTH),
            panel_visible: true,
            split_ratio: StableF32::from(0.6_f32),
            metrics: [true; crate::settings::MetricKind::COUNT],
            show_advanced_metrics: false,
            channels: Vec::new(),
            show_channels: false,
            layer: crate::settings::MapLayerSetting::Osm,
            mapbox_token: String::new(),
            sync_to_map: true,
            theme: crate::settings::ThemeSetting::System,
            track_split_gap_seconds: 300,
            log_marker_window_s: 60,
            detect_gnss_fix_lost: processing.detect_gnss_fix_lost,
            detect_gnss_fix_regained: processing.detect_gnss_fix_regained,
            detect_clock_discontinuities: processing.detect_clock_discontinuities,
            clock_discontinuity_sigmas: StableF64::from(processing.clock_discontinuity_sigmas),
            detect_slips: processing.detect_slips,
            elevation_mask_deg: StableF32::from(analysis.elevation_mask_deg),
            mark_masked_fix: analysis.mark_masked_fix,
            snr_drop_db: StableF32::from(analysis.snr_drop_db),
            slip_window_min: StableF32::from(analysis.slip_window_min),
            storage_enabled: true,
            auto_prune_enabled: false,
            auto_prune_max_bytes: 10 * 1024 * 1024 * 1024,
            auto_prune_confirm: true,
            update_check_on_startup: true,
            skipped_version: None,
            query_history_revision: 0,
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

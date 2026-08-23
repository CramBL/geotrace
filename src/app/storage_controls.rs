//! The auto-store and auto-prune controls, rendered by the History window and
//! by the settings window's Application page. Each control is grayed with a
//! hover text naming what to enable first while auto-storing or auto-pruning
//! is off, and every one of them is grayed in a read-only session, which
//! stores and prunes nothing.

use egui::{Checkbox, DragValue};
use gt_pending_writes::WriteAccess;

use crate::app::read_only_session::READ_ONLY_RECORDING_HISTORY_HOVER;
use crate::settings::StorageSettings;

pub const AUTO_STORE_LABEL: &str = "Auto-store recordings";

const ENABLE_AUTO_STORE_FIRST: &str = "Enable 'Auto-store recordings' to use auto-pruning";

const ENABLE_AUTO_PRUNE_FIRST: &str = "Tick 'Auto-prune when over' to configure this";

pub fn show_auto_store_checkbox(
    ui: &mut egui::Ui,
    storage: &mut StorageSettings,
    write_access: WriteAccess,
) {
    ui.add_enabled(
        write_access.allows_writing(),
        Checkbox::new(&mut storage.enabled, AUTO_STORE_LABEL),
    )
    .on_hover_text(
        "Store every loaded recording in the history database. Turning it off leaves \
         already stored recordings in place.",
    )
    .on_disabled_hover_text(READ_ONLY_RECORDING_HISTORY_HOVER);
}

/// The auto-prune switch and the total stored size it acts on.
pub fn show_auto_prune_limit(
    ui: &mut egui::Ui,
    storage: &mut StorageSettings,
    write_access: WriteAccess,
) {
    ui.horizontal(|ui| {
        let writes_recordings = write_access.allows_writing();
        let storage_on = storage.enabled && writes_recordings;
        let prune_on = storage.auto_prune_enabled && storage_on;

        ui.add_enabled(
            storage_on,
            Checkbox::new(&mut storage.auto_prune_enabled, "Auto-prune when over"),
        )
        .on_hover_text(
            "Automatically delete the oldest recordings when storage exceeds the threshold",
        )
        .on_disabled_hover_text(if writes_recordings {
            ENABLE_AUTO_STORE_FIRST
        } else {
            READ_ONLY_RECORDING_HISTORY_HOVER
        });

        let mut max_gb = storage.auto_prune_max_bytes as f64 / gt_fmt::BYTES_PER_GB as f64;
        ui.add_enabled(
            prune_on,
            DragValue::new(&mut max_gb).range(0.1..=1_000.0).speed(0.1),
        )
        .on_hover_text("Storage limit - oldest recordings are pruned when this is exceeded")
        .on_disabled_hover_text(if !writes_recordings {
            READ_ONLY_RECORDING_HISTORY_HOVER
        } else if storage_on {
            ENABLE_AUTO_PRUNE_FIRST
        } else {
            ENABLE_AUTO_STORE_FIRST
        });

        if prune_on {
            #[expect(
                clippy::cast_sign_loss,
                reason = "DragValue range is 0.1..=1000 so value is always positive"
            )]
            let bytes = (max_gb * gt_fmt::BYTES_PER_GB as f64).round() as u64;
            storage.auto_prune_max_bytes = bytes;
        }

        ui.label("GB");
    });
}

pub fn show_auto_prune_confirm_checkbox(
    ui: &mut egui::Ui,
    storage: &mut StorageSettings,
    write_access: WriteAccess,
) {
    let writes_recordings = write_access.allows_writing();
    let prune_on = storage.auto_prune_enabled && storage.enabled && writes_recordings;
    ui.add_enabled(
        prune_on,
        Checkbox::new(&mut storage.auto_prune_confirm, "Confirm before pruning"),
    )
    .on_hover_text("Show a confirmation dialog before auto-pruning deletes recordings")
    .on_disabled_hover_text(if !writes_recordings {
        READ_ONLY_RECORDING_HISTORY_HOVER
    } else if storage.enabled {
        ENABLE_AUTO_PRUNE_FIRST
    } else {
        ENABLE_AUTO_STORE_FIRST
    });
}

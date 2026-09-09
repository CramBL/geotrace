use gt_history_types::{DatabaseRef, LogAttachmentId};

use crate::LayerColorSlot;
use crate::attachment::LogAttachmentRef;
use crate::loaded_log::{LoadedLogId, LoadedLogs};

pub fn attachment_ref() -> LogAttachmentRef {
    LogAttachmentRef {
        recording: DatabaseRef {
            identity: "nav-devkit-mk2".to_owned(),
            group_name: "2026-01-01T14-02-11".to_owned(),
        },
        id: LogAttachmentId::new_random(),
    }
}

/// Waits for every log's filter scans, as the viewer's per-frame polling
/// does once they land.
pub fn wait_for_scans(logs: &mut LoadedLogs) {
    let ids: Vec<LoadedLogId> = logs.iter_with_ids().map(|(id, _)| id).collect();
    for id in ids {
        if let Some((stack, _)) = logs.filter_stack_mut_by_id(id) {
            stack.wait_for_queries();
        }
    }
}

/// Adds the live filter of the log `id` names as a layer chip, and returns
/// the palette colour that chip took.
pub fn add_layer_chip(logs: &mut LoadedLogs, id: LoadedLogId, text: &str) -> Option<usize> {
    let (stack, slots) = logs.filter_stack_mut_by_id(id)?;
    stack.set_live_filter_text(text);
    let chip = stack.add_live_filter_as_chip(slots)?;
    stack.chip(chip)?.layer_slot().map(LayerColorSlot::index)
}

/// The palette colour the first chip of the log `id` names draws in.
pub fn first_chip_slot(logs: &LoadedLogs, id: LoadedLogId) -> Option<usize> {
    logs.get_by_id(id)?
        .filters()
        .chips()
        .first()?
        .layer_slot()
        .map(LayerColorSlot::index)
}

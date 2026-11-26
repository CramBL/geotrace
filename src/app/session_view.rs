use gt_map::{MapLayer, SatelliteLayerAccess};
use gt_session_bridge::{ErrorCode, Reply, SessionMapLayer, SetSessionView, TimeFilterChange};
use rustc_hash::FxHashSet;
use uuid::Uuid;

use super::App;
use super::mcp_action_notice::McpActionNotice;

impl App {
    pub(super) fn set_session_view(
        &mut self,
        session: Uuid,
        SetSessionView {
            base_layer,
            display_categories,
            time_filter,
        }: SetSessionView,
    ) -> Result<Reply, ErrorCode> {
        if self.shutdown_requested() {
            return Err(ErrorCode::ShuttingDown);
        }
        let layer = base_layer.map(|layer| match layer {
            SessionMapLayer::OpenStreetMap => MapLayer::OpenStreetMap,
            SessionMapLayer::Satellite => MapLayer::Satellite,
        });
        if layer.is_some_and(|layer| {
            !self
                .map
                .can_select_layer(layer, SatelliteLayerAccess::TokenRequired)
        }) {
            return Err(ErrorCode::MapboxTokenRequired);
        }
        let time_window = time_filter.map(TimeFilterChange::window).transpose()?;
        let (mask_changed, time_changed) = {
            let mut shared = self.shared.borrow_mut();
            let mut mask = shared.display_mask;
            let mut edited_categories = FxHashSet::default();
            for edit in display_categories {
                if !edited_categories.insert(edit.category) {
                    return Err(ErrorCode::DuplicateDisplayCategory);
                }
                mask.set_visible(edit.category, edit.visible);
            }

            let mask_changed = shared.display_mask != mask;
            shared.display_mask = mask;
            let changed_window =
                time_window.filter(|window| *window != shared.filter.time_window());
            if let Some(window) = changed_window {
                shared.filter.set_time_window(window);
                shared.filter_state.secondary_zoom = None;
            }
            (mask_changed, changed_window.is_some())
        };
        let changed_layer = layer.filter(|layer| *layer != self.map.layer());
        if let Some(layer) = changed_layer {
            self.map.set_layer(layer);
        }
        let map_changed = mask_changed || changed_layer.is_some();
        if map_changed {
            self.sync_settings_autosave();
        }
        let notice = match (map_changed, time_changed) {
            (true, true) => Some(McpActionNotice::ChangeMapAndTimeFilter),
            (true, false) => Some(McpActionNotice::ChangeMapView),
            (false, true) => Some(McpActionNotice::ChangeTimeFilter),
            (false, false) => None,
        };
        if let Some(notice) = notice {
            notice.enqueue(&mut self.toasts);
            self.ctx.request_repaint();
        }
        Ok(Reply::SessionState {
            state: self.session_state(session),
        })
    }
}

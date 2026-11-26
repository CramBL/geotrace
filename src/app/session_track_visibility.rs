use std::collections::BTreeSet;

use gt_session_bridge::{
    ErrorCode, Reply, SetTrackVisibility, TrackVisibilityAction, TrackVisibilityResult,
};
use uuid::Uuid;

use super::App;
use super::mcp_action_notice::McpActionNotice;

impl App {
    pub(super) fn set_session_track_visibility(
        &mut self,
        session: Uuid,
        SetTrackVisibility { action, tracks }: &SetTrackVisibility,
    ) -> Result<Reply, ErrorCode> {
        if self.shutdown_requested() {
            return Err(ErrorCode::ShuttingDown);
        }
        let mut shared = self.shared.borrow_mut();
        let files = shared.loaded_files.view();
        let targets = tracks
            .as_ref()
            .iter()
            .map(|reference| {
                let track = reference.resolve(files, session)?;
                shared.tree.track_node(track).ok_or(ErrorCode::StaleTrack)?;
                Ok(track)
            })
            .collect::<Result<BTreeSet<_>, ErrorCode>>()?;
        let targeted_tracks = targets.len();
        let changes = match action {
            TrackVisibilityAction::Hide => shared.tree.set_tracks_enabled(targets, false),
            TrackVisibilityAction::Show => shared.tree.set_tracks_enabled(targets, true),
            TrackVisibilityAction::Solo => shared.tree.show_only_tracks(targets),
        };
        drop(shared);
        self.store_the_hidden_tracks_the_user_changed();
        if changes.any() {
            McpActionNotice::ChangeTrackVisibility.enqueue(&mut self.toasts);
        }
        Ok(Reply::TrackVisibility(TrackVisibilityResult {
            action: *action,
            targeted_tracks,
            changed_tracks: changes.tracks,
        }))
    }
}

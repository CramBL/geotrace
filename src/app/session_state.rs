use gt_map::MapLayer;
use gt_session_bridge::{
    OpenPoint, SessionMapBounds, SessionMapLayer, SessionMapState, SessionPanel, SessionState,
    TrackReference,
};
use gt_ui_types::DisplayCategory;
use strum::IntoEnumIterator as _;
use uuid::Uuid;

use super::{App, SharedAppState};

impl App {
    pub(super) fn session_state(&self, session: Uuid) -> SessionState {
        let shared = self.shared.borrow();
        let files = shared.loaded_files.view();
        SessionState {
            recording_count: files.len(),
            recording_preview: SessionState::recording_preview(
                files,
                session,
                &shared.recording_name_template,
            ),
            open_panels: SessionPanel::iter()
                .filter(|panel| match panel {
                    SessionPanel::History => self.history_window.open,
                    SessionPanel::LogViewer => self.log_viewer.open,
                    SessionPanel::Plot => self.plot_is_visible(),
                    SessionPanel::Query => self.query_window.open,
                    SessionPanel::Settings => self.settings_open,
                    SessionPanel::SkyTrails => self.sky_trails_window.is_open(),
                })
                .collect(),
            time_filter: shared.filter.time_window().into(),
            map: SessionMapState {
                base_layer: match self.map.layer() {
                    MapLayer::OpenStreetMap => SessionMapLayer::OpenStreetMap,
                    MapLayer::Satellite => SessionMapLayer::Satellite,
                },
                visible_categories: DisplayCategory::iter()
                    .filter(|&category| shared.display_mask.is_visible(category))
                    .collect(),
                bounds: self.session_map_bounds(),
                pinned_point: shared.session_pinned_point(session),
            },
            query: self.editor_query.session_state(session),
        }
    }

    fn session_map_bounds(&self) -> Option<SessionMapBounds> {
        let bounds = self.map.viewport_geo_bounds()?;
        let laid_out = self.tiles_tree.tiles.is_visible(self.map_tile_id);
        let finite = [
            bounds.lat_min,
            bounds.lat_max,
            bounds.lon_min,
            bounds.lon_max,
        ]
        .into_iter()
        .all(f64::is_finite);
        (laid_out && finite).then_some(SessionMapBounds {
            south_deg: bounds.lat_min,
            north_deg: bounds.lat_max,
            west_deg: bounds.lon_min,
            east_deg: bounds.lon_max,
        })
    }
}

impl SharedAppState {
    fn session_pinned_point(&self, session: Uuid) -> Option<OpenPoint> {
        let element = self.highlight.sticky()?;
        let fix = element.fix()?;
        let files = self.loaded_files.view();
        element.resolve(files.files())?;
        Some(OpenPoint {
            track: TrackReference::new(session, files.session_track_ref(fix.track)?),
            point: fix.point.as_usize(),
        })
    }
}

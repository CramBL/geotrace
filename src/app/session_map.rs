use std::ops::Range;

use gt_filter::GlobalFilter;
use gt_loaded_files::RecordingData;
use gt_map::{NavMap, PresentationCamera};
use gt_query_run::MatchExtent;
use gt_session_bridge::{ErrorCode, OpenPoint, QueryJobId, RangeTarget, ShowOnMap};
use gt_types::{DataCategory, FixRef, Generation, Latitude, Longitude, PointIdx, TrackRef};
use gt_ui_types::{
    DisplayCategory, MapElementRef, MapEligibility, MapPresence, MapPresenceResult, MatchHighlight,
    PinRevision, QueryMatches,
};
use uuid::Uuid;

use super::session_query::SessionQuery;
use super::{App, SharedAppState};

#[derive(Clone, Copy)]
pub(super) struct MapPresentation {
    target: PresentationTarget,
    inputs: MapPresentationInputs,
}

pub(super) struct QueuedMapPresentation {
    presentation: MapPresentation,
    pin_revision: Option<PinRevision>,
}

#[derive(Clone, Copy)]
pub(super) struct AppliedMapPresentation {
    presentation: MapPresentation,
    owned_pin_revision: Option<PinRevision>,
}

impl MapPresentation {
    fn new(
        target: PresentationTarget,
        shared: &SharedAppState,
        query_matches: Option<&QueryMatches>,
    ) -> Self {
        Self {
            target,
            inputs: MapPresentationInputs::of(shared, query_matches),
        }
    }

    fn track_range(
        shared: &SharedAppState,
        query_matches: Option<&QueryMatches>,
        track: TrackRef,
        points: Range<usize>,
        job: Option<QueryJobId>,
    ) -> Result<Self, ErrorCode> {
        if !gt_map::query_match_has_geometry(shared.map_presence(query_matches), track, &points) {
            return Err(ErrorCode::NoVisibleMapGeometry);
        }
        Ok(Self::new(
            PresentationTarget::Range {
                highlight: MatchHighlight::new(track, &points),
                job,
            },
            shared,
            query_matches,
        ))
    }

    fn is_current(
        self,
        shared: &SharedAppState,
        query_matches: Option<&QueryMatches>,
        queries: &SessionQuery,
    ) -> bool {
        if self.inputs != MapPresentationInputs::of(shared, query_matches) {
            return false;
        }
        let scope = shared.map_presence(query_matches);
        match self.target {
            PresentationTarget::Point { fix, .. } => scope.draws(MapElementRef::Fix(fix)),
            PresentationTarget::Range { highlight, job } => {
                job.is_none_or(|id| queries.has_current_results(id, shared.loaded_files.view()))
                    && scope
                        .eligibility()
                        .prepare_track(highlight.track)
                        .category_visible(DataCategory::Track)
            }
        }
    }

    fn can_apply(
        self,
        shared: &SharedAppState,
        query_matches: Option<&QueryMatches>,
        queries: &SessionQuery,
    ) -> bool {
        self.is_current(shared, query_matches, queries)
            && (matches!(self.target, PresentationTarget::Point { .. })
                || shared
                    .display_mask
                    .is_visible(DisplayCategory::QueryHighlights))
    }
}

#[derive(Clone, Copy)]
enum PresentationTarget {
    Point {
        fix: FixRef,
        latitude: Latitude,
        longitude: Longitude,
    },
    Range {
        highlight: MatchHighlight,
        job: Option<QueryJobId>,
    },
}

#[derive(Clone, Copy, PartialEq)]
struct MapPresentationInputs {
    generation: Generation<RecordingData>,
    filter: GlobalFilter,
    editor_run: Option<u64>,
}

impl MapPresentationInputs {
    fn of(shared: &SharedAppState, query_matches: Option<&QueryMatches>) -> Self {
        Self {
            generation: shared.loaded_files.data_generation(),
            filter: shared.filter,
            editor_run: query_matches.map(|matches| matches.run),
        }
    }
}

impl App {
    pub(super) fn session_open_point(
        &self,
        session: Uuid,
        OpenPoint { track, point }: OpenPoint,
    ) -> Result<MapPresentation, ErrorCode> {
        let shared = self.shared.borrow();
        let files = shared.loaded_files.view();
        let track = track.resolve(files, session)?;
        let loaded = track.resolve(files.files()).ok_or(ErrorCode::StaleTrack)?;
        if point >= loaded.points.len() {
            return Err(ErrorCode::InvalidPoint);
        }
        let fix = FixRef::new(track, PointIdx::new(point));
        let query_matches = self.query_window.map_matches(files);
        if !shared
            .map_presence(query_matches)
            .draws(MapElementRef::Fix(fix))
        {
            return Err(ErrorCode::NoVisibleMapGeometry);
        }
        let (latitude, longitude) = loaded
            .resolved_position_at(point)
            .ok_or(ErrorCode::NoVisibleMapGeometry)?;
        Ok(MapPresentation::new(
            PresentationTarget::Point {
                fix,
                latitude,
                longitude,
            },
            &shared,
            query_matches,
        ))
    }

    pub(super) fn session_show_on_map(
        &mut self,
        queries: &mut SessionQuery,
        session: Uuid,
        ShowOnMap { target }: ShowOnMap,
    ) -> Result<MapPresentation, ErrorCode> {
        let (track, points, query_job) = match target {
            RangeTarget::Track { track, start, end } => {
                let shared = self.shared.borrow();
                let files = shared.loaded_files.view();
                let (track, points) = track.resolve_range(files, session, start..end)?;
                (track, points, None)
            }
            RangeTarget::QueryMatch {
                job,
                query,
                matched,
            } => {
                let run = queries.result_session(self, session, job)?;
                let shared = self.shared.borrow();
                let results = run.retained_results(shared.loaded_files.view())?;
                let matched = results.query(query)?.match_at(matched.into())?;
                let MatchExtent::Points(points) = matched.extent() else {
                    return Err(ErrorCode::QueryMatchIsChannel);
                };
                (matched.track(), points.clone(), Some(job))
            }
        };
        let shared = self.shared.borrow();
        let query_matches = self.query_window.map_matches(shared.loaded_files.view());
        MapPresentation::track_range(&shared, query_matches, track, points, query_job)
    }
}

impl SharedAppState {
    pub(super) fn queue_session_map(&mut self, presentation: MapPresentation) {
        self.map_center_request = None;
        self.zoom_to_visible_request = false;
        self.reveal_query_matches_request = None;
        let pin_revision = matches!(presentation.target, PresentationTarget::Point { .. })
            .then(|| self.highlight.pin_revision());
        self.pending_session_map = Some(QueuedMapPresentation {
            presentation,
            pin_revision,
        });
    }

    pub(super) fn discard_stale_session_map(
        &mut self,
        map: &mut NavMap,
        query_matches: Option<&QueryMatches>,
        queries: &SessionQuery,
    ) {
        // Queuing a session request clears the earlier GUI camera requests.
        let later_gui_camera = self.map_center_request.is_some()
            || self.zoom_to_visible_request
            || self.reveal_query_matches_request.is_some();
        let pending = self.pending_session_map.take().filter(|pending| {
            !later_gui_camera
                && pending
                    .pin_revision
                    .is_none_or(|revision| revision == self.highlight.pin_revision())
                && pending.presentation.can_apply(self, query_matches, queries)
        });
        if let Some(active) = self.presented_session_map
            && (!active.presentation.is_current(self, query_matches, queries)
                || active.owned_pin_revision.is_some_and(|revision| {
                    revision != self.highlight.pin_revision()
                        && map.pin_rollback_revision(&self.highlight) != Some(revision)
                }))
        {
            self.clear_session_map(map);
        }
        // Clearing an applied session pin can change the revision after arbitration.
        self.pending_session_map = pending.map(|mut pending| {
            pending.pin_revision = pending.pin_revision.map(|_| self.highlight.pin_revision());
            pending
        });
    }

    pub(super) fn apply_session_map(
        &mut self,
        map: &mut NavMap,
        query_matches: Option<&QueryMatches>,
    ) -> Option<PresentationCamera> {
        let presentation = self.pending_session_map.take()?.presentation;
        self.clear_session_map(map);
        let (camera, owned_pin_revision) = match presentation.target {
            PresentationTarget::Point {
                fix,
                latitude,
                longitude,
            } => {
                let element = MapElementRef::Fix(fix);
                let camera = PresentationCamera::Center {
                    latitude,
                    longitude,
                };
                if self.highlight.is_sticky(element) {
                    return Some(camera);
                }
                let MapPresenceResult::Present(present) =
                    self.map_presence(query_matches).resolve(element)
                else {
                    return None;
                };
                let mut highlight = self.highlight;
                highlight.pin(present);
                self.highlight = highlight;
                self.popup_pos_request = None;
                (camera, Some(self.highlight.pin_revision()))
            }
            PresentationTarget::Range { highlight, .. } => {
                self.highlight.presented_match = Some(highlight);
                (PresentationCamera::TrackRange(highlight), None)
            }
        };
        self.presented_session_map = Some(AppliedMapPresentation {
            presentation,
            owned_pin_revision,
        });
        Some(camera)
    }

    fn clear_session_map(&mut self, map: &mut NavMap) {
        if let Some(active) = self.presented_session_map.take() {
            match active.presentation.target {
                PresentationTarget::Point { .. } => {
                    let Some(owned) = active.owned_pin_revision else {
                        return;
                    };
                    map.discard_pin_rollback(owned);
                    if owned == self.highlight.pin_revision() {
                        self.highlight.clear_sticky();
                    }
                }
                PresentationTarget::Range { .. } => self.highlight.presented_match = None,
            }
        }
    }

    fn map_presence<'a>(&'a self, query_matches: Option<&'a QueryMatches>) -> MapPresence<'a> {
        MapEligibility::new(
            self.loaded_files.files(),
            self.tree.visibility(),
            &self.filter,
            query_matches,
            self.tree.generated_marker_visibility(),
            self.tree.event_marker_visibility(),
        )
        .with_display_mask(self.display_mask)
    }
}

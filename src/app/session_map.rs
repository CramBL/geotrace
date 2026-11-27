use std::ops::Range;

use gt_filter::GlobalFilter;
use gt_loaded_files::RecordingData;
use gt_map::{MapFrameInteraction, MapPresentationInput, NavMap, PresentationCamera};
use gt_query_run::MatchExtent;
use gt_session_bridge::{ErrorCode, OpenPoint, RangeTarget, ShowOnMap};
use gt_types::{DataCategory, FixRef, Generation, Latitude, Longitude, PointIdx, TrackRef};
use gt_ui_types::{
    DisplayCategory, MapElementRef, MapEligibility, MapPresence, MapPresenceResult, MatchHighlight,
    MatchRevealTarget, PinRevision, QueryMatches,
};
use uuid::Uuid;

use super::pane_presentation::{PanePresentation, PresentationDetail, PresentationQueryMatch};
use super::query_run::BackgroundQuery;
use super::{App, SharedAppState};

#[derive(Clone, Copy)]
pub(super) struct MapPresentation {
    id: Uuid,
    target: PresentationTarget,
    inputs: MapPresentationInputs,
}

pub(super) struct QueuedMapPresentation {
    presentation: MapPresentation,
    pin_revision: Option<PinRevision>,
    earlier_camera: Option<RetainedGuiCamera>,
}

#[derive(Default)]
pub(super) struct GuiCameraRequests {
    pub(super) center: Option<(f64, f64)>,
    pub(super) zoom_to_visible: bool,
    pub(super) reveal: Option<MatchRevealTarget>,
}

struct RetainedGuiCamera {
    requests: GuiCameraRequests,
    generation: Generation<RecordingData>,
}

#[derive(Clone, Copy)]
pub(super) struct AppliedMapPresentation {
    presentation: MapPresentation,
    pin: Option<PresentedPin>,
}

impl AppliedMapPresentation {
    pub(super) fn id(self) -> Uuid {
        self.presentation.id
    }

    pub(super) fn target(self) -> PanePresentation {
        match self.presentation.target {
            PresentationTarget::Point { fix, .. } => PanePresentation {
                track: fix.track,
                detail: PresentationDetail::Point(fix.point),
            },
            PresentationTarget::Range {
                highlight,
                query_match,
            } => PanePresentation {
                track: highlight.track,
                detail: query_match.map_or(
                    PresentationDetail::Points {
                        start: highlight.start,
                        end: highlight.end,
                    },
                    PresentationQueryMatch::detail,
                ),
            },
        }
    }
}

#[derive(Clone, Copy)]
struct PresentedPin {
    revision: PinRevision,
    owned: bool,
}

impl MapPresentation {
    fn new(
        target: PresentationTarget,
        shared: &SharedAppState,
        query_matches: Option<&QueryMatches>,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            target,
            inputs: MapPresentationInputs::of(shared, query_matches),
        }
    }

    fn track_range(
        shared: &SharedAppState,
        query_matches: Option<&QueryMatches>,
        track: TrackRef,
        points: Range<usize>,
        query_match: Option<PresentationQueryMatch>,
    ) -> Result<Self, ErrorCode> {
        if !shared
            .display_mask
            .is_visible(DisplayCategory::QueryHighlights)
        {
            return Err(ErrorCode::QueryHighlightsDisabled);
        }
        if !gt_map::query_match_has_geometry(shared.map_presence(query_matches), track, &points) {
            return Err(ErrorCode::NoVisibleMapGeometry);
        }
        Ok(Self::new(
            PresentationTarget::Range {
                highlight: MatchHighlight::new(track, &points),
                query_match,
            },
            shared,
            query_matches,
        ))
    }

    fn is_current(
        self,
        shared: &SharedAppState,
        query_matches: Option<&QueryMatches>,
        queries: &BackgroundQuery,
    ) -> bool {
        if self.inputs != MapPresentationInputs::of(shared, query_matches) {
            return false;
        }
        let scope = shared.map_presence(query_matches);
        match self.target {
            PresentationTarget::Point { fix, .. } => scope.draws(MapElementRef::Fix(fix)),
            PresentationTarget::Range {
                highlight,
                query_match,
            } => {
                query_match.is_none_or(|matched| {
                    queries.has_current_results(matched.job, shared.loaded_files.view())
                }) && scope
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
        queries: &BackgroundQuery,
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
        query_match: Option<PresentationQueryMatch>,
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
        let query_matches = self.editor_query.session().map_matches(files);
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
        session: Uuid,
        ShowOnMap { target }: ShowOnMap,
    ) -> Result<MapPresentation, ErrorCode> {
        let (track, points, query_match) = match target {
            RangeTarget::Track { track, start, end } => {
                let shared = self.shared.borrow();
                let files = shared.loaded_files.view();
                let (track, points) = track.resolve_range(files, session, start..end)?;
                (track, points, None)
            }
            RangeTarget::QueryMatch {
                job,
                query,
                matched: position,
            } => {
                self.require_completed_query(session, job)?;
                let shared = self.shared.borrow();
                let results = self
                    .editor_query
                    .session()
                    .retained_results(shared.loaded_files.view())?;
                let matched = results.query(query)?.match_at(position.into())?;
                let MatchExtent::Points(points) = matched.extent() else {
                    return Err(ErrorCode::QueryMatchIsChannel);
                };
                (
                    matched.track(),
                    points.clone(),
                    Some(PresentationQueryMatch {
                        job,
                        query,
                        match_ordinal: position.match_ordinal,
                    }),
                )
            }
        };
        let shared = self.shared.borrow();
        let query_matches = self
            .editor_query
            .session()
            .map_matches(shared.loaded_files.view());
        MapPresentation::track_range(&shared, query_matches, track, points, query_match)
    }
}

impl SharedAppState {
    fn has_gui_camera_requests(&self) -> bool {
        self.map_center_request.is_some()
            || self.zoom_to_visible_request
            || self.reveal_query_matches_request.is_some()
    }

    fn take_gui_camera_requests(&mut self) -> GuiCameraRequests {
        GuiCameraRequests {
            center: self.map_center_request.take(),
            zoom_to_visible: std::mem::take(&mut self.zoom_to_visible_request),
            reveal: self.reveal_query_matches_request.take(),
        }
    }

    pub(super) fn take_gui_camera_requests_for_draw(&mut self) -> GuiCameraRequests {
        if self.has_gui_camera_requests() {
            self.pending_session_map = None;
        }
        self.take_gui_camera_requests()
    }

    fn restore_earlier_camera(&mut self, pending: QueuedMapPresentation) {
        if let Some(earlier) = pending.earlier_camera
            && !self.has_gui_camera_requests()
        {
            self.map_center_request = earlier.requests.center;
            self.zoom_to_visible_request = earlier.requests.zoom_to_visible;
            if earlier.generation == self.loaded_files.data_generation() {
                self.reveal_query_matches_request = earlier.requests.reveal;
            }
        }
    }

    pub(super) fn queue_session_map(&mut self, presentation: MapPresentation) {
        let previous = self.pending_session_map.take();
        let earlier_camera = if self.has_gui_camera_requests() {
            Some(RetainedGuiCamera {
                requests: self.take_gui_camera_requests(),
                generation: self.loaded_files.data_generation(),
            })
        } else {
            previous.and_then(|pending| pending.earlier_camera)
        };
        let pin_revision = matches!(presentation.target, PresentationTarget::Point { .. })
            .then(|| self.highlight.pin_revision());
        self.pending_session_map = Some(QueuedMapPresentation {
            presentation,
            pin_revision,
            earlier_camera,
        });
    }

    pub(super) fn finish_session_map_input(
        &mut self,
        ctx: &egui::Context,
        input: MapPresentationInput,
        interaction: MapFrameInteraction,
    ) {
        if matches!(input, MapPresentationInput::Ready) {
            return;
        }
        if let Some(pending) = self.pending_session_map.take()
            && !matches!(interaction, MapFrameInteraction::NavigateCamera)
        {
            let point_superseded = pending.pin_revision.is_some_and(|revision| {
                matches!(interaction, MapFrameInteraction::DismissPoint)
                    || revision != self.highlight.pin_revision()
            });
            if point_superseded {
                self.restore_earlier_camera(pending);
            } else {
                self.pending_session_map = Some(pending);
            }
        }
        if matches!(input, MapPresentationInput::FrameInteraction) || self.has_gui_camera_requests()
        {
            ctx.request_repaint();
        }
    }

    fn can_apply_pending_map(
        &self,
        pending: &QueuedMapPresentation,
        query_matches: Option<&QueryMatches>,
        queries: &BackgroundQuery,
    ) -> bool {
        !self.has_gui_camera_requests()
            && pending
                .pin_revision
                .is_none_or(|revision| revision == self.highlight.pin_revision())
            && pending.presentation.can_apply(self, query_matches, queries)
    }

    pub(super) fn discard_stale_session_map(
        &mut self,
        map: &mut NavMap,
        query_matches: Option<&QueryMatches>,
        queries: &BackgroundQuery,
    ) {
        let pending = self.take_current_session_map(query_matches, queries);
        if let Some(active) = self.presented_session_map
            && (!active.presentation.is_current(self, query_matches, queries)
                || active.pin.is_some_and(|pin| {
                    pin.revision != self.highlight.pin_revision()
                        && map.pin_rollback_revision(&self.highlight) != Some(pin.revision)
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

    fn take_current_session_map(
        &mut self,
        query_matches: Option<&QueryMatches>,
        queries: &BackgroundQuery,
    ) -> Option<QueuedMapPresentation> {
        let pending = self.pending_session_map.take()?;
        if self.can_apply_pending_map(&pending, query_matches, queries) {
            Some(pending)
        } else {
            self.restore_earlier_camera(pending);
            None
        }
    }

    pub(super) fn apply_session_map(
        &mut self,
        map: &mut NavMap,
        query_matches: Option<&QueryMatches>,
        queries: &BackgroundQuery,
    ) -> Option<PresentationCamera> {
        let pending = self.take_current_session_map(query_matches, queries)?;
        let presentation = pending.presentation;
        if let PresentationTarget::Range { highlight, .. } = presentation.target
            && !gt_map::query_match_has_geometry(
                self.map_presence(query_matches),
                highlight.track,
                &(highlight.start..highlight.end),
            )
        {
            self.restore_earlier_camera(pending);
            return None;
        }
        self.clear_session_map(map);
        let (camera, pin) = match presentation.target {
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
                let owned = !self.highlight.is_sticky(element);
                if owned {
                    let MapPresenceResult::Present(present) =
                        self.map_presence(query_matches).resolve(element)
                    else {
                        return None;
                    };
                    let mut highlight = self.highlight;
                    highlight.pin(present);
                    self.highlight = highlight;
                    self.popup_pos_request = None;
                }
                (
                    camera,
                    Some(PresentedPin {
                        revision: self.highlight.pin_revision(),
                        owned,
                    }),
                )
            }
            PresentationTarget::Range { highlight, .. } => {
                self.highlight.presented_match = Some(highlight);
                (PresentationCamera::TrackRange(highlight), None)
            }
        };
        self.presented_session_map = Some(AppliedMapPresentation { presentation, pin });
        Some(camera)
    }

    pub(super) fn clear_session_map(&mut self, map: &mut NavMap) {
        if let Some(active) = self.presented_session_map.take() {
            match active.presentation.target {
                PresentationTarget::Point { .. } => {
                    let Some(pin) = active.pin.filter(|pin| pin.owned) else {
                        return;
                    };
                    map.discard_pin_rollback(pin.revision);
                    if pin.revision == self.highlight.pin_revision() {
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

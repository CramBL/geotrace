use std::ops::Range;
use std::time::{Duration, Instant};

use egui_notify::Toasts;
use gt_filter::GlobalFilter;
use gt_loaded_files::RecordingData;
use gt_map::{MapFrameInteraction, MapPresentationInput, NavMap, PresentationCamera};
use gt_query_run::MatchExtent;
use gt_session_bridge::{
    ErrorCode, OpenPoint, RangeTarget, Reply, SessionReply, ShowOnMap, TrackReference,
};
use gt_types::{DataCategory, FixRef, Generation, Latitude, Longitude, PointIdx, TrackRef};
use gt_ui_types::{
    DisplayCategory, MapElementRef, MapEligibility, MapHighlight, MapPresence, MapPresenceResult,
    MatchHighlight, MatchRevealTarget, PinRevision, PresentElementRef, QueryMatches,
};
use uuid::Uuid;

use super::mcp_action_notice::McpActionNotice;
use super::pane_presentation::{PanePresentation, PresentationDetail, PresentationQueryMatch};
use super::query_run::BackgroundQuery;
use super::{App, SharedAppState};

#[derive(Clone, Copy)]
pub(super) struct MapPresentation {
    id: Uuid,
    target: PresentationTarget,
    track: TrackReference,
    inputs: MapPresentationInputs,
}

pub(super) struct QueuedMapPresentation {
    presentation: MapPresentation,
    pin_revision: Option<PinRevision>,
    reply: SessionReply,
    expires: Instant,
    earlier_camera: Option<RetainedGuiCamera>,
}

impl QueuedMapPresentation {
    fn wait_ended(&self) -> Option<PresentationRejection> {
        if !self.reply.client_is_waiting() {
            return Some(PresentationRejection::ClientDisconnected);
        }
        (Instant::now() >= self.expires).then_some(PresentationRejection::Error(
            ErrorCode::MapPresentationTimedOut,
        ))
    }
}

#[derive(Clone, Copy)]
enum PresentationRejection {
    ClientDisconnected,
    Error(ErrorCode),
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

pub(super) struct SessionMapDraw {
    pub(super) camera: PresentationCamera,
    reply: SessionReply,
    presentation: MapPresentation,
}

impl SessionMapDraw {
    pub(super) fn confirm(self, toasts: &mut Toasts) {
        let track = self.presentation.track;
        let (receipt, notice) = match self.presentation.target {
            PresentationTarget::Point { fix, .. } => (
                Reply::MapPointOpened(OpenPoint {
                    track,
                    point: fix.point.as_usize(),
                }),
                McpActionNotice::CenterMapOnPoint,
            ),
            PresentationTarget::Range { highlight, .. } => (
                Reply::MapRangeFramed {
                    track,
                    start: highlight.start,
                    end: highlight.end,
                },
                McpActionNotice::FrameMapRange,
            ),
        };
        notice.enqueue(toasts);
        self.reply.send(receipt);
    }
}

#[derive(Clone, Copy)]
pub(super) struct AppliedMapPresentation {
    presentation: MapPresentation,
    pin: Option<PresentedPin>,
}

impl AppliedMapPresentation {
    fn clear(self, map: &mut NavMap, highlight: &mut MapHighlight) {
        match self.presentation.target {
            PresentationTarget::Point { .. } => {
                let Some(pin) = self.pin.filter(|pin| pin.owned) else {
                    return;
                };
                map.discard_pin_rollback(pin.revision);
                if pin.revision == highlight.pin_revision() {
                    highlight.clear_sticky();
                }
            }
            PresentationTarget::Range { .. } => highlight.presented_match = None,
        }
    }

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
        session: Uuid,
        shared: &SharedAppState,
        query_matches: Option<&QueryMatches>,
    ) -> Result<Self, ErrorCode> {
        let track = match target {
            PresentationTarget::Point { fix, .. } => fix.track,
            PresentationTarget::Range { highlight, .. } => highlight.track,
        };
        let reference = shared
            .loaded_files
            .view()
            .session_track_ref(track)
            .ok_or(ErrorCode::StaleTrack)?;
        Ok(Self {
            id: Uuid::new_v4(),
            target,
            track: TrackReference::new(session, reference),
            inputs: MapPresentationInputs::of(shared, query_matches),
        })
    }

    fn track_range(
        shared: &SharedAppState,
        query_matches: Option<&QueryMatches>,
        session: Uuid,
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
        Self::new(
            PresentationTarget::Range {
                highlight: MatchHighlight::new(track, &points),
                query_match,
            },
            session,
            shared,
            query_matches,
        )
    }

    fn require_current(
        self,
        shared: &SharedAppState,
        query_matches: Option<&QueryMatches>,
        queries: &BackgroundQuery,
    ) -> Result<(), ErrorCode> {
        let files = shared.loaded_files.view();
        self.track.resolve(files, self.track.recording.session)?;
        if let PresentationTarget::Range {
            query_match: Some(matched),
            ..
        } = self.target
        {
            queries.phase_for_job(matched.job.session, matched.job)?;
            if !queries.has_current_results(matched.job, files) {
                return Err(ErrorCode::StaleQueryResults);
            }
        }
        if self.inputs != MapPresentationInputs::of(shared, query_matches) {
            return Err(ErrorCode::MapPresentationStale);
        }
        let scope = shared.map_presence(query_matches);
        let visible = match self.target {
            PresentationTarget::Point { fix, .. } => scope.draws(MapElementRef::Fix(fix)),
            PresentationTarget::Range { highlight, .. } => scope
                .eligibility()
                .prepare_track(highlight.track)
                .category_visible(DataCategory::Track),
        };
        visible.then_some(()).ok_or(ErrorCode::NoVisibleMapGeometry)
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

enum PresentationHighlight<'a> {
    Point(PresentElementRef<'a>),
    Range(MatchHighlight),
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
        if self.shutdown_requested() {
            return Err(ErrorCode::ShuttingDown);
        }
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
        MapPresentation::new(
            PresentationTarget::Point {
                fix,
                latitude,
                longitude,
            },
            session,
            &shared,
            query_matches,
        )
    }

    pub(super) fn session_show_on_map(
        &mut self,
        session: Uuid,
        ShowOnMap { target }: ShowOnMap,
    ) -> Result<MapPresentation, ErrorCode> {
        if self.shutdown_requested() {
            return Err(ErrorCode::ShuttingDown);
        }
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
        MapPresentation::track_range(&shared, query_matches, session, track, points, query_match)
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
        if self.has_gui_camera_requests()
            && let Some(pending) = self.take_waiting_session_map()
        {
            self.reject_session_map(
                pending,
                PresentationRejection::Error(ErrorCode::MapPresentationSuperseded),
            );
        }
        self.take_gui_camera_requests()
    }

    fn reject_session_map(
        &mut self,
        pending: QueuedMapPresentation,
        rejection: PresentationRejection,
    ) {
        if let Some(earlier) = pending.earlier_camera
            && !self.has_gui_camera_requests()
        {
            self.map_center_request = earlier.requests.center;
            self.zoom_to_visible_request = earlier.requests.zoom_to_visible;
            if earlier.generation == self.loaded_files.data_generation() {
                self.reveal_query_matches_request = earlier.requests.reveal;
            }
        }
        if let PresentationRejection::Error(error) = rejection {
            pending.reply.send(error);
        }
    }

    fn take_waiting_session_map(&mut self) -> Option<QueuedMapPresentation> {
        let pending = self.pending_session_map.take()?;
        if let Some(rejection) = pending.wait_ended() {
            self.reject_session_map(pending, rejection);
            return None;
        }
        Some(pending)
    }

    pub(super) fn queue_session_map(&mut self, presentation: MapPresentation, reply: SessionReply) {
        if !reply.client_is_waiting() {
            return;
        }
        let mut previous = self.take_waiting_session_map();
        let earlier_camera = if self.has_gui_camera_requests() {
            Some(RetainedGuiCamera {
                requests: self.take_gui_camera_requests(),
                generation: self.loaded_files.data_generation(),
            })
        } else {
            previous
                .as_mut()
                .and_then(|pending| pending.earlier_camera.take())
        };
        let pin_revision = matches!(presentation.target, PresentationTarget::Point { .. })
            .then(|| self.highlight.pin_revision());
        self.pending_session_map = Some(QueuedMapPresentation {
            presentation,
            pin_revision,
            reply,
            expires: Instant::now() + MAP_PRESENTATION_WAIT,
            earlier_camera,
        });
        if let Some(previous) = previous {
            previous.reply.send(ErrorCode::MapPresentationSuperseded);
        }
    }

    pub(super) fn poll_session_map(&mut self, ctx: &egui::Context) {
        let was_waiting = self.pending_session_map.is_some();
        self.pending_session_map = self.take_waiting_session_map();
        if let Some(pending) = &self.pending_session_map {
            ctx.request_repaint_after(pending.expires.saturating_duration_since(Instant::now()));
        } else if was_waiting && self.has_gui_camera_requests() {
            ctx.request_repaint();
        }
    }

    pub(super) fn cancel_pending_session_map(&mut self) {
        if let Some(pending) = self.pending_session_map.take() {
            self.reject_session_map(pending, PresentationRejection::ClientDisconnected);
        }
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
        if let Some(mut pending) = self.pending_session_map.take() {
            let navigated = matches!(interaction, MapFrameInteraction::NavigateCamera);
            if navigated {
                pending.earlier_camera = None;
            }
            let point_superseded = pending.pin_revision.is_some_and(|revision| {
                matches!(interaction, MapFrameInteraction::DismissPoint)
                    || revision != self.highlight.pin_revision()
            });
            let rejection = pending.wait_ended().or_else(|| {
                (navigated || point_superseded).then_some(PresentationRejection::Error(
                    ErrorCode::MapPresentationSuperseded,
                ))
            });
            if let Some(rejection) = rejection {
                self.reject_session_map(pending, rejection);
            } else {
                self.pending_session_map = Some(pending);
            }
        }
        if matches!(input, MapPresentationInput::FrameInteraction) || self.has_gui_camera_requests()
        {
            ctx.request_repaint();
        }
    }

    fn require_pending_map_prerequisites(
        &self,
        pending: &QueuedMapPresentation,
        query_matches: Option<&QueryMatches>,
        queries: &BackgroundQuery,
    ) -> Result<(), ErrorCode> {
        if self.has_gui_camera_requests()
            || pending
                .pin_revision
                .is_some_and(|revision| revision != self.highlight.pin_revision())
        {
            return Err(ErrorCode::MapPresentationSuperseded);
        }
        pending
            .presentation
            .require_current(self, query_matches, queries)?;
        if matches!(
            pending.presentation.target,
            PresentationTarget::Range { .. }
        ) && !self
            .display_mask
            .is_visible(DisplayCategory::QueryHighlights)
        {
            return Err(ErrorCode::QueryHighlightsDisabled);
        }
        Ok(())
    }

    pub(super) fn discard_stale_session_map(
        &mut self,
        map: &mut NavMap,
        query_matches: Option<&QueryMatches>,
        queries: &BackgroundQuery,
    ) {
        let pending = self.take_current_session_map(query_matches, queries);
        if let Some(active) = self.presented_session_map
            && (active
                .presentation
                .require_current(self, query_matches, queries)
                .is_err()
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
        let pending = self.take_waiting_session_map()?;
        match self.require_pending_map_prerequisites(&pending, query_matches, queries) {
            Ok(()) => Some(pending),
            Err(error) => {
                self.reject_session_map(pending, PresentationRejection::Error(error));
                None
            }
        }
    }

    pub(super) fn apply_session_map(
        &mut self,
        map: &mut NavMap,
        query_matches: Option<&QueryMatches>,
        queries: &BackgroundQuery,
    ) -> Option<SessionMapDraw> {
        let pending = self.take_current_session_map(query_matches, queries)?;
        let presentation = pending.presentation;
        let scope = self.map_presence(query_matches);
        let (camera, highlight) = match presentation.target {
            PresentationTarget::Point {
                fix,
                latitude,
                longitude,
            } => {
                let element = MapElementRef::Fix(fix);
                let MapPresenceResult::Present(present) = scope.resolve(element) else {
                    self.reject_session_map(
                        pending,
                        PresentationRejection::Error(ErrorCode::NoVisibleMapGeometry),
                    );
                    return None;
                };
                (
                    PresentationCamera::Center {
                        latitude,
                        longitude,
                    },
                    PresentationHighlight::Point(present),
                )
            }
            PresentationTarget::Range { highlight, .. } => {
                let Some(bounds) = gt_map::query_match_bounds(
                    scope,
                    highlight.track,
                    &(highlight.start..highlight.end),
                ) else {
                    self.reject_session_map(
                        pending,
                        PresentationRejection::Error(ErrorCode::NoVisibleMapGeometry),
                    );
                    return None;
                };
                (
                    PresentationCamera::TrackRange(bounds),
                    PresentationHighlight::Range(highlight),
                )
            }
        };
        if let Some(rejection) = pending.wait_ended() {
            self.reject_session_map(pending, rejection);
            return None;
        }
        let mut next_highlight = self.highlight;
        if let Some(previous) = self.presented_session_map {
            previous.clear(map, &mut next_highlight);
        }
        let pin = match highlight {
            PresentationHighlight::Point(present) => {
                let owned = !next_highlight.is_sticky(present.element_ref());
                if owned {
                    next_highlight.pin(present);
                    self.popup_pos_request = None;
                }
                Some(PresentedPin {
                    revision: next_highlight.pin_revision(),
                    owned,
                })
            }
            PresentationHighlight::Range(highlight) => {
                next_highlight.presented_match = Some(highlight);
                None
            }
        };
        self.highlight = next_highlight;
        self.presented_session_map = Some(AppliedMapPresentation { presentation, pin });
        Some(SessionMapDraw {
            camera,
            reply: pending.reply,
            presentation,
        })
    }

    pub(super) fn clear_session_map(&mut self, map: &mut NavMap) {
        if let Some(active) = self.presented_session_map.take() {
            active.clear(map, &mut self.highlight);
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

const MAP_PRESENTATION_WAIT: Duration = Duration::from_millis(500);

#[cfg(test)]
mod tests;

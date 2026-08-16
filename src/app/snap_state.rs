use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use gt_query_run::SnapErrorValues;
use gt_side_panel::{SnapCostingTarget, SnapRowView};
use gt_snap::wire::Costing;
use gt_types::{FileIdx, LoadedTrack, TrackIdx, TrackRef};

use super::modals::{SnapScope, SnapScopeCount, SnapScopeCounts};
use super::{App, snap, snap_persist};

/// A "Snap again as" choice the track already has a run for, waiting on
/// the confirmation dialog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct SnapReplacePrompt {
    pub(super) track_ref: TrackRef,
    pub(super) choice: gt_ui_types::SnapCosting,
}

/// A snap trigger waiting on the upload-consent dialog. Nothing it asks
/// for is applied before consent: the costing overrides and the cached
/// runs it replaces stay untouched until the runs actually go out.
#[derive(Debug, Default)]
pub(super) struct PendingSnapRequest {
    pub(super) track_refs: Vec<TrackRef>,
    /// Set when the trigger was a "Snap again as" choice, whose runs take
    /// the costing override and replace what the tracks already have.
    costing_choice: Option<gt_ui_types::SnapCosting>,
}

/// A recording-level "Snap again as" choice waiting for the scope dialog
/// to say which of the recording's tracks it covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct SnapScopePrompt {
    pub(super) fi: FileIdx,
    pub(super) choice: gt_ui_types::SnapCosting,
}

/// Snap error data derived from one run for one track: the plot series and
/// the dense per-point values the query providers read. Built once per run
/// (see [`App::with_snap_error_derived`]).
pub(super) struct SnapErrorDerived {
    /// Identity of the source run, for invalidation.
    run: gt_ui_types::ArcIdentity,
    /// The plot series, one entry per sent point.
    series: Arc<Vec<gt_ui_types::SnapErrorPoint>>,
    /// One slot per track point; `Some` exactly for sent points that came
    /// back snapped or interpolated - the fixed `snap_error` semantics
    /// (see docs/snap/design.md, "Query integration").
    values: Arc<Vec<Option<f64>>>,
}

impl SnapErrorDerived {
    fn build(run: gt_ui_types::ArcIdentity, source: &snap::SnapRun, track: &LoadedTrack) -> Self {
        let mut values = vec![None; track.points.len()];
        let series = source
            .result
            .points
            .iter()
            .filter_map(|p| {
                let nav = p.point.get(&track.points)?;
                if matches!(
                    p.kind,
                    gt_snap::wire::SnapPointKind::Snapped
                        | gt_snap::wire::SnapPointKind::Interpolated
                ) && let (Some(error), Some(slot)) =
                    (p.error_m, values.get_mut(p.point.as_usize()))
                {
                    *slot = Some(error);
                }
                Some(gt_ui_types::SnapErrorPoint {
                    x_secs: nav.tpv.time().as_secs_f64(),
                    error_m: p.error_m,
                    kind: Self::kind(p.kind),
                    follows_gap: p.follows_gap,
                })
            })
            .collect();
        Self {
            run,
            series: Arc::new(series),
            values: Arc::new(values),
        }
    }

    /// Map gt-snap's wire-format point kind onto the plot's plain mirror
    /// (a mirror so `gt-ui-types` stays free of the gt-snap dependency;
    /// this function is the one place both types are visible).
    fn kind(kind: gt_snap::wire::SnapPointKind) -> gt_ui_types::SnapErrorKind {
        match kind {
            gt_snap::wire::SnapPointKind::Snapped => gt_ui_types::SnapErrorKind::Snapped,
            gt_snap::wire::SnapPointKind::Interpolated => gt_ui_types::SnapErrorKind::Interpolated,
            gt_snap::wire::SnapPointKind::Unsnapped => gt_ui_types::SnapErrorKind::Unsnapped,
        }
    }
}

impl App {
    /// The costing a track would snap with now: the session override beats
    /// the declared travel mode, which beats the configured default. An
    /// override makes even a road-less declared mode snappable - overriding
    /// wrong declarations is what it exists for.
    pub(super) fn effective_costing(
        &self,
        file: &gt_types::LoadedFile,
        track: &gt_types::LoadedTrack,
    ) -> Option<Costing> {
        if let Some(&costing) = self
            .snap_costing_overrides
            .get(&snap::TrackContentKey::new(track))
        {
            return Some(costing);
        }
        snap::resolve_costing(
            file.metadata.travel_mode.as_ref(),
            self.snap_settings.costing,
        )
    }

    /// The wire costing for a panel-side mirror choice. Exhaustive both
    /// ways, so a costing added to either side fails to compile here.
    pub(super) fn costing_from_choice(choice: gt_ui_types::SnapCosting) -> Costing {
        match choice {
            gt_ui_types::SnapCosting::Auto => Costing::Auto,
            gt_ui_types::SnapCosting::Bicycle => Costing::Bicycle,
            gt_ui_types::SnapCosting::Pedestrian => Costing::Pedestrian,
        }
    }

    /// The re-run submenu's choices, labeled from the wire type's canonical
    /// spelling (the single source, [`Costing::display_name`]).
    pub(super) fn costing_choices() -> Vec<(gt_ui_types::SnapCosting, String)> {
        use strum::IntoEnumIterator;
        gt_ui_types::SnapCosting::iter()
            .map(|choice| {
                let label = Self::costing_from_choice(choice).display_name().to_owned();
                (choice, label)
            })
            .collect()
    }

    /// Act on a "Snap again as" choice. A track choice runs right away
    /// unless that track already has a run for the costing, which raises
    /// the replace dialog; a recording choice raises the scope dialog.
    pub(super) fn handle_snap_costing_request(
        &mut self,
        target: SnapCostingTarget,
        choice: gt_ui_types::SnapCosting,
    ) {
        match target {
            SnapCostingTarget::Track(track_ref) => {
                let params = self.snap_settings.params(Self::costing_from_choice(choice));
                let cached = {
                    let shared = self.shared.borrow();
                    track_ref
                        .resolve(shared.loaded_files.files())
                        .is_some_and(|track| self.snap.has_cached_run(track, params))
                };
                if cached {
                    self.snap_replace_prompt = Some(SnapReplacePrompt { track_ref, choice });
                } else {
                    self.snap_tracks_as(vec![track_ref], choice);
                }
            }
            SnapCostingTarget::Recording(fi) => {
                self.snap_scope_prompt = Some(SnapScopePrompt { fi, choice });
            }
        }
    }

    /// Snap these tracks under an explicitly chosen costing.
    pub(super) fn snap_tracks_as(
        &mut self,
        track_refs: Vec<TrackRef>,
        choice: gt_ui_types::SnapCosting,
    ) {
        self.run_snap_request(PendingSnapRequest {
            track_refs,
            costing_choice: Some(choice),
        });
    }

    /// The recording's tracks covered by one scope of the scope dialog.
    pub(super) fn scoped_track_refs(&self, fi: FileIdx, scope: SnapScope) -> Vec<TrackRef> {
        let shared = self.shared.borrow();
        let Some(file) = fi.get(shared.loaded_files.files()) else {
            return Vec::new();
        };
        (0..file.tracks.len())
            .map(|ti| TrackRef::new(fi, TrackIdx::new(ti)))
            .filter(|track_ref| match scope {
                SnapScope::AllTracks => true,
                SnapScope::SelectedTracks => shared
                    .tree
                    .selection
                    .contains(&gt_side_panel::NodeKey::Track(*track_ref)),
            })
            .collect()
    }

    /// What the scope dialog reports for a recording: the size of each
    /// scope and how many of its tracks already have a run for the chosen
    /// costing.
    pub(super) fn snap_scope_counts(&self, prompt: SnapScopePrompt) -> SnapScopeCounts {
        let params = self
            .snap_settings
            .params(Self::costing_from_choice(prompt.choice));
        let count = |scope| {
            let track_refs = self.scoped_track_refs(prompt.fi, scope);
            let shared = self.shared.borrow();
            let already_snapped = track_refs
                .iter()
                .filter_map(|track_ref| track_ref.resolve(shared.loaded_files.files()))
                .filter(|track| self.snap.has_cached_run(track, params))
                .count();
            SnapScopeCount {
                tracks: track_refs.len(),
                already_snapped,
            }
        };
        SnapScopeCounts {
            selected: count(SnapScope::SelectedTracks),
            all: count(SnapScope::AllTracks),
        }
    }

    /// The side panel's per-track snap view: scheduler activity and cached
    /// runs resolved against each file's declared travel mode and the
    /// configured costing. Tracks in the default idle state get no entry.
    pub(super) fn snap_row_views(&self) -> HashMap<TrackRef, SnapRowView> {
        let shared = self.shared.borrow();
        let mut rows = HashMap::new();
        for (fi, file) in shared.loaded_files.files().iter().enumerate() {
            let declared = file.metadata.travel_mode.as_ref();
            for (ti, track) in file.tracks.iter().enumerate() {
                let track_ref = TrackRef::new(FileIdx::new(fi), TrackIdx::new(ti));
                let row = match self.effective_costing(file, track) {
                    None => {
                        // Without an override, `effective_costing` returns
                        // `None` only for a declared road-less mode, so
                        // `declared` is present here; skip (= idle)
                        // defensively rather than unwrap.
                        let Some(mode) = declared else { continue };
                        SnapRowView::Unsnappable {
                            travel_mode: mode.display_name().to_owned(),
                        }
                    }
                    Some(costing) => match self.snap.activity_for(track_ref) {
                        Some(snap::SnapActivity::Queued) => SnapRowView::Queued,
                        Some(snap::SnapActivity::InFlight {
                            completed_chunks,
                            total_chunks,
                        }) => SnapRowView::InFlight {
                            completed_chunks: *completed_chunks,
                            total_chunks: *total_chunks,
                        },
                        Some(snap::SnapActivity::Failed { error }) => SnapRowView::Failed {
                            error: error.clone(),
                        },
                        Some(snap::SnapActivity::NothingToSend) => SnapRowView::NothingToSend,
                        None => match self.snap.latest_run_for(track) {
                            Some(run) => {
                                let reasons = snap::stale_reasons(
                                    &run,
                                    self.snap_settings.params(costing),
                                    self.snap.current_host().as_deref(),
                                );
                                SnapRowView::Done {
                                    snapped: run.result.kind_counts.snapped,
                                    interpolated: run.result.kind_counts.interpolated,
                                    unsnapped: run.result.kind_counts.unsnapped,
                                    confidence_score: run.result.confidence_score,
                                    shown: !self.hidden_snapped.contains(&track_ref),
                                    stale: (!reasons.is_empty()).then_some(reasons),
                                    partial: run.result.partial,
                                    warnings: run.warnings.iter().map(snap::warning_line).collect(),
                                }
                            }
                            None => continue,
                        },
                    },
                };
                rows.insert(track_ref, row);
            }
        }
        rows
    }

    /// The map's snapped-track geometry: one entry per tree-visible track
    /// whose completed run is not toggled hidden. The per-run projection is
    /// shared via `Arc`, so assembling this each frame is cheap.
    pub(super) fn snapped_tracks_view(&self) -> gt_ui_types::SnappedTracks {
        let shared = self.shared.borrow();
        let visibility = shared.tree.visibility();
        let mut snapped = gt_ui_types::SnappedTracks::default();
        for (fi, file) in shared.loaded_files.files().iter().enumerate() {
            let fi = FileIdx::new(fi);
            for (ti, track) in file.tracks.iter().enumerate() {
                let track_ref = TrackRef::new(fi, TrackIdx::new(ti));
                if self.hidden_snapped.contains(&track_ref) {
                    continue;
                }
                if !visibility.track_shown(track_ref) {
                    continue;
                }
                if let Some(run) = self.snap.latest_run_for(track) {
                    snapped
                        .by_track
                        .insert(track_ref, Arc::clone(&run.geometry));
                }
            }
        }
        snapped
    }

    /// The plot's snap error series: per sent point of each completed run,
    /// the point's plot time, its snap error, and its match kind. Unlike the
    /// map geometry this is not gated on visibility or the snapped-track
    /// toggle - the plot filters by its own track visibility, and hiding the
    /// snapped geometry on the map does not retract the error data.
    pub(super) fn snap_error_view(&mut self) -> gt_ui_types::SnapErrorSeries {
        let mut series = gt_ui_types::SnapErrorSeries::default();
        self.with_snap_error_derived(&mut series, |track_ref, derived, series| {
            series
                .points_by_track
                .insert(track_ref, Arc::clone(&derived.series));
        });
        series
    }

    /// Per-track dense snap error values for the query providers, one entry
    /// per track with a completed run.
    pub(super) fn snap_error_values(&mut self) -> SnapErrorValues {
        let mut values = SnapErrorValues::new();
        self.with_snap_error_derived(&mut values, |track_ref, derived, values| {
            values.insert(track_ref, Arc::clone(&derived.values));
        });
        values
    }

    /// Walk every track with a completed run, handing `f` its cached
    /// derived data. The data is built once per run and reused by `Arc`
    /// identity: downstream (the plot's mipmap cache, the query
    /// fingerprint) keys off the pointers, so a fresh allocation per frame
    /// would defeat every cache below this point. Entries for unloaded
    /// tracks are pruned.
    fn with_snap_error_derived<T>(
        &mut self,
        out: &mut T,
        mut f: impl FnMut(TrackRef, &SnapErrorDerived, &mut T),
    ) {
        let shared = self.shared.borrow();
        let mut seen: HashSet<snap::TrackContentKey> = HashSet::new();
        for (fi, file) in shared.loaded_files.files().iter().enumerate() {
            for (ti, track) in file.tracks.iter().enumerate() {
                let track_ref = TrackRef::new(FileIdx::new(fi), TrackIdx::new(ti));
                let Some(run) = self.snap.latest_run_for(track) else {
                    continue;
                };
                let content = snap::TrackContentKey::new(track);
                seen.insert(content);
                let run_id = gt_ui_types::ArcIdentity::of(&run);
                if self
                    .snap_error_cache
                    .get(&content)
                    .is_none_or(|derived| derived.run != run_id)
                {
                    self.snap_error_cache
                        .insert(content, SnapErrorDerived::build(run_id, &run, track));
                }
                // Present by construction: inserted just above when absent.
                let Some(derived) = self.snap_error_cache.get(&content) else {
                    continue;
                };
                f(track_ref, derived, out);
            }
        }
        self.snap_error_cache
            .retain(|content, _| seen.contains(content));
    }

    /// Act on a snap trigger from the side panel.
    pub(super) fn handle_snap_request(&mut self, track_refs: Vec<TrackRef>) {
        self.run_snap_request(PendingSnapRequest {
            track_refs,
            costing_choice: None,
        });
    }

    /// Run a snap trigger: park it on the consent dialog while consent is
    /// pending, otherwise apply its costing choice and queue the runs. The
    /// scheduler sends them one at a time, whatever the batch size.
    pub(super) fn run_snap_request(&mut self, request: PendingSnapRequest) {
        if request.track_refs.is_empty() {
            return;
        }
        if !self.snap_settings.consent_granted() {
            self.pending_snap = request;
            self.snap_consent_prompt = true;
            return;
        }
        if let Some(choice) = request.costing_choice {
            self.apply_costing_choice(&request.track_refs, choice);
        }
        for track_ref in request.track_refs {
            self.queue_snap(track_ref);
        }
    }

    /// Give each track the session costing override and forget its cached
    /// run for that costing, so the request reaches the server and its
    /// result replaces what the track had. The fresh runs are not stale:
    /// the override feeds the effective parameters.
    fn apply_costing_choice(&mut self, track_refs: &[TrackRef], choice: gt_ui_types::SnapCosting) {
        let costing = Self::costing_from_choice(choice);
        let params = self.snap_settings.params(costing);
        let shared = self.shared.borrow();
        for track in track_refs
            .iter()
            .filter_map(|track_ref| track_ref.resolve(shared.loaded_files.files()))
        {
            self.snap_costing_overrides
                .insert(snap::TrackContentKey::new(track), costing);
            self.snap.discard_cached_run(track, params);
        }
    }

    /// Whether any loaded track can snap (no road-less declared mode).
    /// Gates the consent and auto-choice prompts: neither shows on an
    /// empty session.
    pub(super) fn any_snappable_track(&self) -> bool {
        let shared = self.shared.borrow();
        shared.loaded_files.files().iter().any(|file| {
            !file.tracks.is_empty()
                && snap::resolve_costing(
                    file.metadata.travel_mode.as_ref(),
                    self.snap_settings.costing,
                )
                .is_some()
        })
    }

    /// Enqueue an automatic run for every snappable track without a
    /// displayed run. Hidden tracks park in the queue until shown; tracks
    /// with transient activity (queued, in flight, failed) are left alone
    /// by the scheduler, and stale runs are re-run manually only.
    pub(super) fn queue_auto_snaps(&mut self) {
        // Offline pauses auto mode entirely (the scheduler would refuse
        // each request anyway; skipping documents the pause and saves the
        // per-track planning work).
        if self.offline || !self.snap_settings.auto_snap_active() {
            return;
        }
        let shared = self.shared.borrow();
        for (fi, file) in shared.loaded_files.files().iter().enumerate() {
            for (ti, track) in file.tracks.iter().enumerate() {
                let track_ref = TrackRef::new(FileIdx::new(fi), TrackIdx::new(ti));
                let Some(costing) = self.effective_costing(file, track) else {
                    continue;
                };
                if self.snap.latest_run_for(track).is_some() {
                    continue;
                }
                self.snap.request_snap(
                    track_ref,
                    track,
                    self.snap_settings.params(costing),
                    snap::SnapPriority::Auto,
                );
            }
        }
    }

    /// Queue a snap run for a track under its effective costing (session
    /// override, else declared travel mode, else the configured default).
    fn queue_snap(&mut self, track_ref: TrackRef) {
        let shared = self.shared.borrow();
        let files = shared.loaded_files.files();
        let Some(file) = track_ref.fi.get(files) else {
            return;
        };
        let Some(track) = track_ref.resolve(files) else {
            return;
        };
        let Some(costing) = self.effective_costing(file, track) else {
            return;
        };
        self.snap.request_snap(
            track_ref,
            track,
            self.snap_settings.params(costing),
            snap::SnapPriority::Manual,
        );
    }

    /// Seed a recording's stored snap runs into the session stores. Runs
    /// are matched to tracks by content fingerprint, so index shifts or a
    /// re-segmentation since storage simply leave non-matching entries
    /// unrestored. Each run restores once, to its first matching track
    /// among the files loaded from this recording; the content-keyed
    /// stores serve every duplicate of that track from the same entry.
    pub(super) fn restore_snap_runs(&mut self, db_ref: &gt_store::DatabaseRef, blob: &[u8]) {
        let Some(stored) = snap_persist::decode(blob) else {
            return;
        };
        let shared = self.shared.borrow();
        let view = shared.loaded_files.view();
        for run in stored {
            let target = view.entries().enumerate().find_map(|(fi, entry)| {
                if entry.history().db_ref() != Some(db_ref) {
                    return None;
                }
                entry
                    .file()
                    .tracks
                    .iter()
                    .position(|track| run.matches(track))
                    .map(|ti| TrackRef::new(FileIdx::new(fi), TrackIdx::new(ti)))
            });
            let Some(track_ref) = target else {
                continue;
            };
            let Some(track) = track_ref.resolve(view.files()) else {
                continue;
            };
            self.snap.restore_run(track_ref, track, run.into_run());
        }
    }

    /// Persist the latest snap runs of every history-stored file that owns
    /// one of the just-completed tracks. The whole file's runs are written
    /// each time (the blob holds all of them), so the stored copy always
    /// mirrors the session's latest state.
    pub(super) fn persist_snap_runs(&self, completed: &[snap::TrackContentKey]) {
        let shared = self.shared.borrow();
        for entry in shared.loaded_files.view().entries() {
            let file = entry.file();
            let affected = file
                .tracks
                .iter()
                .any(|track| completed.contains(&snap::TrackContentKey::new(track)));
            if !affected {
                continue;
            }
            let Some(db_ref) = entry.history().db_ref().cloned() else {
                continue;
            };
            let runs: Vec<(&gt_types::LoadedTrack, std::sync::Arc<snap::SnapRun>)> = file
                .tracks
                .iter()
                .filter_map(|track| self.snap.latest_run_for(track).map(|run| (track, run)))
                .collect();
            let blob = snap_persist::encode(runs.iter().map(|(track, run)| (*track, run.as_ref())));
            self.history.store_snap_runs(db_ref, blob);
        }
    }
}

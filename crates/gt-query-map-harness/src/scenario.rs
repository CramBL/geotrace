use std::ops::Range;

use chrono::Duration;
use gt_filter::GlobalFilter;
use gt_map::display_counts::{DisplayCounts, SuppliedCounts};
use gt_map::scope;
use gt_query_run::{QuerySession, RunInputs, RunResults, schema_from_files};
use gt_types::{DataCategory, PointIdx, TrackRef};
use gt_ui_types::{
    DataPointRef, DisplayCategory, DisplayMask, EventMarkerVisibility, GeneratedMarkerVisibility,
    MapHighlight, MatchHighlight, QueryMatches, TrackDataVisibility,
};

use crate::classify::PointClass;
use crate::dataset::{Dataset, epoch};
use crate::panel::{PanelView, RunAttempt};
use crate::picture::{MapPicture, TrackPicture};

/// A query window and a map, driven step by step with no egui, no threads, and
/// no GPU.
///
/// Steps mutate the state a frame of the app would: the editor text, the run,
/// the global filter, the tree, the selection. Every step then refreshes result
/// staleness the way the app does once per frame, so the state after a step is
/// what the app would show.
///
/// Observe the result through [`MapScenario::picture`] (what the map draws) or
/// [`MapScenario::panel`] (what the query window says), both snapshot-friendly.
pub struct MapScenario {
    dataset: Dataset,
    visibility: TrackDataVisibility,
    filter: GlobalFilter,
    /// Left at the default, and deliberately without a step to change it:
    /// [`DisplayCounts`] is defined pre-display-mask, and
    /// [`MapScenario::picture`] asserts the per-point classification against
    /// those counts. Adding a step here means teaching that check about the
    /// mask.
    display_mask: DisplayMask,
    highlight: MapHighlight,
    session: QuerySession,
    last_run: Option<RunAttempt>,
}

impl MapScenario {
    /// A scenario over `dataset`, everything visible and nothing run yet.
    pub fn new(dataset: Dataset) -> Self {
        let visibility = TrackDataVisibility::from_loaded(dataset.files().files());
        Self {
            dataset,
            visibility,
            filter: GlobalFilter::default(),
            display_mask: DisplayMask::default(),
            highlight: MapHighlight::default(),
            session: QuerySession::new(),
            last_run: None,
        }
    }

    /// Put `text` in the editor without running it.
    pub fn set_text(&mut self, text: &str) -> &mut Self {
        self.session.set_text(text.to_owned());
        self.sync()
    }

    /// Put `text` in the editor and run it to completion.
    pub fn run(&mut self, text: &str) -> &mut Self {
        self.set_text(text).run_current()
    }

    /// Run whatever is in the editor to completion, or record that the session
    /// refused (a failing or empty editor, or a mixed channel source).
    pub fn run_current(&mut self) -> &mut Self {
        let Self {
            dataset,
            visibility,
            filter,
            session,
            ..
        } = self;
        session.sync_checks(&schema_from_files(dataset.files().files()));
        let inputs = RunInputs {
            loaded_files: dataset.files().view(),
            visibility,
            filter,
            snap_errors: dataset.snap_errors(),
            jamming: dataset.jamming(),
        };
        self.last_run = match session.start_run(inputs) {
            Some(prepared) => {
                session.finish_run(prepared.execute());
                Some(RunAttempt::Completed)
            }
            None => Some(RunAttempt::Refused),
        };
        // A new run relists the results, so the row the pointer was on is gone.
        self.highlight.hover_match = None;
        self.sync()
    }

    /// Drop the last run's results, as the toolbar's clear action does. The
    /// hovered match goes with them - the app sets it per frame from the row
    /// under the pointer, and that row no longer exists.
    pub fn clear(&mut self) -> &mut Self {
        self.session.clear_results();
        self.last_run = None;
        self.highlight.hover_match = None;
        self.sync()
    }

    /// Narrow the global time filter to `start..=end`, in seconds from the
    /// dataset epoch. `None` leaves that end open.
    pub fn set_time_filter_secs(&mut self, start: Option<i64>, end: Option<i64>) -> &mut Self {
        self.filter.time_start = start.map(|secs| epoch() + Duration::seconds(secs));
        self.filter.time_end = end.map(|secs| epoch() + Duration::seconds(secs));
        self.sync()
    }

    /// Replace the whole global filter, for the conditions with no shorthand.
    pub fn set_filter(&mut self, filter: GlobalFilter) -> &mut Self {
        self.filter = filter;
        self.sync()
    }

    /// Enable or disable a track in the tree.
    pub fn set_track_visible(&mut self, track: TrackRef, visible: bool) -> &mut Self {
        if let Some(file) = self.visibility.files.get_mut(track.fi.as_usize())
            && let Some(entry) = file.tracks.get_mut(track.index.as_usize())
        {
            entry.enabled = visible;
        }
        self.sync()
    }

    /// Click a point, pinning its popup - or unpinning it when it was already
    /// the selected one, exactly as a click in the map or a results table does.
    pub fn select_point(&mut self, track: TrackRef, point_index: usize) -> &mut Self {
        self.highlight.toggle_sticky(point_ref(track, point_index));
        self
    }

    /// Hover the `match_index`-th match of the `query_index`-th query in the
    /// results panel, cross-highlighting it on the map.
    pub fn hover_match(&mut self, query_index: usize, match_index: usize) -> &mut Self {
        let matches = self.panel_matches(query_index);
        let hovered = matches.get(match_index).cloned();
        assert!(
            hovered.is_some(),
            "query {query_index} has {} matches, so {match_index} cannot be hovered",
            matches.len()
        );
        self.highlight.hover_match =
            hovered.map(|(track, range)| MatchHighlight::new(track, &range));
        self
    }

    /// Stop hovering the results table, as leaving the row does.
    pub fn clear_hover_match(&mut self) -> &mut Self {
        self.highlight.hover_match = None;
        self
    }

    /// Recompare the results against the current inputs. Every step already
    /// does this, mirroring the app's per-frame refresh; call it to say so.
    pub fn refresh_staleness(&mut self) -> &mut Self {
        self.sync()
    }

    /// The matches of one query as the results panel lists them: per track, in
    /// track then range order, with absolute point indices.
    pub fn panel_matches(&self, query_index: usize) -> Vec<(TrackRef, Range<usize>)> {
        let Some(RunResults::Points(points)) = self.session.results() else {
            return Vec::new();
        };
        points
            .queries
            .get(query_index)
            .map(|query| {
                query
                    .matches
                    .iter()
                    .flat_map(|tm| tm.ranges.iter().map(|range| (tm.track, range.clone())))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// How the map reads one point right now.
    pub fn classify(&self, track: TrackRef, point_index: usize) -> PointClass {
        let matches = self.session.matches();
        PointClass {
            visibility: scope::point_visibility(
                self.dataset.files().files(),
                &self.visibility,
                &self.filter,
                self.display_mask,
                matches,
                point_ref(track, point_index),
            ),
            draw_layers: matches
                .map(|m| m.draw_mask(track, point_index))
                .unwrap_or_default(),
            hover_matched: self
                .highlight
                .hover_match
                .is_some_and(|hovered| hovered.track == track && hovered.contains(point_index)),
            selected: self.highlight.sticky == Some(point_ref(track, point_index)),
        }
    }

    /// The element counts the map itself derives, for cross-checking the
    /// per-point classification against real map code.
    pub fn counts(&self) -> DisplayCounts {
        DisplayCounts::compute(
            self.dataset.files().files(),
            &self.visibility,
            &self.filter,
            &EventMarkerVisibility::default(),
            &GeneratedMarkerVisibility::default(),
            self.session.matches(),
            SuppliedCounts::default(),
        )
    }

    /// What the map shows, as a picture. Panics when the per-point
    /// classification and the map's own counts disagree - that divergence is
    /// what the harness exists to catch, so every assertion checks it.
    pub fn picture(&self) -> MapPicture {
        let counts = self.counts();
        let tracks: Vec<TrackPicture> = self
            .dataset
            .track_refs()
            .into_iter()
            .map(|track| TrackPicture {
                track,
                label: self.dataset.label(track),
                points: (0..self.point_count(track))
                    .map(|pi| self.classify(track, pi))
                    .collect(),
            })
            .collect();
        let shown = tracks
            .iter()
            .flat_map(|track| &track.points)
            .filter(|point| point.is_shown())
            .count();
        assert_eq!(
            shown,
            counts.get(DisplayCategory::TrackPoints),
            "the classification and the map's own point count disagree"
        );
        assert_eq!(
            self.drawn_match_count(),
            counts.get(DisplayCategory::QueryHighlights),
            "the classification and the map's own halo count disagree"
        );
        MapPicture {
            tracks,
            stale: self.session.results().is_some_and(RunResults::stale),
            shown_points: counts.get(DisplayCategory::TrackPoints),
            halos: counts.get(DisplayCategory::QueryHighlights),
        }
    }

    /// What the query window says: how the editor text split into queries, how
    /// each checked, the last run, and the per-query summary lines.
    pub fn panel(&self) -> PanelView<'_> {
        PanelView {
            chunks: self.session.chunks(),
            run: self.last_run,
            results: self.session.results(),
        }
    }

    /// The composed map effect of the last run, for an assertion about ranges
    /// rather than points.
    pub fn matches(&self) -> Option<&QueryMatches> {
        self.session.matches()
    }

    pub fn dataset(&self) -> &Dataset {
        &self.dataset
    }

    /// Recompute result staleness from the current inputs, as the app does
    /// every frame the window is open.
    fn sync(&mut self) -> &mut Self {
        let Self {
            dataset,
            visibility,
            filter,
            session,
            ..
        } = self;
        session.refresh_staleness(RunInputs {
            loaded_files: dataset.files().view(),
            visibility,
            filter,
            snap_errors: dataset.snap_errors(),
            jamming: dataset.jamming(),
        });
        self
    }

    fn point_count(&self, track: TrackRef) -> usize {
        track
            .resolve(self.dataset.files().files())
            .map_or(0, |t| t.points.len())
    }

    /// Matches with at least one drawn point, counted the way
    /// [`DisplayCounts`] counts halos.
    ///
    /// Deliberately re-aggregated from the per-point classification rather than
    /// reusing the counts' own loop, so the check in
    /// [`picture`](Self::picture) cannot pass by construction.
    fn drawn_match_count(&self) -> usize {
        let Some(matches) = self.session.matches() else {
            return 0;
        };
        self.dataset
            .track_refs()
            .into_iter()
            .map(|track| {
                matches
                    .draws
                    .iter()
                    .flat_map(|layer| layer.ranges_for(track))
                    .filter(|range| {
                        (range.start..range.end).any(|pi| self.classify(track, pi).is_shown())
                    })
                    .count()
            })
            .sum()
    }
}

fn point_ref(track: TrackRef, point_index: usize) -> DataPointRef {
    DataPointRef {
        track,
        category: DataCategory::Tpv,
        point_index: PointIdx::new(point_index),
    }
}

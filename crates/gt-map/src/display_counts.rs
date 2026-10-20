//! Availability counts before display masking.
//! Count scans run only when requested by the display popup.

use gt_filter::GlobalFilter;
use gt_loaded_files::IndexAlignedFiles;
use gt_types::{
    DataCategory, FileIdx, FixRef, Generation, LoadedFile, PointIdx, SkyGeometry, TrackIdx,
    TrackRef,
};
use gt_ui_types::log_matches::LogMatchScope;
use gt_ui_types::{
    DisplayCategory, EventMarkerVisibility, GeneratedMarkerVisibility, LogMatches, MapElementRef,
    MapEligibility, MapEligibilityResult, QueryMatches, SkyGlyphVariant, SnappedTracks,
    TrackDataVisibility,
};

use crate::MapFiles;

/// Counts the app supplies, not derived from the loaded recordings.
#[derive(Debug, Clone, Copy, Default)]
pub struct SuppliedCounts<'a> {
    /// Snapped geometry, already scoped by the app to the tree-visible
    /// tracks whose run is shown. The filter is applied here.
    pub snapped_tracks: Option<&'a SnappedTracks>,
    /// Interference cells archived for the day the overlay shows.
    pub jamming_cells: usize,
    /// Grid nodes archived for the day the heatmap shows.
    pub tec_nodes: usize,
    /// Hexagons the loaded logs' filters put on the map, already scoped by the
    /// app to the shown logs and enabled filters. The tree and the global
    /// filter are applied by [`DisplayCounts::compute`], not before this field
    /// is filled.
    pub log_matches: Option<&'a LogMatches>,
}

/// The in-scope element count of every [`DisplayCategory`].
///
/// Computing is a full scan over the loaded points and markers - call it
/// when the counts are shown (the display toggle popup is open), not
/// unconditionally per frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DisplayCounts {
    tracks: usize,
    track_points: usize,
    ghost_fixes: usize,
    satellite_labels: usize,
    custom_markers: usize,
    generated_markers: usize,
    event_markers: usize,
    query_highlights: usize,
    snapped_tracks: usize,
    sky_glyphs: usize,
    sky_discs: usize,
    /// Interference cells available for the shown day, from the archive.
    jamming_hexes: usize,
    /// TEC grid nodes available for the shown instant, from the archive.
    tec_heatmap: usize,
    /// Hexagons the loaded logs' filters selected, those on a track the tree
    /// and the global filter keep.
    log_matches: usize,
}

impl DisplayCounts {
    pub fn sky_glyphs(self, variant: SkyGlyphVariant) -> usize {
        match variant {
            SkyGlyphVariant::Ring => self.sky_glyphs,
            SkyGlyphVariant::Disc => self.sky_discs,
        }
    }

    pub fn get(self, category: DisplayCategory) -> usize {
        match category {
            DisplayCategory::Tracks => self.tracks,
            DisplayCategory::TrackPoints => self.track_points,
            DisplayCategory::GhostFixes => self.ghost_fixes,
            DisplayCategory::SatelliteLabels => self.satellite_labels,
            DisplayCategory::CustomMarkers => self.custom_markers,
            DisplayCategory::GeneratedMarkers => self.generated_markers,
            DisplayCategory::EventMarkers => self.event_markers,
            DisplayCategory::QueryHighlights => self.query_highlights,
            DisplayCategory::SnappedTracks => self.snapped_tracks,
            DisplayCategory::SkyGlyphs => self.sky_glyphs,
            DisplayCategory::JammingHexes => self.jamming_hexes,
            DisplayCategory::TecHeatmap => self.tec_heatmap,
            DisplayCategory::LogMatches => self.log_matches,
        }
    }

    /// Build counts from a per-category lookup, for UI tests that need
    /// known numbers without a full fixture file.
    #[cfg(test)]
    pub(crate) fn from_fn(get: impl Fn(DisplayCategory) -> usize) -> Self {
        Self {
            tracks: get(DisplayCategory::Tracks),
            track_points: get(DisplayCategory::TrackPoints),
            ghost_fixes: get(DisplayCategory::GhostFixes),
            satellite_labels: get(DisplayCategory::SatelliteLabels),
            custom_markers: get(DisplayCategory::CustomMarkers),
            generated_markers: get(DisplayCategory::GeneratedMarkers),
            event_markers: get(DisplayCategory::EventMarkers),
            query_highlights: get(DisplayCategory::QueryHighlights),
            snapped_tracks: get(DisplayCategory::SnappedTracks),
            sky_glyphs: get(DisplayCategory::SkyGlyphs),
            sky_discs: get(DisplayCategory::SkyGlyphs),
            log_matches: get(DisplayCategory::LogMatches),
            jamming_hexes: get(DisplayCategory::JammingHexes),
            tec_heatmap: get(DisplayCategory::TecHeatmap),
        }
    }

    pub fn compute(
        files: &[LoadedFile],
        visibility: &TrackDataVisibility,
        filter: &GlobalFilter,
        event_marker_visibility: &EventMarkerVisibility,
        generated_marker_visibility: &GeneratedMarkerVisibility,
        query_matches: Option<&QueryMatches>,
        supplied: SuppliedCounts<'_>,
    ) -> Self {
        let eligibility = MapEligibility::new(
            files,
            visibility,
            filter,
            query_matches,
            generated_marker_visibility,
            event_marker_visibility,
        )
        .prepare();
        let mut counts = Self {
            jamming_hexes: supplied.jamming_cells,
            tec_heatmap: supplied.tec_nodes,
            log_matches: supplied.log_matches.map_or(0, |matches| {
                let scope = LogMatchScope::from_prepared(files, filter, |track| {
                    eligibility.track(track).map(|prepared| prepared.scope())
                });
                matches
                    .layers()
                    .iter()
                    .map(|layer| scope.matches(layer).count())
                    .sum()
            }),
            ..Self::default()
        };
        for (fi, file) in files.iter().enumerate() {
            for ti in 0..file.tracks.len() {
                let track_ref = TrackRef::new(FileIdx::new(fi), TrackIdx::new(ti));
                let Some(prepared) = eligibility.track(track_ref) else {
                    continue;
                };
                if !prepared.is_in_scope() {
                    continue;
                }
                let Some(track) = prepared.track() else {
                    continue;
                };
                let track_vis = prepared;
                if prepared.category_visible(DataCategory::Track)
                    && supplied
                        .snapped_tracks
                        .is_some_and(|snapped| snapped.get(track_ref).is_some())
                {
                    counts.snapped_tracks += 1;
                }
                let point_in_scope = |pi: usize| {
                    prepared.element_eligibility(MapElementRef::Fix(FixRef::new(
                        track_ref,
                        PointIdx::new(pi),
                    ))) == MapEligibilityResult::Eligible
                };

                if track_vis.category_visible(DataCategory::Track) {
                    // Counted per track regardless of query-hide coverage:
                    // "Tracks" counts whether this track's line is eligible
                    // to draw, not whether it currently draws any pixels.
                    counts.tracks += 1;
                }
                if track_vis.category_visible(DataCategory::Tpv) {
                    counts.track_points += track
                        .points
                        .iter()
                        .enumerate()
                        .filter(|(pi, point)| {
                            !point.is_navigation_estimate() && point_in_scope(*pi)
                        })
                        .count();
                    counts.satellite_labels += track
                        .sat_label_anchors
                        .iter()
                        .filter(|a| point_in_scope(a.point.as_usize()))
                        .count();
                    // Sky glyphs anchor on report-bearing points. Like the
                    // labels they are per-point track ink, so they share the
                    // `DataCategory::Tpv` tree gate.
                    counts.sky_discs += track
                        .points
                        .iter()
                        .enumerate()
                        .filter(|(pi, point)| {
                            point.supports_sky_geometry(SkyGeometry::Position)
                                && point_in_scope(*pi)
                        })
                        .count();
                    counts.sky_glyphs += track
                        .points
                        .iter()
                        .enumerate()
                        .filter(|(pi, p)| {
                            p.supports_sky_geometry(SkyGeometry::Azimuth) && point_in_scope(*pi)
                        })
                        .count();
                }
                if track_vis.category_visible(DataCategory::Tpv) {
                    counts.ghost_fixes += track
                        .points
                        .iter()
                        .enumerate()
                        .filter(|(pi, p)| p.is_navigation_estimate() && point_in_scope(*pi))
                        .count();
                }
                counts.custom_markers += prepared.eligible_custom_markers().count();
                counts.generated_markers += prepared.eligible_generated_markers().count();
                counts.event_markers += prepared.eligible_event_markers().count();
                // A match counts while any of its points is in scope - the
                // halo renderer paints over the filtered point path, so a
                // range fully outside the time window stays invisible.
                counts.query_highlights += query_matches.map_or(0, |m| {
                    m.draws
                        .iter()
                        .flat_map(|layer| layer.ranges_for(track_ref))
                        .filter(|&range| {
                            track_vis.category_visible(DataCategory::Track)
                                && range
                                    .clone()
                                    .any(|pi| prepared.fix_is_available(PointIdx::new(pi)))
                        })
                        .count()
                });
            }
        }
        counts
    }
}

#[derive(Clone, PartialEq)]
struct DisplayCountsKey {
    files_generation: Generation<IndexAlignedFiles>,
    snapped_track_refs: Vec<TrackRef>,
    jamming_cells: usize,
    tec_nodes: usize,
    log_matches: Option<LogMatches>,
    filter: GlobalFilter,
    visibility: TrackDataVisibility,
    event_marker_visibility: EventMarkerVisibility,
    generated_marker_visibility: GeneratedMarkerVisibility,
    query_matches: Option<QueryMatches>,
}

/// Memoizes [`DisplayCounts::compute`] across frames. The display-toggle popup
/// requests counts every frame it is open, but the O(all points) walk only
/// needs to rerun when one of its inputs changes.
#[derive(Default)]
pub(crate) struct DisplayCountsCache {
    cached: Option<(DisplayCountsKey, DisplayCounts)>,
    /// Number of times the full walk actually ran, so a test can prove hits
    /// skip it.
    #[cfg(test)]
    computes: usize,
}

impl DisplayCountsCache {
    /// Return the counts for the current inputs, reusing the last result when
    /// nothing `compute` reads has changed. The hit path clones nothing: it
    /// compares the stored key against the borrowed inputs. Only a miss stores
    /// a fresh key.
    #[expect(clippy::too_many_arguments, reason = "mirrors DisplayCounts::compute")]
    pub(crate) fn get(
        &mut self,
        files: MapFiles<'_>,
        visibility: &TrackDataVisibility,
        filter: &GlobalFilter,
        event_marker_visibility: &EventMarkerVisibility,
        generated_marker_visibility: &GeneratedMarkerVisibility,
        query_matches: Option<&QueryMatches>,
        supplied: SuppliedCounts<'_>,
    ) -> DisplayCounts {
        let files_generation = match files {
            MapFiles::Loaded(files) => Some(files.generation()),
            MapFiles::Borrowed(_) => None,
        };
        if let Some((key, counts)) = &self.cached
            && Some(key.files_generation) == files_generation
            && supplied
                .snapped_tracks
                .into_iter()
                .flat_map(|snapped| snapped.iter().map(|(track_ref, _)| track_ref))
                .eq(key.snapped_track_refs.iter().copied())
            && key.jamming_cells == supplied.jamming_cells
            && key.tec_nodes == supplied.tec_nodes
            && key.log_matches.as_ref() == supplied.log_matches
            && key.filter == *filter
            && key.visibility == *visibility
            && key.event_marker_visibility == *event_marker_visibility
            && key.generated_marker_visibility == *generated_marker_visibility
            && key.query_matches.as_ref() == query_matches
        {
            return *counts;
        }
        #[cfg(test)]
        {
            self.computes += 1;
        }
        let counts = DisplayCounts::compute(
            files.files(),
            visibility,
            filter,
            event_marker_visibility,
            generated_marker_visibility,
            query_matches,
            supplied,
        );
        self.cached = files_generation.map(|files_generation| {
            (
                DisplayCountsKey {
                    jamming_cells: supplied.jamming_cells,
                    tec_nodes: supplied.tec_nodes,
                    log_matches: supplied.log_matches.cloned(),
                    files_generation,
                    snapped_track_refs: supplied
                        .snapped_tracks
                        .into_iter()
                        .flat_map(|snapped| snapped.iter().map(|(track_ref, _)| track_ref))
                        .collect(),
                    filter: *filter,
                    visibility: visibility.clone(),
                    event_marker_visibility: event_marker_visibility.clone(),
                    generated_marker_visibility: generated_marker_visibility.clone(),
                    query_matches: query_matches.cloned(),
                },
                counts,
            )
        });
        counts
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::Arc;

    use chrono::{DateTime, Duration, Utc};
    use gt_loaded_files::{FileHistory, LoadedFiles};
    use gt_types::sat_label::{SatLabelAnchor, SatLabelTier};
    use gt_types::time_types::GpsTime;
    use gt_types::{
        CustomMarker, EventMarker, FileSource, GeneratedMarker, GeneratedMarkerKind,
        GeneratedMarkerKindTag, Latitude, LoadedFile, LoadedTrack, Longitude, MarkerIcon, NavPoint,
        PointIdx, PositionProvenance, SolutionState, TimeRange, TrackLod, TrackMetadata, mercator,
    };
    use gt_ui_types::{DrawLayer, FileVisibility, TrackRanges, TrackVisibility};
    use rustc_hash::FxHashMap;
    use strum::IntoEnumIterator;

    use super::*;
    use crate::test_util;

    fn t(secs: i64) -> DateTime<Utc> {
        DateTime::<Utc>::UNIX_EPOCH + Duration::seconds(secs)
    }

    /// One track: four points at t0..t3 (point 1 report-bearing), anchors
    /// on points 0 and 3, two custom markers (t0, t2), one generated
    /// marker (t1), one event marker (t3).
    fn fixture() -> LoadedFile {
        let lat = Latitude::new(55.0);
        let lon = Longitude::new(12.0);
        let points: Vec<NavPoint> = (0..4)
            .map(|i| {
                let tpv = gt_types::TimePositionVelocity::builder()
                    .time(GpsTime::from_utc(t(i)))
                    .lat(lat)
                    .lon(lon)
                    .build();
                // Point 1 has an empty satellite report.
                let satellites =
                    (i == 1).then(|| gt_types::satellites::Satellites::new(None, None, Vec::new()));
                NavPoint::new(tpv, satellites)
            })
            .collect();
        let anchor = |pi, tier| SatLabelAnchor {
            point: PointIdx::new(pi),
            tier,
        };
        let track = LoadedTrack {
            metadata: TrackMetadata {
                time_range: TimeRange::new(t(0), t(3)),
                tpv_count: points.len(),
                invalid_position_count: 0,
                ..gt_test_utils::empty_track_metadata()
            },
            geometry: gt_test_utils::track_geometry(&points),
            points,
            lod: TrackLod::default(),
            sat_label_anchors: vec![
                anchor(0, SatLabelTier::Endpoint),
                anchor(3, SatLabelTier::Endpoint),
            ],
            custom_markers: vec![
                CustomMarker::new(t(0), "a".into(), MarkerIcon::Pin, lat, lon),
                CustomMarker::new(t(2), "b".into(), MarkerIcon::Pin, lat, lon),
            ],
            generated_markers: vec![GeneratedMarker {
                time: t(1),
                kind: GeneratedMarkerKind::GnssFixRegained {
                    fix_lost_duration: Duration::seconds(1),
                },
                lat,
                lon,
                merc: mercator::normalize(lat, lon),
            }],
            event_markers: vec![EventMarker::new(
                t(3),
                "Lap/Start".to_string(),
                None,
                lat,
                lon,
            )],
            channels: Vec::new(),
        };
        LoadedFile {
            metadata: gt_test_utils::empty_file_metadata(),
            tracks: vec![track],
            event_marker_styles: FxHashMap::default(),
            orphaned_event_markers: Vec::new(),
            source: FileSource::GtdPath(PathBuf::from("counts.gtd")),
            load_warnings: Vec::new(),
        }
    }

    fn supplied(snapped_tracks: Option<&SnappedTracks>) -> SuppliedCounts<'_> {
        SuppliedCounts {
            snapped_tracks,
            ..SuppliedCounts::default()
        }
    }

    fn vis_all() -> TrackDataVisibility {
        TrackDataVisibility {
            files: vec![FileVisibility {
                enabled: true,
                tracks: vec![TrackVisibility::all_visible()],
            }],
        }
    }

    fn compute(
        files: &[LoadedFile],
        visibility: &TrackDataVisibility,
        filter: &GlobalFilter,
        query_matches: Option<&QueryMatches>,
    ) -> DisplayCounts {
        DisplayCounts::compute(
            files,
            visibility,
            filter,
            &EventMarkerVisibility::default(),
            &GeneratedMarkerVisibility::default(),
            query_matches,
            SuppliedCounts::default(),
        )
    }

    #[test]
    fn unfiltered_fixture_counts_everything() {
        let files = vec![fixture()];
        let counts = compute(&files, &vis_all(), &GlobalFilter::default(), None);
        let expected = [
            (DisplayCategory::Tracks, 1),
            (DisplayCategory::TrackPoints, 3),
            (DisplayCategory::GhostFixes, 1),
            (DisplayCategory::SatelliteLabels, 2),
            (DisplayCategory::CustomMarkers, 2),
            (DisplayCategory::GeneratedMarkers, 1),
            (DisplayCategory::EventMarkers, 1),
            (DisplayCategory::QueryHighlights, 0),
            (DisplayCategory::SnappedTracks, 0),
            (DisplayCategory::SkyGlyphs, 0),
            // Supplied by the app, so a recording fixture contributes none.
            (DisplayCategory::JammingHexes, 0),
            (DisplayCategory::TecHeatmap, 0),
            (DisplayCategory::LogMatches, 0),
        ];
        assert_eq!(expected.len(), DisplayCategory::iter().count());
        for (category, n) in expected {
            assert_eq!(counts.get(category), n, "{category}");
        }
    }

    #[test]
    fn ordinary_and_estimated_counts_follow_resolved_navigation_state() {
        let mut files = vec![fixture()];
        let points = &mut files[0].tracks[0].points;
        points[0].navigation.state = Some(SolutionState::Fix);
        points[0].navigation.used_count = Some(0);
        points[1].navigation.state = Some(SolutionState::Unknown);
        points[1].navigation.used_count = Some(0);
        points[2].navigation.state = Some(SolutionState::EstimatedOnly);
        points[3].navigation.position_provenance = Some(PositionProvenance::SdkSynthesized);
        let counts = compute(&files, &vis_all(), &GlobalFilter::default(), None);
        assert_eq!(counts.get(DisplayCategory::TrackPoints), 2);
        assert_eq!(counts.get(DisplayCategory::GhostFixes), 2);
    }

    /// Snapped geometry arrives pre-scoped by tree visibility and the
    /// per-track toggle, and counts once per track whose recording the filter
    /// keeps - the renderer's own gate.
    #[rstest::rstest]
    #[case::the_filter_keeps_the_recording(GlobalFilter::default(), 1)]
    #[case::the_time_window_misses_the_recording(
        GlobalFilter::default().with_time_bounds(Some(t(10)), None),
        0
    )]
    #[case::the_recording_is_shorter_than_the_minimum_duration(
        GlobalFilter::default().with_minimum_duration(Some(Duration::seconds(60))),
        0
    )]
    fn snapped_tracks_are_counted_for_the_recordings_the_filter_keeps(
        #[case] filter: GlobalFilter,
        #[case] expected: usize,
    ) {
        let files = vec![fixture()];
        let mut snapped = SnappedTracks::default();
        snapped.insert(
            test_util::track0(),
            Arc::new(gt_ui_types::SnappedTrackGeometry::default()),
        );
        let counts = DisplayCounts::compute(
            &files,
            &vis_all(),
            &filter,
            &EventMarkerVisibility::default(),
            &GeneratedMarkerVisibility::default(),
            None,
            SuppliedCounts {
                snapped_tracks: Some(&snapped),
                ..SuppliedCounts::default()
            },
        );
        assert_eq!(counts.get(DisplayCategory::SnappedTracks), expected);
    }

    #[test]
    fn time_filter_trims_points_anchors_and_markers() {
        let files = vec![fixture()];
        let filter = GlobalFilter::default().with_time_bounds(Some(t(1)), Some(t(2)));
        let counts = compute(&files, &vis_all(), &filter, None);
        assert_eq!(counts.get(DisplayCategory::TrackPoints), 1);
        assert_eq!(counts.get(DisplayCategory::GhostFixes), 1);
        assert_eq!(counts.get(DisplayCategory::SatelliteLabels), 0);
        assert_eq!(counts.get(DisplayCategory::CustomMarkers), 1);
        assert_eq!(counts.get(DisplayCategory::GeneratedMarkers), 1);
        assert_eq!(counts.get(DisplayCategory::EventMarkers), 0);
    }

    #[test]
    fn tree_toggles_zero_their_category() {
        let files = vec![fixture()];
        let mut vis = vis_all();
        vis.files[0].tracks[0].set_category_visible(DataCategory::Tpv, false);
        vis.files[0].tracks[0].set_category_visible(DataCategory::CustomMarker, false);
        let counts = compute(&files, &vis, &GlobalFilter::default(), None);
        assert_eq!(counts.get(DisplayCategory::Tracks), 1);
        assert_eq!(counts.get(DisplayCategory::TrackPoints), 0);
        assert_eq!(counts.get(DisplayCategory::GhostFixes), 0);
        assert_eq!(counts.get(DisplayCategory::SkyGlyphs), 0);
        assert_eq!(counts.get(DisplayCategory::SatelliteLabels), 0);
        assert_eq!(counts.get(DisplayCategory::CustomMarkers), 0);
        assert_eq!(counts.get(DisplayCategory::GeneratedMarkers), 1);
    }

    #[test]
    fn disabled_track_zeroes_everything() {
        let files = vec![fixture()];
        let mut vis = vis_all();
        vis.files[0].tracks[0].enabled = false;
        let counts = compute(&files, &vis, &GlobalFilter::default(), None);
        assert_eq!(counts, DisplayCounts::default());
    }

    #[test]
    fn query_hide_and_draws_are_counted() {
        let files = vec![fixture()];
        let track_ref = test_util::track0();
        let matches = QueryMatches {
            // Points 0 and 1 removed by a `hide` query, which drops them from
            // the point count and drops the point-0 anchor.
            hidden: TrackRanges::from_iter([(track_ref, vec![0..1, 1..2])]),
            draws: vec![DrawLayer {
                color: 0,
                ranges: TrackRanges::from_iter([(track_ref, vec![2..3, 3..4])]),
            }],
            ..QueryMatches::default()
        };
        let counts = compute(&files, &vis_all(), &GlobalFilter::default(), Some(&matches));
        assert_eq!(counts.get(DisplayCategory::TrackPoints), 2);
        assert_eq!(counts.get(DisplayCategory::SatelliteLabels), 1);
        assert_eq!(counts.get(DisplayCategory::QueryHighlights), 2);
    }

    #[test]
    fn query_counts_follow_line_policy_while_fix_icons_are_hidden() {
        let files = vec![fixture()];
        let mut visibility = vis_all();
        visibility.files[0].tracks[0].set_category_visible(DataCategory::Tpv, false);
        let matches = crate::test_util::a_run_drawing(test_util::track0(), 0..2);
        let counts = compute(
            &files,
            &visibility,
            &GlobalFilter::default(),
            Some(&matches),
        );
        assert_eq!(counts.get(DisplayCategory::TrackPoints), 0);
        assert_eq!(counts.get(DisplayCategory::QueryHighlights), 1);
        visibility.files[0].tracks[0].set_category_visible(DataCategory::Track, false);
        let counts = compute(
            &files,
            &visibility,
            &GlobalFilter::default(),
            Some(&matches),
        );
        assert_eq!(counts.get(DisplayCategory::QueryHighlights), 0);
    }

    #[test]
    fn draws_outside_the_time_window_are_not_counted() {
        // A query ran before the filter was narrowed: its match on points
        // 0-1 is invisible now, so it must not be counted either.
        let files = vec![fixture()];
        let track_ref = test_util::track0();
        let matches = QueryMatches {
            draws: vec![DrawLayer {
                color: 0,
                ranges: TrackRanges::from_iter([(track_ref, vec![0..2, 2..4])]),
            }],
            ..QueryMatches::default()
        };
        let filter = GlobalFilter::default().with_time_bounds(Some(t(2)), None);
        let counts = compute(&files, &vis_all(), &filter, Some(&matches));
        assert_eq!(counts.get(DisplayCategory::QueryHighlights), 1);
    }

    /// The cache must never return a stale count: after any input `compute`
    /// reads changes, `get` must agree with a fresh `compute`. Each step
    /// varies exactly one input dimension so an incomplete key surfaces as a
    /// mismatch here. Steps repeat a state to exercise the (cheap) hit path.
    #[test]
    fn cache_agrees_with_compute_across_every_input_change() {
        let track_ref = test_util::track0();
        let mut files = LoadedFiles::new();
        files.push(fixture(), FileHistory::None);
        let files_more = {
            let mut f = vec![fixture()];
            let lat = Latitude::new(55.0);
            let lon = Longitude::new(12.0);
            f[0].tracks[0].custom_markers.push(CustomMarker::new(
                t(1),
                "c".into(),
                MarkerIcon::Pin,
                lat,
                lon,
            ));
            let mut loaded = LoadedFiles::new();
            loaded.push(f.remove(0), FileHistory::None);
            loaded
        };

        let vis_hidden = {
            let mut v = vis_all();
            v.files[0].tracks[0].set_category_visible(DataCategory::CustomMarker, false);
            v
        };
        let filter_window = GlobalFilter::default().with_time_bounds(Some(t(1)), Some(t(2)));
        let mut gmv_hidden = GeneratedMarkerVisibility::default();
        gmv_hidden.set_hidden(
            track_ref,
            std::iter::once(GeneratedMarkerKindTag::GnssFixRegained),
        );
        let mut emv_hidden = EventMarkerVisibility::default();
        emv_hidden.set_hidden(track_ref, std::iter::once("Lap".to_string()));
        let mut snapped = SnappedTracks::default();
        snapped.insert(
            track_ref,
            Arc::new(gt_ui_types::SnappedTrackGeometry::default()),
        );
        let query = QueryMatches {
            hidden: TrackRanges::from_iter([(track_ref, std::iter::once(0..1).collect())]),
            ..QueryMatches::default()
        };

        let emv0 = EventMarkerVisibility::default();
        let gmv0 = GeneratedMarkerVisibility::default();
        let filter0 = GlobalFilter::default();

        let mut cache = DisplayCountsCache::default();
        type Args<'a> = (
            &'a LoadedFiles,
            &'a TrackDataVisibility,
            &'a GlobalFilter,
            &'a EventMarkerVisibility,
            &'a GeneratedMarkerVisibility,
            Option<&'a QueryMatches>,
            Option<&'a SnappedTracks>,
        );
        let vis0 = vis_all();
        let steps: &[Args] = &[
            (&files, &vis0, &filter0, &emv0, &gmv0, None, None),
            // Repeat: hit path must still return the correct counts.
            (&files, &vis0, &filter0, &emv0, &gmv0, None, None),
            (&files, &vis_hidden, &filter0, &emv0, &gmv0, None, None),
            (&files, &vis0, &filter_window, &emv0, &gmv0, None, None),
            (&files, &vis0, &filter0, &emv0, &gmv_hidden, None, None),
            (&files, &vis0, &filter0, &emv_hidden, &gmv0, None, None),
            (&files, &vis0, &filter0, &emv0, &gmv0, Some(&query), None),
            (&files, &vis0, &filter0, &emv0, &gmv0, None, Some(&snapped)),
            (&files_more, &vis0, &filter0, &emv0, &gmv0, None, None),
            // Back to the baseline: the key must flip back too.
            (&files, &vis0, &filter0, &emv0, &gmv0, None, None),
        ];
        for &(f, v, fi, em, gm, q, s) in steps {
            let got = cache.get(MapFiles::Loaded(f.view()), v, fi, em, gm, q, supplied(s));
            let want = DisplayCounts::compute(f.files(), v, fi, em, gm, q, supplied(s));
            assert_eq!(got, want);
        }
    }

    /// Repeated calls with unchanged inputs run the O(all points) walk once. A
    /// changed input runs it again.
    #[test]
    fn cache_skips_the_walk_when_inputs_are_unchanged() {
        let mut files = LoadedFiles::new();
        files.push(fixture(), FileHistory::None);
        let vis = vis_all();
        let (emv, gmv) = (
            EventMarkerVisibility::default(),
            GeneratedMarkerVisibility::default(),
        );
        let mut cache = DisplayCountsCache::default();
        let get = |cache: &mut DisplayCountsCache, filter: &GlobalFilter| {
            cache.get(
                MapFiles::Loaded(files.view()),
                &vis,
                filter,
                &emv,
                &gmv,
                None,
                SuppliedCounts::default(),
            )
        };

        get(&mut cache, &GlobalFilter::default());
        get(&mut cache, &GlobalFilter::default());
        get(&mut cache, &GlobalFilter::default());
        assert_eq!(cache.computes, 1, "unchanged inputs must reuse the cache");

        let narrowed = GlobalFilter::default().with_time_bounds(Some(t(2)), None);
        get(&mut cache, &narrowed);
        assert_eq!(cache.computes, 2, "a changed input must recompute");
    }

    #[test]
    fn cache_recomputes_after_same_shaped_file_replacement() {
        let mut files = LoadedFiles::new();
        files.push(fixture(), FileHistory::None);
        let visibility = vis_all();
        let filter = GlobalFilter::default().with_time_bounds(Some(t(1)), Some(t(1)));
        let events = EventMarkerVisibility::default();
        let generated = GeneratedMarkerVisibility::default();
        let mut cache = DisplayCountsCache::default();
        let before = cache.get(
            MapFiles::Loaded(files.view()),
            &visibility,
            &filter,
            &events,
            &generated,
            None,
            SuppliedCounts::default(),
        );
        assert_eq!(before.get(DisplayCategory::CustomMarkers), 0);
        let mut replacement = fixture();
        replacement.tracks[0].custom_markers[0].time = t(1);
        files.replace_file(0, replacement, FileHistory::None);
        let after = cache.get(
            MapFiles::Loaded(files.view()),
            &visibility,
            &filter,
            &events,
            &generated,
            None,
            SuppliedCounts::default(),
        );
        assert_eq!(after.get(DisplayCategory::CustomMarkers), 1);
        assert_eq!(cache.computes, 2);
    }

    #[test]
    fn borrowed_files_are_counted_without_caching() {
        let files = [fixture()];
        let mut cache = DisplayCountsCache::default();
        for _ in 0..2 {
            cache.get(
                MapFiles::Borrowed(&files),
                &vis_all(),
                &GlobalFilter::default(),
                &EventMarkerVisibility::default(),
                &GeneratedMarkerVisibility::default(),
                None,
                SuppliedCounts::default(),
            );
        }
        assert_eq!(cache.computes, 2);
    }
}

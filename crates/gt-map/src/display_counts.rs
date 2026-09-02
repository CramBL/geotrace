//! Per-category counts of the elements currently in scope on the map.
//!
//! "In scope" means post-tree (file/track enablement and the per-category
//! tree toggles, including the per-kind marker refinements), post-filter
//! (track filter and time window), post-query (`keep`/`hide` point
//! removal), and pre-display-mask. The counts therefore report how much ink
//! a category would draw if displayed - which is what the display toggle
//! popup shows next to each row, and why a viewport-dependent number would
//! be wrong here.
//!
//! The gating comes from the crate's `scope` module, the same predicates
//! the renderers apply, so the counts cannot drift from what actually
//! draws.

use std::hash::{Hash, Hasher};
use std::ops::Range;

use gt_filter::{GlobalFilter, point_passes_time_filter, track_passes_filter};
use gt_types::{DataCategory, FileIdx, LoadedFile, LoadedTrack, TrackIdx, TrackRef};
use gt_ui_types::{
    DisplayCategory, EventMarkerVisibility, GeneratedMarkerVisibility, LogMatches, QueryMatches,
    SnappedTracks, TrackDataVisibility, visibility,
};
use rustc_hash::FxHasher;

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
    /// app to the shown logs and enabled filters. The global filter is applied
    /// by [`DisplayCounts::compute`], not before this field is filled.
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
    satellite_labels: usize,
    custom_markers: usize,
    generated_markers: usize,
    event_markers: usize,
    query_highlights: usize,
    snapped_tracks: usize,
    sky_glyphs: usize,
    /// Interference cells available for the shown day, from the archive.
    jamming_hexes: usize,
    /// TEC grid nodes available for the shown instant, from the archive.
    tec_heatmap: usize,
    /// Hexagons the loaded logs' filters selected, those the global filter
    /// keeps.
    log_matches: usize,
}

impl DisplayCounts {
    pub fn get(self, category: DisplayCategory) -> usize {
        match category {
            DisplayCategory::Tracks => self.tracks,
            DisplayCategory::TrackPoints => self.track_points,
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
            satellite_labels: get(DisplayCategory::SatelliteLabels),
            custom_markers: get(DisplayCategory::CustomMarkers),
            generated_markers: get(DisplayCategory::GeneratedMarkers),
            event_markers: get(DisplayCategory::EventMarkers),
            query_highlights: get(DisplayCategory::QueryHighlights),
            snapped_tracks: get(DisplayCategory::SnappedTracks),
            sky_glyphs: get(DisplayCategory::SkyGlyphs),
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
        let mut counts = Self {
            jamming_hexes: supplied.jamming_cells,
            tec_heatmap: supplied.tec_nodes,
            log_matches: supplied
                .log_matches
                .map_or(0, |matches| matches.count_passing_filter(files, filter)),
            // The count is per track, like "Tracks": how many snapped tracks
            // are eligible to draw.
            snapped_tracks: supplied.snapped_tracks.map_or(0, |snapped| {
                snapped
                    .iter()
                    .filter(|(track_ref, _)| {
                        track_ref
                            .resolve(files)
                            .is_some_and(|track| track_passes_filter(track, filter))
                    })
                    .count()
            }),
            ..Self::default()
        };
        for (fi, file) in files.iter().enumerate() {
            for ti in 0..file.tracks.len() {
                let track_ref = TrackRef::new(FileIdx::new(fi), TrackIdx::new(ti));
                let Some((track, track_vis)) =
                    visibility::track_in_scope(files, visibility, filter, track_ref)
                else {
                    continue;
                };
                const NO_RANGES: &[Range<usize>] = &[];
                let hidden_ranges = query_matches.map_or(NO_RANGES, |m| m.hidden_ranges(track_ref));
                let point_in_scope = |pi: usize| {
                    track.points.get(pi).is_some_and(|p| {
                        point_passes_time_filter(p.tpv.time().utc(), filter)
                            && QueryMatches::range_at(hidden_ranges, pi).is_none()
                    })
                };

                if track_vis.category_visible(DataCategory::Track) {
                    // Counted per track regardless of query-hide coverage:
                    // "Tracks" counts whether this track's line is eligible
                    // to draw, not whether it currently draws any pixels.
                    counts.tracks += 1;
                }
                if track_vis.category_visible(DataCategory::Tpv) {
                    counts.track_points += (0..track.points.len())
                        .filter(|&pi| point_in_scope(pi))
                        .count();
                    counts.satellite_labels += track
                        .sat_label_anchors
                        .iter()
                        .filter(|a| point_in_scope(a.point.as_usize()))
                        .count();
                    // Sky glyphs anchor on report-bearing points. Like the
                    // labels they are per-point track ink, so they share the
                    // Tpv tree gate.
                    counts.sky_glyphs += track
                        .points
                        .iter()
                        .enumerate()
                        .filter(|(pi, p)| p.satellites.is_some() && point_in_scope(*pi))
                        .count();
                }
                if track_vis.category_visible(DataCategory::CustomMarker) {
                    counts.custom_markers += track
                        .custom_markers
                        .iter()
                        .filter(|m| point_passes_time_filter(m.time, filter))
                        .count();
                }
                if track_vis.category_visible(DataCategory::GeneratedMarker) {
                    counts.generated_markers += track
                        .generated_markers
                        .iter()
                        .filter(|m| {
                            point_passes_time_filter(m.time, filter)
                                && generated_marker_visibility.is_visible(track_ref, m.kind.tag())
                        })
                        .count();
                }
                if track_vis.category_visible(DataCategory::EventMarker) {
                    counts.event_markers += track
                        .event_markers
                        .iter()
                        .filter(|m| {
                            point_passes_time_filter(m.time, filter)
                                && event_marker_visibility.is_visible(track_ref, &m.variant_path)
                        })
                        .count();
                }
                // A match counts while any of its points is in scope - the
                // halo renderer paints over the filtered point path, so a
                // range fully outside the time window draws no ink.
                counts.query_highlights += query_matches.map_or(0, |m| {
                    m.draws
                        .iter()
                        .flat_map(|layer| layer.ranges_for(track_ref))
                        .filter(|&range| range.clone().any(&point_in_scope))
                        .count()
                });
            }
        }
        counts
    }
}

/// The inputs [`DisplayCounts::compute`] reads, captured so a cache can detect
/// when a recompute is needed. Compared by value: equal keys guarantee equal
/// counts.
///
/// `files` is fingerprinted structurally from the per-track array lengths plus
/// the file and track counts. Point and marker data is immutable once loaded,
/// so those change exactly when a load, unload, or reload changes what could be
/// counted. `snapped_tracks` collapses to the tracks it holds, the only thing
/// `compute` reads from it.
#[derive(Clone, PartialEq)]
struct DisplayCountsKey {
    files_sig: u64,
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

/// A structural fingerprint of the loaded data: the file/track shape and each
/// track's per-array element counts. O(tracks), not O(points), so it is cheap
/// to compute every frame the popup is open.
///
/// [`LoadedFile`] and [`LoadedTrack`] are destructured exhaustively (no `..`):
/// a new field on either is a compile error here, forcing a decision about
/// whether it changes what [`DisplayCounts::compute`] counts - so the cache
/// key can never silently miss a new source of countable data. Fields the
/// counts do not depend on are bound to `_` deliberately.
fn files_signature(files: &[LoadedFile]) -> u64 {
    let mut hasher = FxHasher::default();
    files.len().hash(&mut hasher);
    for file in files {
        let LoadedFile {
            tracks,
            metadata: _,
            event_marker_styles: _,
            orphaned_event_markers: _,
            source: _,
            load_warnings: _,
        } = file;
        tracks.len().hash(&mut hasher);
        for track in tracks {
            let LoadedTrack {
                points,
                sat_label_anchors,
                custom_markers,
                generated_markers,
                event_markers,
                metadata: _,
                geometry: _,
                lod: _,
                channels: _,
            } = track;
            points.len().hash(&mut hasher);
            custom_markers.len().hash(&mut hasher);
            generated_markers.len().hash(&mut hasher);
            event_markers.len().hash(&mut hasher);
            sat_label_anchors.len().hash(&mut hasher);
        }
    }
    hasher.finish()
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
        files: &[LoadedFile],
        visibility: &TrackDataVisibility,
        filter: &GlobalFilter,
        event_marker_visibility: &EventMarkerVisibility,
        generated_marker_visibility: &GeneratedMarkerVisibility,
        query_matches: Option<&QueryMatches>,
        supplied: SuppliedCounts<'_>,
    ) -> DisplayCounts {
        let files_sig = files_signature(files);
        let snapped_track_refs: Vec<TrackRef> = supplied
            .snapped_tracks
            .map(|snapped| snapped.iter().map(|(track_ref, _)| track_ref).collect())
            .unwrap_or_default();
        if let Some((key, counts)) = &self.cached
            && key.files_sig == files_sig
            && key.snapped_track_refs == snapped_track_refs
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
            files,
            visibility,
            filter,
            event_marker_visibility,
            generated_marker_visibility,
            query_matches,
            supplied,
        );
        self.cached = Some((
            DisplayCountsKey {
                jamming_cells: supplied.jamming_cells,
                tec_nodes: supplied.tec_nodes,
                log_matches: supplied.log_matches.cloned(),
                files_sig,
                snapped_track_refs,
                filter: *filter,
                visibility: visibility.clone(),
                event_marker_visibility: event_marker_visibility.clone(),
                generated_marker_visibility: generated_marker_visibility.clone(),
                query_matches: query_matches.cloned(),
            },
            counts,
        ));
        counts
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::Arc;

    use chrono::{DateTime, Duration, Utc};
    use gt_types::sat_label::{SatLabelAnchor, SatLabelTier};
    use gt_types::time_types::GpsTime;
    use gt_types::{
        CustomMarker, EventMarker, FileSource, GeneratedMarker, GeneratedMarkerKind,
        GeneratedMarkerKindTag, Latitude, LoadedFile, LoadedTrack, Longitude, MarkerIcon, NavPoint,
        PointIdx, TimeRange, TrackLod, TrackMetadata, mercator,
    };
    use gt_ui_types::{DrawLayer, FileVisibility, TrackRanges, TrackVisibility};
    use rustc_hash::FxHashMap;
    use strum::IntoEnumIterator;

    use super::*;

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
                // Point 1 has a satellite report, so it anchors a sky glyph.
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
            (DisplayCategory::TrackPoints, 4),
            (DisplayCategory::SatelliteLabels, 2),
            (DisplayCategory::CustomMarkers, 2),
            (DisplayCategory::GeneratedMarkers, 1),
            (DisplayCategory::EventMarkers, 1),
            (DisplayCategory::QueryHighlights, 0),
            (DisplayCategory::SnappedTracks, 0),
            (DisplayCategory::SkyGlyphs, 1),
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

    /// Snapped geometry arrives pre-scoped by tree visibility and the
    /// per-track toggle, and counts once per track whose recording the filter
    /// keeps - the renderer's own gate.
    #[rstest::rstest]
    #[case::the_filter_keeps_the_recording(GlobalFilter::default(), 1)]
    #[case::the_time_window_misses_the_recording(
        GlobalFilter { time_start: Some(t(10)), ..GlobalFilter::default() },
        0
    )]
    #[case::the_recording_is_shorter_than_the_minimum_duration(
        GlobalFilter { min_duration: Some(Duration::seconds(60)), ..GlobalFilter::default() },
        0
    )]
    fn snapped_tracks_are_counted_for_the_recordings_the_filter_keeps(
        #[case] filter: GlobalFilter,
        #[case] expected: usize,
    ) {
        let files = vec![fixture()];
        let mut snapped = SnappedTracks::default();
        snapped.insert(
            TrackRef::new(FileIdx::new(0), TrackIdx::new(0)),
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
        let filter = GlobalFilter {
            time_start: Some(t(1)),
            time_end: Some(t(2)),
            ..GlobalFilter::default()
        };
        let counts = compute(&files, &vis_all(), &filter, None);
        // Points t1 and t2 pass, both anchors (t0, t3) fall outside. The t2
        // custom marker and the t1 generated marker stay.
        assert_eq!(counts.get(DisplayCategory::TrackPoints), 2);
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
        let track_ref = TrackRef::new(FileIdx::new(0), TrackIdx::new(0));
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
    fn draws_outside_the_time_window_are_not_counted() {
        // A query ran before the filter was narrowed: its match on points
        // 0-1 now draws no halo ink, so it must not be counted either.
        let files = vec![fixture()];
        let track_ref = TrackRef::new(FileIdx::new(0), TrackIdx::new(0));
        let matches = QueryMatches {
            draws: vec![DrawLayer {
                color: 0,
                ranges: TrackRanges::from_iter([(track_ref, vec![0..2, 2..4])]),
            }],
            ..QueryMatches::default()
        };
        let filter = GlobalFilter {
            time_start: Some(t(2)),
            ..GlobalFilter::default()
        };
        let counts = compute(&files, &vis_all(), &filter, Some(&matches));
        assert_eq!(counts.get(DisplayCategory::QueryHighlights), 1);
    }

    /// The cache must never return a stale count: after any input `compute`
    /// reads changes, `get` must agree with a fresh `compute`. Each step
    /// varies exactly one input dimension so an incomplete key surfaces as a
    /// mismatch here. Steps repeat a state to exercise the (cheap) hit path.
    #[test]
    fn cache_agrees_with_compute_across_every_input_change() {
        let track_ref = TrackRef::new(FileIdx::new(0), TrackIdx::new(0));
        let files = vec![fixture()];
        // A second file state with an extra custom marker: same track shape
        // otherwise, so only the structural files signature distinguishes it.
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
            f
        };

        let vis_hidden = {
            let mut v = vis_all();
            v.files[0].tracks[0].set_category_visible(DataCategory::CustomMarker, false);
            v
        };
        let filter_window = GlobalFilter {
            time_start: Some(t(1)),
            time_end: Some(t(2)),
            ..GlobalFilter::default()
        };
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
        // (files, visibility, filter, emv, gmv, query, snapped)
        type Args<'a> = (
            &'a [LoadedFile],
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
            let got = cache.get(f, v, fi, em, gm, q, supplied(s));
            let want = DisplayCounts::compute(f, v, fi, em, gm, q, supplied(s));
            assert_eq!(got, want);
        }
    }

    /// Repeated calls with unchanged inputs run the O(all points) walk once. A
    /// changed input runs it again.
    #[test]
    fn cache_skips_the_walk_when_inputs_are_unchanged() {
        let files = vec![fixture()];
        let vis = vis_all();
        let (emv, gmv) = (
            EventMarkerVisibility::default(),
            GeneratedMarkerVisibility::default(),
        );
        let mut cache = DisplayCountsCache::default();
        let get = |cache: &mut DisplayCountsCache, filter: &GlobalFilter| {
            cache.get(
                &files,
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

        let narrowed = GlobalFilter {
            time_start: Some(t(2)),
            ..GlobalFilter::default()
        };
        get(&mut cache, &narrowed);
        assert_eq!(cache.computes, 2, "a changed input must recompute");
    }
}

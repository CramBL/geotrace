//! Per-category counts of the elements currently in scope on the map.
//!
//! "In scope" means post-tree (file/track enablement and the per-category
//! tree toggles, including the per-kind marker refinements), post-filter
//! (track filter and time window), post-query (`keep`/`hide` point
//! removal), and pre-display-mask. The counts therefore answer "how much
//! ink would this category draw if displayed" - which is what the display
//! toggle popup shows next to each row, and why a viewport-dependent
//! number would be wrong here.
//!
//! TODO(display-mask): the in-scope predicates here re-derive the gating
//! that each renderer also implements (see the marker renderers and
//! `track_layers`). Extract shared scope predicates so the counts cannot
//! drift from what actually draws.

use std::ops::Range;

use gt_filter::{GlobalFilter, point_passes_time_filter, track_passes_filter};
use gt_types::{FileIdx, LoadedFile, TrackIdx, TrackRef};
use gt_ui_types::{
    DisplayCategory, EventMarkerVisibility, GeneratedMarkerVisibility, QueryMatches,
    TrackDataVisibility,
};

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
        }
    }

    pub fn compute(
        files: &[LoadedFile],
        visibility: &TrackDataVisibility,
        filter: &GlobalFilter,
        event_marker_visibility: &EventMarkerVisibility,
        generated_marker_visibility: &GeneratedMarkerVisibility,
        query_matches: Option<&QueryMatches>,
    ) -> Self {
        let mut counts = Self::default();
        for (fi, file) in files.iter().enumerate() {
            let file_vis = FileIdx::new(fi).get(&visibility.files);
            if !file_vis.is_some_and(|fv| fv.enabled) {
                continue;
            }
            for (ti, track) in file.tracks.iter().enumerate() {
                let Some(trip_vis) = file_vis.and_then(|fv| TrackIdx::new(ti).get(&fv.tracks))
                else {
                    continue;
                };
                if !trip_vis.enabled || !track_passes_filter(&track.metadata, filter) {
                    continue;
                }
                let track_ref = TrackRef::new(FileIdx::new(fi), TrackIdx::new(ti));
                const NO_RANGES: &[Range<usize>] = &[];
                let hidden_ranges = query_matches.map_or(NO_RANGES, |m| m.hidden_ranges(track_ref));
                let point_in_scope = |pi: usize| {
                    track.points.get(pi).is_some_and(|p| {
                        point_passes_time_filter(p.tpv.time().utc(), filter)
                            && QueryMatches::range_at(hidden_ranges, pi).is_none()
                    })
                };

                if trip_vis.track_visible {
                    // Counted per track regardless of query-hide coverage:
                    // "Tracks" answers "is this track's line eligible to
                    // draw", not "does it currently draw any pixels".
                    counts.tracks += 1;
                }
                if trip_vis.tpv_visible {
                    counts.track_points += (0..track.points.len())
                        .filter(|&pi| point_in_scope(pi))
                        .count();
                    counts.satellite_labels += track
                        .sat_label_anchors
                        .iter()
                        .filter(|a| point_in_scope(a.point.as_usize()))
                        .count();
                }
                if trip_vis.custom_markers_visible {
                    counts.custom_markers += track
                        .custom_markers
                        .iter()
                        .filter(|m| point_passes_time_filter(m.time, filter))
                        .count();
                }
                if trip_vis.generated_markers_visible {
                    counts.generated_markers += track
                        .generated_markers
                        .iter()
                        .filter(|m| {
                            point_passes_time_filter(m.time, filter)
                                && generated_marker_visibility.is_visible(track_ref, m.kind.tag())
                        })
                        .count();
                }
                if trip_vis.event_markers_visible {
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

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::PathBuf;

    use chrono::{DateTime, Duration, Utc};
    use gt_types::sat_label::{SatLabelAnchor, SatLabelTier};
    use gt_types::time_types::GpsTime;
    use gt_types::{
        CustomMarker, EventMarker, FileMetadata, FileSource, GeneratedMarker, GeneratedMarkerKind,
        Latitude, LoadedFile, LoadedTrack, Longitude, MarkerIcon, NavPoint, PointIdx, TimeRange,
        TrackLod, TrackMetadata, mercator,
    };
    use gt_ui_types::{DrawLayer, FileVisibility, TrackVisibility};
    use strum::IntoEnumIterator;

    use super::*;

    fn t(secs: i64) -> DateTime<Utc> {
        DateTime::<Utc>::UNIX_EPOCH + Duration::seconds(secs)
    }

    /// One track: four points at t0..t3, anchors on points 0 and 3, two
    /// custom markers (t0, t2), one generated marker (t1), one event
    /// marker (t3).
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
                NavPoint::new(tpv, None)
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
                ..TrackMetadata::default()
            },
            points,
            lod: TrackLod::default(),
            sat_label_anchors: vec![
                anchor(0, SatLabelTier::Endpoint),
                anchor(3, SatLabelTier::Endpoint),
            ],
            custom_markers: vec![
                CustomMarker::new(t(0), "a".into(), MarkerIcon::Pin, lat, lon, None),
                CustomMarker::new(t(2), "b".into(), MarkerIcon::Pin, lat, lon, None),
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
            metadata: FileMetadata::default(),
            tracks: vec![track],
            event_marker_styles: HashMap::new(),
            orphaned_event_markers: Vec::new(),
            source: FileSource::GtdPath(PathBuf::from("counts.gtd")),
            load_warnings: Vec::new(),
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
        ];
        assert_eq!(expected.len(), DisplayCategory::iter().count());
        for (category, n) in expected {
            assert_eq!(counts.get(category), n, "{category}");
        }
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
        // Points t1 and t2 pass; both anchors (t0, t3) fall outside; the
        // t2 custom marker and the t1 generated marker stay.
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
        vis.files[0].tracks[0].tpv_visible = false;
        vis.files[0].tracks[0].custom_markers_visible = false;
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
            // Points 0 and 1 removed by a `hide` query: they leave the
            // point count and take the point-0 anchor with them.
            hidden: HashMap::from([(track_ref, vec![0..1, 1..2])]),
            draws: vec![DrawLayer {
                color: 0,
                ranges: HashMap::from([(track_ref, vec![2..3, 3..4])]),
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
                ranges: HashMap::from([(track_ref, vec![0..2, 2..4])]),
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
}

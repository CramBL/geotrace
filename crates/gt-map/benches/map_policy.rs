//! Architectural performance baselines for map filtering and visibility.
//!
//! These deliberately stay few and coarse. They guard the O(tracks) frame
//! preparation requirement, candidate-local policy cost, and nearest-first
//! rejection of hidden spatial candidates without becoming a catalogue of
//! component benchmarks.

#![allow(
    clippy::restriction,
    clippy::allow_attributes,
    reason = "benchmark: development-only code"
)]

use std::hint;
use std::iter;
use std::ops::Range;

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use gt_filter::GlobalFilter;
use gt_map::{benchmark_support, test_util};
use gt_track_builder::SpatialIndex;
use gt_types::{DataCategory, FixRef, LoadedFile, NavPoint, PointIdx, SpatialPoint};
use gt_ui_types::{
    DisplayMask, EventMarkerVisibility, GeneratedMarkerVisibility, MapElementRef, MapScope,
    QueryMatches, TrackDataVisibility, TrackRanges,
};

const BENCH_ZOOM: f64 = 15.0;
const TRACK_STEP_DEGREES: f64 = 0.000_01;

fn recording_with_tracks(track_count: usize, points_per_track: usize) -> Vec<LoadedFile> {
    let mut files = test_util::a_recording_of(points_per_track, TRACK_STEP_DEGREES);
    let file = files
        .first_mut()
        .expect("a_recording_of always creates one file");
    let template = file
        .tracks
        .first()
        .cloned()
        .expect("a positive point count creates one track");
    file.tracks = (0..track_count)
        .map(|index| {
            let mut track = template.clone();
            track.metadata.index = index + 1;
            track
        })
        .collect();
    files
}

fn bench_frame_policy_scaling(c: &mut Criterion) {
    let mut group = c.benchmark_group("frame_policy");
    for (name, track_count, points_per_track) in [
        ("8_tracks_100_points", 8, 100),
        ("128_tracks_100_points", 128, 100),
        ("1024_tracks_100_points", 1_024, 100),
        ("128_tracks_10000_points", 128, 10_000),
    ] {
        let files = recording_with_tracks(track_count, points_per_track);
        let visibility = TrackDataVisibility::from_loaded(&files);
        let filter = GlobalFilter::default();
        let display_mask = DisplayMask::default();
        group.throughput(Throughput::Elements(track_count as u64));
        group.bench_function(BenchmarkId::from_parameter(name), |b| {
            b.iter(|| {
                benchmark_support::compile_track_plan(
                    hint::black_box(&files),
                    hint::black_box(&visibility),
                    hint::black_box(&filter),
                    display_mask,
                    BENCH_ZOOM,
                );
            });
        });
    }
    group.finish();
}

fn point_ref(category: DataCategory, point_index: usize) -> MapElementRef {
    test_util::point_ref(category, point_index)
}

fn hidden_range(range: Range<usize>) -> QueryMatches {
    QueryMatches {
        hidden: TrackRanges::from_iter([(test_util::track0(), std::iter::once(range).collect())]),
        ..QueryMatches::default()
    }
}

fn bench_candidate_resolution(c: &mut Criterion) {
    let mut files = vec![test_util::a_recording_with_every_marker_kind()];
    let track = files
        .first_mut()
        .and_then(|file| file.tracks.first_mut())
        .expect("the marker fixture has one track");
    let ghost_point = track
        .points
        .get_mut(2)
        .expect("the marker fixture has at least three fixes");
    let ghost_time = ghost_point.tpv.time();
    ghost_point.satellites = Some(gt_types::satellites::Satellites::new(
        Some(ghost_time),
        None,
        Vec::new(),
    ));

    let track = files
        .first()
        .and_then(|file| file.tracks.first())
        .expect("the marker fixture has one track");
    let measured_index = track
        .points
        .iter()
        .position(|point| !point.is_ghost_fix())
        .expect("the marker fixture has a measured fix");
    let ghost_index = track
        .points
        .iter()
        .position(NavPoint::is_ghost_fix)
        .expect("the marker fixture has a ghost fix");
    let outside_time_index = 20;
    let time_end = track
        .points
        .get(10)
        .expect("the marker fixture has more than ten fixes")
        .tpv
        .time()
        .utc();
    let generated_kind = track
        .generated_markers
        .first()
        .expect("the marker fixture has a generated marker")
        .kind
        .tag();
    let event_path = &track
        .event_markers
        .first()
        .expect("the marker fixture has an event marker")
        .variant_path;
    let event_parent = event_path
        .split('/')
        .next()
        .expect("the fixture event path is non-empty")
        .to_owned();

    let visibility = TrackDataVisibility::from_loaded(&files);
    let mut generated_visibility = GeneratedMarkerVisibility::default();
    generated_visibility.set_hidden(test_util::track0(), iter::once(generated_kind));
    let mut event_visibility = EventMarkerVisibility::default();
    event_visibility.set_hidden(test_util::track0(), iter::once(event_parent));
    let filter = GlobalFilter::default().with_time_bounds(None, Some(time_end));
    let query_matches = hidden_range(measured_index..measured_index + 1);
    let scope = MapScope {
        files: &files,
        visibility: &visibility,
        event_marker_visibility: &event_visibility,
        generated_marker_visibility: &generated_visibility,
        filter: &filter,
        display_mask: DisplayMask::default(),
        query_matches: Some(&query_matches),
    };
    let workload = [
        point_ref(DataCategory::Tpv, measured_index + 1),
        point_ref(DataCategory::SatelliteReport, measured_index + 1),
        point_ref(DataCategory::Tpv, measured_index),
        point_ref(DataCategory::Tpv, ghost_index),
        point_ref(DataCategory::Tpv, outside_time_index),
        point_ref(DataCategory::CustomMarker, 0),
        point_ref(DataCategory::GeneratedMarker, 0),
        point_ref(DataCategory::EventMarker, 0),
        point_ref(DataCategory::Tpv, track.points.len() + 7),
    ];

    let mut group = c.benchmark_group("candidate_resolution");
    group.throughput(Throughput::Elements(workload.len() as u64));
    group.bench_function("mixed", |b| {
        b.iter(|| {
            for point in workload {
                hint::black_box(scope.point_visibility(hint::black_box(point)));
            }
        });
    });
    group.finish();
}

fn data_point_ref(point: &SpatialPoint) -> MapElementRef {
    MapElementRef::Fix(FixRef::new(point.track_ref(), point.point_index))
}

fn nearest_visible_fix(
    spatial_index: &SpatialIndex,
    cursor: [f64; 2],
    scope: MapScope<'_>,
) -> Option<MapElementRef> {
    spatial_index
        .fixes
        .nearest_neighbor_iter(cursor)
        .find_map(|point| {
            benchmark_support::spatial_point_visible(point, scope).then_some(data_point_ref(point))
        })
}

fn bench_nearest_candidate(c: &mut Criterion) {
    let files = test_util::a_recording_of(16, TRACK_STEP_DEGREES);
    let spatial_index = SpatialIndex::build(&files);
    let first = spatial_index
        .fixes
        .iter()
        .find(|point| point.point_index == PointIdx::new(0))
        .expect("the spatial fixture indexes its first fix");
    let cursor = [first.merc.x, first.merc.y];
    let visibility = TrackDataVisibility::from_loaded(&files);
    let event_visibility = EventMarkerVisibility::default();
    let generated_visibility = GeneratedMarkerVisibility::default();
    let filter = GlobalFilter::default();
    let hidden = hidden_range(0..8);
    let visible_scope = MapScope {
        files: &files,
        visibility: &visibility,
        event_marker_visibility: &event_visibility,
        generated_marker_visibility: &generated_visibility,
        filter: &filter,
        display_mask: DisplayMask::default(),
        query_matches: None,
    };
    let hidden_scope = MapScope {
        query_matches: Some(&hidden),
        ..visible_scope
    };

    let mut group = c.benchmark_group("nearest_candidate");
    group.bench_function("visible_first", |b| {
        b.iter(|| {
            hint::black_box(nearest_visible_fix(
                hint::black_box(&spatial_index),
                cursor,
                visible_scope,
            ));
        });
    });
    group.bench_function("8_hidden_then_visible", |b| {
        b.iter(|| {
            hint::black_box(nearest_visible_fix(
                hint::black_box(&spatial_index),
                cursor,
                hidden_scope,
            ));
        });
    });
    group.finish();
}

criterion_group!(
    benches,
    bench_frame_policy_scaling,
    bench_candidate_resolution,
    bench_nearest_candidate
);
criterion_main!(benches);

use chrono::TimeDelta;
use gt_filter::{GlobalFilter, TimeWindow};
use gt_types::fixtures::{self, FixKind};
use gt_types::sat_label::{SatLabelAnchor, SatLabelTier};
use gt_types::satellites::{Constellation, Satellite, Satellites};
use gt_types::{
    DataCategory, EventMarker, FileIdx, GeneratedMarker, GeneratedMarkerKind,
    GeneratedMarkerKindTag, GeoBounds, Latitude, LoadedFile, Longitude, PointIdx,
    SatelliteEvidence, TrackIdx, TrackRef, mercator,
};
use gt_ui_types::{
    DisplayCategory, DisplayMask, EventMarkerVisibility, GeneratedMarkerVisibility, MapEligibility,
    MapPresence, QueryMatches, SkyGlyphVariant, TrackDataVisibility, TrackRanges,
};

use crate::test_util::{self, MapScene};

struct Policy {
    files: Vec<LoadedFile>,
    visibility: TrackDataVisibility,
    filter: GlobalFilter,
    matches: QueryMatches,
    generated: GeneratedMarkerVisibility,
    events: EventMarkerVisibility,
    display: DisplayMask,
}

impl Policy {
    fn fixture() -> Self {
        let mut files = vec![test_util::a_recording_with_every_marker_kind()];
        let track = &mut files[0].tracks[0];
        let time = track.points[0].tpv.time().utc();
        track.generated_markers = vec![GeneratedMarker::new(
            time,
            GeneratedMarkerKind::GnssFixLost,
            Latitude::new(55.0),
            Longitude::new(90.0),
        )];
        track.event_markers = vec![EventMarker::new(
            time,
            "power/boot".to_owned(),
            None,
            Latitude::new(55.0),
            Longitude::new(100.0),
        )];
        Self {
            visibility: TrackDataVisibility::from_loaded(&files),
            files,
            filter: GlobalFilter::default(),
            matches: QueryMatches::default(),
            generated: GeneratedMarkerVisibility::default(),
            events: EventMarkerVisibility::default(),
            display: DisplayMask::default(),
        }
    }

    fn scope(&self) -> MapPresence<'_> {
        MapEligibility::new(
            &self.files,
            &self.visibility,
            &self.filter,
            Some(&self.matches),
            &self.generated,
            &self.events,
        )
        .with_display_mask(self.display)
    }
}

#[test]
fn fit_uses_the_selected_sky_glyph_geometry() {
    let latitude = Latitude::new(55.0);
    let points = [12.0, 90.0, 14.0]
        .into_iter()
        .enumerate()
        .map(|(index, longitude)| {
            let mut point = fixtures::nav_point(
                test_util::epoch() + TimeDelta::seconds(index as i64),
                latitude,
                Longitude::new(longitude),
                FixKind::GhostWithoutHeading,
            );
            if index == 1 {
                let report = Satellites::new(
                    None,
                    None,
                    vec![Satellite::new(
                        Constellation::Gps,
                        1,
                        None,
                        Some(45.0),
                        None,
                        false,
                    )],
                );
                point.satellite_evidence = SatelliteEvidence::from_legacy(&report, &point.tpv);
                point.satellite_evidence.usage.clear();
            }
            point
        })
        .collect();
    let mut policy = Policy::fixture();
    policy.files = vec![gt_test_utils::loaded_file_with_tracks(vec![
        gt_test_utils::loaded_track_with_points(points),
    ])];
    policy.visibility = TrackDataVisibility::from_loaded(&policy.files);
    policy.display.solo(DisplayCategory::SkyGlyphs);
    let expected = |longitudes: &[f64]| {
        GeoBounds::from_positions(
            longitudes
                .iter()
                .map(|longitude| (latitude, Longitude::new(*longitude))),
        )
    };
    assert_eq!(
        super::visible_bounds(policy.scope(), SkyGlyphVariant::Ring),
        expected(&[12.0, 90.0])
    );
    assert_eq!(
        super::visible_bounds(policy.scope(), SkyGlyphVariant::Disc),
        expected(&[12.0, 14.0])
    );
}

#[rstest::rstest]
#[case::generated(DisplayCategory::GeneratedMarkers, false)]
#[case::event(DisplayCategory::EventMarkers, false)]
#[case::generated_kind_hidden(DisplayCategory::GeneratedMarkers, true)]
#[case::event_parent_hidden(DisplayCategory::EventMarkers, true)]
fn fit_frames_marker_only_scenes_and_applies_refinements(
    #[case] category: DisplayCategory,
    #[case] hidden: bool,
) {
    let mut policy = Policy::fixture();
    policy.display.solo(category);
    if hidden {
        policy.generated.set_hidden(
            test_util::track0(),
            std::iter::once(GeneratedMarkerKindTag::GnssFixLost),
        );
        policy
            .events
            .set_hidden(test_util::track0(), std::iter::once("power".to_owned()));
    }
    let bounds = super::visible_bounds(policy.scope(), SkyGlyphVariant::default());
    let expected_lon = if category == DisplayCategory::GeneratedMarkers {
        90.0
    } else {
        100.0
    };
    assert_eq!(
        bounds.map(|bounds| bounds.center().1),
        (!hidden).then_some(Longitude::new(expected_lon))
    );
}

#[rstest::rstest]
#[case::track_line(DisplayCategory::Tracks, DataCategory::Track)]
#[case::fix_icons(DisplayCategory::TrackPoints, DataCategory::Tpv)]
#[case::satellite_labels(DisplayCategory::SatelliteLabels, DataCategory::Tpv)]
fn fit_respects_each_manifestations_tree_gate(
    #[case] display: DisplayCategory,
    #[case] category: DataCategory,
) {
    let mut policy = Policy::fixture();
    policy.display.solo(display);
    assert!(super::visible_bounds(policy.scope(), SkyGlyphVariant::default()).is_some());
    policy.visibility.files[0].tracks[0].set_category_visible(category, false);
    assert!(super::visible_bounds(policy.scope(), SkyGlyphVariant::default()).is_none());
}

#[test]
fn fit_and_reveal_exclude_query_hidden_and_time_filtered_distant_fixes() {
    let mut policy = Policy::fixture();
    policy.display.solo(DisplayCategory::Tracks);
    policy
        .display
        .set_visible(DisplayCategory::QueryHighlights, true);
    let track = TrackRef::new(FileIdx::new(0), TrackIdx::new(0));
    policy.matches = test_util::a_run_drawing(track, 0..3);
    policy.matches.hidden = TrackRanges::from_iter([(track, std::iter::once(2..3).collect())]);
    let second_time = policy.files[0].tracks[0].points[1].tpv.time().utc();
    policy
        .filter
        .set_time_bounds(None, Some(second_time - TimeDelta::nanoseconds(1)));
    let expected = policy.files[0].tracks[0]
        .resolved_position_at(0)
        .expect("fixture geometry");
    for bounds in [
        super::visible_bounds(policy.scope(), SkyGlyphVariant::default()),
        super::query_bounds(policy.scope(), &policy.matches),
        super::query_match_bounds(policy.scope(), track, &(0..3)),
    ] {
        assert_eq!(bounds.expect("first fix remains").center(), expected);
    }
    policy
        .display
        .set_visible(DisplayCategory::QueryHighlights, false);
    assert!(super::query_bounds(policy.scope(), &policy.matches).is_none());
    policy.filter.set_time_window(TimeWindow::Empty);
    assert!(super::visible_bounds(policy.scope(), SkyGlyphVariant::default()).is_none());
}

#[test]
fn query_reveal_uses_the_track_line_tree_gate_with_fix_icons_hidden() {
    let mut policy = Policy::fixture();
    policy.display.solo(DisplayCategory::QueryHighlights);
    policy.matches = test_util::a_run_drawing(test_util::track0(), 0..3);
    policy.visibility.files[0].tracks[0].set_category_visible(DataCategory::Tpv, false);
    assert!(super::query_bounds(policy.scope(), &policy.matches).is_some());
    policy.visibility.files[0].tracks[0].set_category_visible(DataCategory::Track, false);
    assert!(super::query_bounds(policy.scope(), &policy.matches).is_none());
}

#[test]
fn fit_excludes_ghost_label_anchors_when_ghost_fixes_are_hidden() {
    let mut policy = Policy::fixture();
    let time = policy.files[0].tracks[0].points[0].tpv.time().utc();
    let points = [
        (12.0, FixKind::Measured),
        (90.0, FixKind::GhostWithoutSatellitesInFix),
        (13.0, FixKind::Measured),
    ]
    .into_iter()
    .enumerate()
    .map(|(index, (longitude, kind))| {
        fixtures::nav_point(
            time + TimeDelta::seconds(index as i64),
            Latitude::new(55.0),
            Longitude::new(longitude),
            kind,
        )
    })
    .collect();
    let mut track = gt_test_utils::loaded_track_with_points(points);
    track.sat_label_anchors = vec![
        SatLabelAnchor {
            point: PointIdx::new(0),
            tier: SatLabelTier::Endpoint,
        },
        SatLabelAnchor {
            point: PointIdx::new(1),
            tier: SatLabelTier::Fill,
        },
        SatLabelAnchor {
            point: PointIdx::new(2),
            tier: SatLabelTier::Endpoint,
        },
    ];
    let measured_bounds = GeoBounds::from_positions(
        [0, 2]
            .into_iter()
            .filter_map(|index| track.resolved_position_at(index)),
    );
    let all_bounds =
        GeoBounds::from_positions((0..3).filter_map(|index| track.resolved_position_at(index)));
    assert_ne!(measured_bounds, all_bounds);
    policy.files = vec![gt_test_utils::loaded_file_with_tracks(vec![track])];
    policy.visibility = TrackDataVisibility::from_loaded(&policy.files);
    policy.display.solo(DisplayCategory::SatelliteLabels);
    assert_eq!(
        super::visible_bounds(policy.scope(), SkyGlyphVariant::default()),
        measured_bounds
    );
    policy
        .display
        .set_visible(DisplayCategory::GhostFixes, true);
    assert_eq!(
        super::visible_bounds(policy.scope(), SkyGlyphVariant::default()),
        all_bounds
    );
}

#[rstest::rstest]
#[case::ghost_only(DisplayCategory::GhostFixes, false)]
#[case::labels_only(DisplayCategory::SatelliteLabels, false)]
#[case::sky_only(DisplayCategory::SkyGlyphs, false)]
#[case::line_only(DisplayCategory::Tracks, false)]
#[case::icons_only(DisplayCategory::TrackPoints, false)]
#[case::query_and_time_withheld_endpoints(DisplayCategory::GhostFixes, true)]
fn fit_frames_the_endpoint_anchors_selected_by_rendering(
    #[case] category: DisplayCategory,
    #[case] with_withheld_endpoints: bool,
) {
    let mut positions = vec![
        (12.0, FixKind::Measured),
        (13.0, FixKind::GhostWithoutSatellitesInFix),
        (14.0, FixKind::Measured),
    ];
    if with_withheld_endpoints {
        positions.insert(0, (0.0, FixKind::Measured));
        positions.push((100.0, FixKind::Measured));
    }
    let points = positions
        .into_iter()
        .enumerate()
        .map(|(index, (longitude, kind))| {
            fixtures::nav_point(
                test_util::epoch() + TimeDelta::seconds(index as i64),
                Latitude::new(55.0),
                Longitude::new(longitude),
                kind,
            )
        })
        .collect();
    let mut track = gt_test_utils::loaded_track_with_points(points);
    track.sat_label_anchors = vec![SatLabelAnchor {
        point: PointIdx::new(if with_withheld_endpoints { 2 } else { 1 }),
        tier: SatLabelTier::Fill,
    }];
    let files = vec![gt_test_utils::loaded_file_with_tracks(vec![track])];
    let query = with_withheld_endpoints.then(|| QueryMatches {
        hidden: TrackRanges::from_iter([(test_util::track0(), std::iter::once(0..1).collect())]),
        ..QueryMatches::default()
    });
    let map = MapScene::of(files)
        .draw_state(|state| {
            state.display_mask.solo(category);
            state.highlight.fading_enabled = false;
            if with_withheld_endpoints {
                state
                    .filter
                    .set_time_bounds(None, Some(test_util::epoch() + TimeDelta::seconds(3)));
            }
        })
        .overlays(|overlays| overlays.query_matches = query)
        .render();
    let anchors: Vec<_> = map
        .map()
        .expect("rendered map")
        .endpoint_flags
        .anchor_positions()
        .collect();
    assert_eq!(
        anchors,
        [12.0, 14.0].map(|longitude| {
            mercator::normalize(Latitude::new(55.0), Longitude::new(longitude))
        }),
    );
    let framed = map.framed().expect("fitted viewport");
    if with_withheld_endpoints {
        for longitude in [0.0, 100.0] {
            assert!(!(framed.lon_min..=framed.lon_max).contains(&longitude));
        }
    }
    for (latitude, longitude) in anchors.into_iter().map(mercator::denormalize) {
        assert!((framed.lat_min..=framed.lat_max).contains(&latitude));
        assert!(
            (framed.lon_min..=framed.lon_max).contains(&longitude),
            "{category:?}: flag at {longitude} is outside {framed:?}",
        );
    }
}

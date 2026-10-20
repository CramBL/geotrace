use std::path::PathBuf;

use gt_types::coordinates::{Latitude, Longitude};
use gt_types::fixtures::{self, FixKind};
use gt_types::nav_point::NavPoint;
use gt_types::track::{FileSource, TrackGeometry};
use gt_types::{PositionProvenance, SolutionState};
use rstest::rstest;

use crate::segment::{self, FileMeta, FixPlacementRule, SegmentationConfig};
use crate::test_util;

/// A fix the receiver measured: a heading, and satellites in its fix.
fn measured_fix(second: i64, lat: Latitude, lon: Longitude) -> NavPoint {
    fixtures::nav_point(
        test_util::time_at_second(second),
        lat,
        lon,
        FixKind::Measured,
    )
}

/// An epoch the receiver dead-reckoned: no heading, and no satellite report.
fn dead_reckoned_fix(second: i64, lat: Latitude, lon: Longitude) -> NavPoint {
    let mut point = fixtures::nav_point(
        test_util::time_at_second(second),
        lat,
        lon,
        FixKind::GhostWithoutHeading,
    );
    point.navigation.position_provenance = Some(PositionProvenance::SdkSynthesized);
    point
}

/// Where the builder draws each fix of `points`, taken as a track of their
/// own. Empty for a track without a placed fix.
fn drawn_positions(points: &[NavPoint]) -> Vec<(Latitude, Longitude)> {
    segment::measure_track_geometry(points, FixPlacementRule::default())
        .measured()
        .map_or_else(Vec::new, |measured| {
            measured
                .resolved_positions
                .iter()
                .map(|resolved| resolved.coordinates())
                .collect()
        })
}

#[test]
fn a_track_of_no_fixes_has_no_geometry() {
    assert_eq!(
        segment::measure_track_geometry(&[], FixPlacementRule::default()),
        TrackGeometry::NoValidPosition
    );
}

#[test]
fn a_ghost_fix_between_two_anchors_is_interpolated() {
    // Real fixes on the equator at t=0 (lon=0) and t=10 (lon=1), ghost at
    // t=5. The equator is a great circle, so the ghost lands at lon=0.5.
    let points = vec![
        measured_fix(0, Latitude::new(0.0), Longitude::new(0.0)),
        dead_reckoned_fix(5, Latitude::new(10.0), Longitude::new(10.0)),
        measured_fix(10, Latitude::new(0.0), Longitude::new(1.0)),
    ];

    let (latitude, longitude) = drawn_positions(&points)[1];
    assert!(
        latitude.as_degrees().abs() < 1e-9,
        "latitude mismatch: {} vs 0.0",
        latitude.as_degrees(),
    );
    assert!(
        (longitude.as_degrees() - 0.5).abs() < 1e-9,
        "longitude mismatch: {} vs 0.5",
        longitude.as_degrees(),
    );
    assert_eq!(
        points[1].tpv.position(),
        Some((Latitude::new(10.0), Longitude::new(10.0))),
        "the recorded coordinates must survive interpolation"
    );
}

#[test]
fn a_ghost_fix_after_the_last_anchor_snaps_to_it() {
    let points = vec![
        measured_fix(0, Latitude::new(55.0), Longitude::new(12.0)),
        dead_reckoned_fix(10, Latitude::new(10.0), Longitude::new(10.0)),
    ];

    assert_eq!(
        drawn_positions(&points)[1],
        (Latitude::new(55.0), Longitude::new(12.0))
    );
}

/// A fix on the equator with a heading and no satellite report: the receiver
/// reported where it was but not what it tracked.
fn measured_fix_without_a_satellite_report(second: i64, lon_degrees: f64) -> NavPoint {
    test_util::fix_without_a_satellite_report_at(
        second,
        Latitude::new(EQUATORIAL_LATITUDE_DEGREES),
        Longitude::new(lon_degrees),
    )
}

/// A fix the receiver wrote a latitude of NaN for. Its heading is present,
/// leaving the unusable coordinate as the only reason to place it. A
/// position kept as recorded is distinguishable from a placed one: its
/// longitude is far from the fixes around it.
fn fix_without_a_recorded_position(second: i64) -> NavPoint {
    fixtures::nav_point(
        test_util::time_at_second(second),
        Latitude::new(EQUATORIAL_LATITUDE_DEGREES),
        Longitude::new(RECORDED_LONGITUDE_OF_A_FIX_WITHOUT_A_POSITION),
        FixKind::WithoutAPosition,
    )
}

#[rstest]
#[case::between_two_measured_fixes(
    vec![
        measured_fix_without_a_satellite_report(0, 0.0),
        fix_without_a_recorded_position(5),
        measured_fix_without_a_satellite_report(10, 10.0),
    ],
    vec![0.0, 5.0, 10.0]
)]
#[case::before_the_first_measured_fix(
    vec![
        fix_without_a_recorded_position(0),
        measured_fix_without_a_satellite_report(10, 10.0),
        measured_fix_without_a_satellite_report(20, 20.0),
    ],
    vec![10.0, 10.0, 20.0]
)]
#[case::after_the_last_measured_fix(
    vec![
        measured_fix_without_a_satellite_report(0, 0.0),
        measured_fix_without_a_satellite_report(10, 10.0),
        fix_without_a_recorded_position(20),
    ],
    vec![0.0, 10.0, 10.0]
)]
#[case::a_run_of_three_spreads_over_the_time_they_span(
    vec![
        measured_fix_without_a_satellite_report(0, 0.0),
        fix_without_a_recorded_position(2),
        fix_without_a_recorded_position(5),
        fix_without_a_recorded_position(8),
        measured_fix_without_a_satellite_report(10, 10.0),
    ],
    vec![0.0, 2.0, 5.0, 8.0, 10.0]
)]
fn a_fix_without_a_recorded_position_is_placed_from_the_fixes_around_it(
    #[case] points: Vec<NavPoint>,
    #[case] expected_longitudes: Vec<f64>,
) {
    let drawn_longitudes: Vec<f64> = drawn_positions(&points)
        .into_iter()
        .map(|(_, longitude)| longitude.as_degrees())
        .collect();
    assert_eq!(drawn_longitudes.len(), expected_longitudes.len());
    for (index, (drawn, expected)) in drawn_longitudes
        .iter()
        .zip(&expected_longitudes)
        .enumerate()
    {
        assert!(
            (drawn - expected).abs() < PLACEMENT_TOLERANCE_DEGREES,
            "fix {index} drawn at lon {drawn}, expected {expected}"
        );
    }
}

/// A track whose every fix is out of range has no anchor of its own, and
/// the fixes of the recording's other tracks place it: 3610 s is halfway
/// between the fixes at 10 s (lon 10) and 7210 s (lon 20).
#[test]
fn a_track_without_a_position_is_placed_from_the_rest_of_the_recording() {
    let points = vec![
        measured_fix_without_a_satellite_report(0, 0.0),
        measured_fix_without_a_satellite_report(10, 10.0),
        fix_without_a_recorded_position(3610),
        measured_fix_without_a_satellite_report(7210, 20.0),
    ];

    let file = segment::build_loaded_file(
        "out_of_range.gtd".to_owned(),
        &points,
        &[],
        vec![],
        vec![],
        &[],
        &SegmentationConfig::default(),
        FileSource::GtdPath(PathBuf::from("out_of_range.gtd")),
        FileMeta::default(),
        vec![],
    );

    let drawn = file
        .tracks
        .get(1)
        .and_then(|track| track.placed_points()?.get(0))
        .expect("the middle fix is a track of its own");
    let longitude = drawn.resolved_position().1.as_degrees();
    assert!(
        (longitude - 15.0).abs() < PLACEMENT_TOLERANCE_DEGREES,
        "drawn at lon {longitude}, expected 15"
    );
}

/// Placement is read from longitude alone in the tests here: every fix of
/// them sits on the equator. 1e-9° is about 0.1 mm.
const PLACEMENT_TOLERANCE_DEGREES: f64 = 1e-9;

const EQUATORIAL_LATITUDE_DEGREES: f64 = 0.0;

/// Far from the fixes around it, so a position kept as recorded stands out
/// against a placed one.
const RECORDED_LONGITUDE_OF_A_FIX_WITHOUT_A_POSITION: f64 = 88.0;

#[rstest]
#[case::legacy_unknown(None, None, None, FixKind::GhostWithoutHeading, false)]
#[case::source_fix(
    Some(SolutionState::Fix),
    Some(PositionProvenance::SourceProvided),
    Some(8),
    FixKind::GhostWithoutHeading,
    false
)]
#[case::synthesized_with_heading(
    None,
    Some(PositionProvenance::SdkSynthesized),
    Some(8),
    FixKind::Measured,
    true
)]
#[case::headingless_no_fix(
    Some(SolutionState::NoFix),
    Some(PositionProvenance::SourceProvided),
    Some(12),
    FixKind::GhostWithoutHeading,
    true
)]
#[case::explicit_fix_with_zero(
    Some(SolutionState::Fix),
    None,
    Some(0),
    FixKind::GhostWithoutHeading,
    false
)]
#[case::explicit_unknown_with_zero(
    Some(SolutionState::Unknown),
    None,
    Some(0),
    FixKind::GhostWithoutHeading,
    false
)]
#[case::no_fix_with_heading(Some(SolutionState::NoFix), None, Some(12), FixKind::Measured, false)]
#[case::estimated_only(
    Some(SolutionState::EstimatedOnly),
    None,
    None,
    FixKind::GhostWithoutHeading,
    false
)]
#[case::invalid_coordinates(
    Some(SolutionState::Fix),
    Some(PositionProvenance::SourceProvided),
    Some(8),
    FixKind::WithoutAPosition,
    true
)]
fn navigation_placement_preserves_recorded_facts(
    #[case] state: Option<SolutionState>,
    #[case] provenance: Option<PositionProvenance>,
    #[case] used_count: Option<u32>,
    #[case] kind: FixKind,
    #[case] interpolated: bool,
) {
    let mut middle = fixtures::nav_point(
        test_util::time_at_second(5),
        Latitude::new(10.0),
        Longitude::new(10.0),
        kind,
    );
    middle.navigation.state = state;
    middle.navigation.position_provenance = provenance;
    middle.navigation.used_count = used_count;
    let recorded = (middle.tpv.lat(), middle.tpv.lon());
    let points = [
        measured_fix(0, Latitude::new(0.0), Longitude::new(0.0)),
        middle,
        measured_fix(10, Latitude::new(0.0), Longitude::new(1.0)),
    ];
    let geometry = segment::measure_track_geometry(&points, FixPlacementRule::default());
    let resolved = geometry.measured().unwrap().resolved_positions[1];
    assert_eq!(
        matches!(resolved, gt_types::ResolvedPosition::Interpolated(_)),
        interpolated
    );
    let (lat, lon) = resolved.coordinates();
    if interpolated {
        assert!(lat.as_degrees().abs() < PLACEMENT_TOLERANCE_DEGREES);
        assert!((lon.as_degrees() - 0.5).abs() < PLACEMENT_TOLERANCE_DEGREES);
    } else {
        assert_eq!((lat, lon), (Latitude::new(10.0), Longitude::new(10.0)));
    }
    assert_eq!(points[1].tpv.lon(), recorded.1);
    if kind != FixKind::WithoutAPosition {
        assert_eq!(points[1].tpv.lat(), recorded.0);
    } else {
        assert!(points[1].tpv.lat().as_written().is_nan());
    }
    assert_eq!(points[1].navigation.state, state);
    assert_eq!(points[1].navigation.used_count, used_count);
    assert_eq!(
        geometry,
        segment::measure_track_geometry(&points, FixPlacementRule::default())
    );
}

#[rstest]
#[case::unknown_recorded_anchor(None, None, true)]
#[case::explicit_unknown_anchor(Some(SolutionState::Unknown), None, true)]
#[case::no_fix_with_heading_is_not_an_anchor(Some(SolutionState::NoFix), None, false)]
#[case::synthesized_fix_is_not_an_anchor(
    Some(SolutionState::Fix),
    Some(PositionProvenance::SdkSynthesized),
    false
)]
fn navigation_placement_selects_anchors_from_recorded_facts(
    #[case] state: Option<SolutionState>,
    #[case] provenance: Option<PositionProvenance>,
    #[case] middle_is_anchor: bool,
) {
    let mut possible_anchor = measured_fix_without_a_satellite_report(10, 10.0);
    possible_anchor.navigation.state = state;
    possible_anchor.navigation.position_provenance = provenance;
    let mut target = dead_reckoned_fix(15, Latitude::new(10.0), Longitude::new(88.0));
    target.navigation.state = Some(SolutionState::NoFix);
    let points = [
        measured_fix_without_a_satellite_report(0, 0.0),
        possible_anchor,
        target,
        measured_fix_without_a_satellite_report(20, 20.0),
    ];
    let geometry = segment::measure_track_geometry(&points, FixPlacementRule::default());
    let (latitude, longitude) = geometry.measured().unwrap().resolved_positions[2].coordinates();
    assert!(latitude.as_degrees().abs() < PLACEMENT_TOLERANCE_DEGREES);
    assert!((longitude.as_degrees() - 15.0).abs() < PLACEMENT_TOLERANCE_DEGREES);
    let intermediate = geometry.measured().unwrap().resolved_positions[1];
    assert_eq!(
        matches!(intermediate, gt_types::ResolvedPosition::Measured(_)),
        provenance.is_none()
    );
    let mut changed = points;
    changed[1] = measured_fix_without_a_satellite_report(10, 4.0);
    changed[1].navigation.state = state;
    changed[1].navigation.position_provenance = provenance;
    let changed_geometry = segment::measure_track_geometry(&changed, FixPlacementRule::default());
    let changed_lon = changed_geometry.measured().unwrap().resolved_positions[2]
        .coordinates()
        .1
        .as_degrees();
    let expected = if middle_is_anchor { 12.0 } else { 15.0 };
    assert!((changed_lon - expected).abs() < PLACEMENT_TOLERANCE_DEGREES);
}

#[rstest]
#[case::missing_heading(FixPlacementRule::MissingHeading, true)]
#[case::missing_heading_and_no_satellites(FixPlacementRule::MissingHeadingAndNothingInFix, true)]
#[case::navigation_evidence(FixPlacementRule::SynthesizedOrHeadinglessNoFix, false)]
fn legacy_rules_reproduce_headingless_reportless_placement(
    #[case] rule: FixPlacementRule,
    #[case] interpolated: bool,
) {
    let points = [
        measured_fix(0, Latitude::new(0.0), Longitude::new(0.0)),
        fixtures::nav_point(
            test_util::time_at_second(5),
            Latitude::new(10.0),
            Longitude::new(10.0),
            FixKind::GhostWithoutHeading,
        ),
        measured_fix(10, Latitude::new(0.0), Longitude::new(1.0)),
    ];
    let geometry = segment::measure_track_geometry(&points, rule);
    let position = geometry.measured().unwrap().resolved_positions[1];
    assert_eq!(
        matches!(position, gt_types::ResolvedPosition::Interpolated(_)),
        interpolated
    );
    let expected = if interpolated {
        (0.0, 0.5)
    } else {
        (10.0, 10.0)
    };
    let (lat, lon) = position.coordinates();
    assert!((lat.as_degrees() - expected.0).abs() < PLACEMENT_TOLERANCE_DEGREES);
    assert!((lon.as_degrees() - expected.1).abs() < PLACEMENT_TOLERANCE_DEGREES);
}

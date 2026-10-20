//! Where the map draws the fix a receiver measured at the turning point of a
//! cul-de-sac, with a full solution behind it and no course to report.
//!
//! The recording runs east up the street, turns at the far end, and comes back
//! west on the other side. The fix at the turn is the tip of the track. It
//! holds twelve satellites in fix and no heading: the receiver stands still
//! there.

use std::path::PathBuf;

use chrono::{DateTime, Duration, Utc};
use gt_map::test_util::{self, MapScene};
use gt_track_builder::{FileMeta, SegmentationConfig};
use gt_types::fixtures::FixKind;
use gt_types::{
    FileSource, FixRef, Latitude, LoadedFile, Longitude, MapFixKind, NavPoint, NavigationMetadata,
    PointIdx, PositionProvenance, SatelliteEvidence, SatelliteQueryScope, SolutionState, mercator,
};
use gt_ui_types::{DisplayCategory, MapElementRef};
use rstest::rstest;
use uom::si::angle::degree;
use uom::si::f64::Angle;

fn time_of(index: usize) -> DateTime<Utc> {
    test_util::epoch() + Duration::seconds(index as i64 * SECONDS_BETWEEN_FIXES)
}

/// The longitude of the fix at `step` steps up the street.
fn longitude_of(step: usize) -> Longitude {
    Longitude::new(FIRST_LONGITUDE_DEGREES + step as f64 * LONGITUDE_STEP_DEGREES)
}

/// A fix under a full solution, twelve satellites in fix, on the course the
/// receiver reported for it.
fn fix(index: usize, lat: Latitude, lon: Longitude, heading: Option<Angle>) -> NavPoint {
    gt_types::fixtures::nav_point_heading(time_of(index), lat, lon, heading, FixKind::Measured)
}

/// The position of the fix at the turn, where the map draws the tip of the
/// track.
fn turn_position() -> (Latitude, Longitude) {
    (
        Latitude::new(EASTBOUND_LATITUDE_DEGREES.midpoint(WESTBOUND_LATITUDE_DEGREES)),
        longitude_of(FIXES_PER_LEG),
    )
}

#[derive(Clone, Copy)]
enum TurnEvidence {
    LegacyReport,
    Unreported,
    Summary,
    SummaryWithObservations,
}

fn a_recording_that_turns_at_the_end_of_a_cul_de_sac(evidence: TurnEvidence) -> Vec<LoadedFile> {
    let eastbound = (0..FIXES_PER_LEG).map(|step| {
        fix(
            step,
            Latitude::new(EASTBOUND_LATITUDE_DEGREES),
            longitude_of(step),
            Some(Angle::new::<degree>(EAST_HEADING_DEGREES)),
        )
    });
    let (turn_lat, turn_lon) = turn_position();
    let mut turn = fix(TURN_FIX_INDEX, turn_lat, turn_lon, None);
    if !matches!(evidence, TurnEvidence::LegacyReport) {
        turn.satellites = None;
        if !matches!(evidence, TurnEvidence::SummaryWithObservations) {
            turn.satellite_evidence = SatelliteEvidence::default();
        } else {
            turn.satellite_evidence.usage.clear();
        }
    }
    if matches!(
        evidence,
        TurnEvidence::Summary | TurnEvidence::SummaryWithObservations
    ) {
        turn.navigation = NavigationMetadata {
            state: Some(SolutionState::Fix),
            used_count: Some(8),
            hdop: Some(0.9),
            position_provenance: Some(PositionProvenance::SourceProvided),
            ..NavigationMetadata::default()
        };
    }
    let turn = std::iter::once(turn);
    let westbound = (0..FIXES_PER_LEG).map(|step| {
        fix(
            TURN_FIX_INDEX + 1 + step,
            Latitude::new(WESTBOUND_LATITUDE_DEGREES),
            longitude_of(FIXES_PER_LEG - 1 - step),
            Some(Angle::new::<degree>(WEST_HEADING_DEGREES)),
        )
    });

    let points: Vec<NavPoint> = eastbound.chain(turn).chain(westbound).collect();
    vec![gt_track_builder::build_loaded_file(
        "cul_de_sac.gtd".to_owned(),
        &points,
        &[],
        vec![],
        vec![],
        &[],
        &SegmentationConfig::default(),
        FileSource::GtdPath(PathBuf::from("cul_de_sac.gtd")),
        FileMeta::default(),
        vec![],
    )]
}

#[test]
fn snapshot_a_fix_measured_without_a_heading_is_drawn_at_the_tip_of_the_track() {
    let files = a_recording_that_turns_at_the_end_of_a_cul_de_sac(TurnEvidence::LegacyReport);
    let (turn_lat, turn_lon) = turn_position();
    assert_eq!(
        test_util::drawn_positions(&files).get(TURN_FIX_INDEX),
        Some(&mercator::normalize(turn_lat, turn_lon)),
        "the fix at the turn is drawn where the receiver measured it"
    );

    let mut map = MapScene::of(files).render();
    map.snapshot("fix_measured_without_a_heading");
}

#[rstest]
#[case::legacy_without_detail(TurnEvidence::Unreported, "headingless_without_detail")]
#[case::source_summary(TurnEvidence::Summary, "headingless_with_summary")]
#[case::source_summary_with_observations(
    TurnEvidence::SummaryWithObservations,
    "headingless_with_observations"
)]
fn snapshot_headingless_positions_remain_selectable_at_the_recorded_turn(
    #[case] evidence: TurnEvidence,
    #[case] snapshot: &str,
    #[values(false, true)] dark: bool,
) {
    let files = a_recording_that_turns_at_the_end_of_a_cul_de_sac(evidence);
    let point = &files[0].tracks[0].points[TURN_FIX_INDEX];
    assert_eq!(point.tpv.position(), Some(turn_position()));
    assert_eq!(point.map_fix_kind(), MapFixKind::Recorded);
    assert_eq!(point.tpv.heading(), None);
    assert_eq!(
        test_util::drawn_positions(&files)[TURN_FIX_INDEX],
        mercator::normalize(turn_position().0, turn_position().1)
    );
    if matches!(
        evidence,
        TurnEvidence::Summary | TurnEvidence::SummaryWithObservations
    ) {
        assert_eq!(point.resolved_navigation_state().state, SolutionState::Fix);
        assert_eq!(
            point
                .resolved_used_count(SatelliteQueryScope::Global)
                .map(|count| count.count),
            Some(8)
        );
        assert_eq!(
            point
                .resolved_seen_count(SatelliteQueryScope::Global)
                .map(|count| count.count),
            matches!(evidence, TurnEvidence::SummaryWithObservations).then_some(12)
        );
    }
    let reference = MapElementRef::Fix(FixRef::new(
        test_util::track0(),
        PointIdx::new(TURN_FIX_INDEX),
    ));
    let mut map = MapScene::of(files)
        .theme(dark)
        .centred_on_fix(TURN_FIX_INDEX)
        .zoomed_to(15.0)
        .render();
    map.snapshot(&format!(
        "{snapshot}_{}",
        if dark { "dark" } else { "light" }
    ));
    assert_eq!(
        map.primary_hover_candidate_at(test_util::viewport_center()),
        Some(reference)
    );
    assert!(map.pin(reference));
    map.draw_state()
        .display_mask
        .solo(DisplayCategory::GhostFixes);
    assert_eq!(
        map.primary_hover_candidate_at(test_util::viewport_center()),
        None
    );
    map.draw_state()
        .display_mask
        .solo(DisplayCategory::TrackPoints);
    assert_eq!(
        map.primary_hover_candidate_at(test_util::viewport_center()),
        Some(reference)
    );
}

const SECONDS_BETWEEN_FIXES: i64 = 30;

/// Fixes of each leg, at [`LONGITUDE_STEP_DEGREES`] apart.
const FIXES_PER_LEG: usize = 6;

/// The fix at the turning point, between the two legs.
const TURN_FIX_INDEX: usize = FIXES_PER_LEG;

const FIRST_LONGITUDE_DEGREES: f64 = 12.550;

/// Longitude between consecutive fixes of a leg, about 420 m at these
/// latitudes. The six steps of a leg fill four fifths of the viewport once the
/// map frames the recording.
const LONGITUDE_STEP_DEGREES: f64 = 0.006_69;

/// The latitude of the leg running east, and of the leg running west. They are
/// about 222 m apart, which is a tenth of the viewport.
const EASTBOUND_LATITUDE_DEGREES: f64 = 55.674;
const WESTBOUND_LATITUDE_DEGREES: f64 = 55.676;

const EAST_HEADING_DEGREES: f64 = 90.0;
const WEST_HEADING_DEGREES: f64 = 270.0;

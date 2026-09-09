use gt_types::coordinates::{Latitude, Longitude};
use uom::si::f64::Length;
use uom::si::length::kilometer;

use crate::segment::{self, FixPlacementRule};
use crate::test_util;

#[test]
fn compute_track_metadata_basic() {
    let pts = vec1::vec1![
        test_util::fix_without_a_satellite_report_at(0, Latitude::new(55.0), Longitude::new(12.0)),
        // 1 h later, ~13 km away
        test_util::fix_without_a_satellite_report_at(
            3600,
            Latitude::new(55.1),
            Longitude::new(12.1)
        ),
    ];
    let meta = segment::compute_track_metadata(1, &pts, &[], &[]);
    assert_eq!(meta.index, 1);
    assert_eq!(meta.tpv_count, 2);
    assert_eq!(meta.duration.num_seconds(), 3600);
    assert!(!meta.has_custom_markers);
    assert_eq!(meta.satellite_report_count, 0);

    let distance_km = segment::measure_track_geometry(&pts, FixPlacementRule::default())
        .measured()
        .expect("both fixes have a recorded position")
        .distance_km;
    assert!(
        distance_km > Length::new::<kilometer>(5.0),
        "expected > 5 km, got {distance_km:?}"
    );
}

#[test]
fn compute_track_metadata_single_point_has_zero_duration() {
    let pts = vec1::vec1![test_util::fix_without_a_satellite_report_at(
        0,
        Latitude::new(55.0),
        Longitude::new(12.0)
    )];
    let meta = segment::compute_track_metadata(1, &pts, &[], &[]);
    assert_eq!(meta.duration.num_seconds(), 0);

    let distance_km = segment::measure_track_geometry(&pts, FixPlacementRule::default())
        .measured()
        .expect("the fix has a recorded position")
        .distance_km;
    assert_eq!(distance_km, Length::new::<kilometer>(0.0));
}

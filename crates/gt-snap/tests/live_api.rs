//! Live smoke test against the real Valhalla server: the on-demand drift
//! check for the API boundary.
//!
//! Excluded from the default nextest profile (see `.config/nextest.toml`,
//! same mechanism as gt-arch) and run via `just snap-live-test` - never in
//! CI. Asserts structural invariants, not values: matching output drifts
//! with OSM data, but the shape of the exchange must hold.
//!
//! The `live` nextest test group caps this binary at one thread so a run
//! can never exceed the server's 1 request/s fair-use budget (the transport
//! also paces itself).

mod support;

use support::base_time;

use gt_snap::DEFAULT_SERVER_URL;
use gt_snap::request_plan::{self, SnapParams};
use gt_snap::stitch::{self, ChunkOutcome, SnapWarningReporter};
use gt_snap::transport::{self, HttpTransport};
use gt_snap::wire::Costing;
use gt_types::nav_point::NavPoint;
use gt_types::time_types::GpsTime;
use gt_types::tpv::TimePositionVelocity;
use gt_types::{Latitude, Longitude};

/// A 30-point 1 Hz trace along H.C. Andersens Boulevard, Copenhagen (the
/// capture harness's clean-drive reference street).
fn boulevard_points() -> Vec<NavPoint> {
    // Two anchors of the /route-sampled boulevard geometry.
    let (from, to) = ((55.678_74, 12.564_49), (55.674_34, 12.570_53));
    let count = 30;
    (0..count)
        .map(|i| {
            let t = i as f64 / (count - 1) as f64;
            let time = base_time() + chrono::Duration::seconds(i as i64);
            let tpv = TimePositionVelocity::builder()
                .time(GpsTime::from_utc(time))
                .lat(Latitude::new(from.0 + (to.0 - from.0) * t))
                .lon(Longitude::new(from.1 + (to.1 - from.1) * t))
                .build();
            NavPoint::new(tpv, None)
        })
        .collect()
}

/// Full pipeline against the live server: plan, send, stitch. Structural
/// assertions only.
#[test]
fn full_pipeline_against_live_server() {
    let points = boulevard_points();
    let plan = request_plan::plan(&points);
    assert_eq!(plan.chunks.len(), 1);

    let transport = HttpTransport::new(DEFAULT_SERVER_URL).expect("transport builds");
    let outcomes = transport::send_plan(
        &transport,
        &plan,
        &SnapParams::new(Costing::Auto),
        |_, _| {},
    );
    assert_eq!(outcomes.len(), 1);
    assert!(
        matches!(outcomes.first(), Some(ChunkOutcome::Success(_))),
        "a clean on-street trace must match; got {outcomes:?}"
    );

    let reporter = SnapWarningReporter::default();
    let result = stitch::stitch(&plan, SnapParams::new(Costing::Auto), &outcomes, &reporter);

    // Structural invariants, not values.
    assert_eq!(result.points.len(), plan.sent_point_count());
    assert!(!result.partial);
    assert_eq!(result.kind_counts.total(), result.points.len());
    assert!(
        result.kind_counts.snapped + result.kind_counts.interpolated > 0,
        "an on-street trace yields snappable points"
    );
    assert!(
        result
            .points
            .iter()
            .filter(|p| p.kind != gt_snap::wire::SnapPointKind::Unsnapped)
            .all(|p| p.error_m.is_some()),
        "every snappable point carries a snap error"
    );
    assert!(
        !result.segments.is_empty(),
        "the snapped track has geometry"
    );
    assert!(result.osm_changeset.is_some(), "map data version present");
    assert!(result.confidence_score.is_some(), "confidence present");
}

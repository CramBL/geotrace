//! Pin the persisted form of a snap run.
//!
//! [`SnapResult`], [`SnapParams`], and [`SnapWarning`] serialize into the
//! recording history database's snap blob. Their JSON shape is a storage
//! schema: any drift breaks decoding of previously stored runs, so it must
//! show up as a reviewed snapshot diff, never as an incidental change.

use serde_json::json;
use strum::EnumCount;

use gt_snap::request_plan::SnapParams;
use gt_snap::snapped_track::{Position, SnappedEdgeSpan, SnappedTrackSegment};
use gt_snap::stitch::{SnapKindCounts, SnapPoint, SnapResult, SnapWarning};
use gt_snap::wire::{Costing, Edge, RoadClass, SnapPointKind, SpeedLimit, Surface};

use gt_types::PointIdx;

/// A result with every field populated and one point per kind, so the
/// snapshot pins the serialized spelling of the full schema.
fn exhaustive_result() -> SnapResult {
    SnapResult {
        points: vec![
            SnapPoint {
                point: PointIdx::new(0),
                kind: SnapPointKind::Snapped,
                error_m: Some(3.25),
                snapped: Some(Position {
                    lat: 55.6787,
                    lon: 12.5645,
                }),
                edge: Some(0),
                follows_gap: true,
            },
            SnapPoint {
                point: PointIdx::new(4),
                kind: SnapPointKind::Interpolated,
                error_m: Some(1.5),
                snapped: Some(Position {
                    lat: 55.6779,
                    lon: 12.5654,
                }),
                edge: Some(1),
                follows_gap: false,
            },
            SnapPoint {
                point: PointIdx::new(9),
                kind: SnapPointKind::Unsnapped,
                error_m: None,
                snapped: None,
                edge: None,
                follows_gap: false,
            },
        ],
        segments: vec![SnappedTrackSegment {
            positions: vec![
                Position {
                    lat: 55.6787,
                    lon: 12.5645,
                },
                Position {
                    lat: 55.6779,
                    lon: 12.5654,
                },
            ],
            edge_spans: vec![SnappedEdgeSpan {
                start: 0,
                end: 2,
                edge: 0,
            }],
        }],
        edges: vec![
            Edge {
                names: vec!["H.C. Andersens Boulevard".to_owned()],
                way_id: Some(496_181_694),
                road_class: Some(RoadClass::Tertiary),
                speed_limit: Some(SpeedLimit::Kmh(50)),
                surface: Some(Surface::PavedSmooth),
                begin_shape_index: Some(0),
                end_shape_index: Some(1),
            },
            Edge {
                names: Vec::new(),
                way_id: None,
                road_class: Some(RoadClass::Unknown),
                speed_limit: None,
                surface: Some(Surface::Unknown),
                begin_shape_index: None,
                end_shape_index: None,
            },
        ],
        kind_counts: SnapKindCounts {
            snapped: 1,
            interpolated: 1,
            unsnapped: 1,
        },
        confidence_score: Some(0.87),
        osm_changeset: Some(1_783_810_896),
        params: SnapParams {
            costing: Costing::Bicycle,
            search_radius_m: Some(25.0),
            turn_penalty_factor: Some(300.0),
            gps_accuracy_override_m: Some(10.0),
        },
        gps_accuracy_sent_m: Some(10.0),
        partial: true,
    }
}

/// One warning per variant, so the snapshot pins every tag and payload
/// spelling.
fn exhaustive_warnings() -> Vec<SnapWarning> {
    vec![
        SnapWarning::ChunkFailed {
            chunk_index: 2,
            detail: "HTTP 502 Bad Gateway".to_owned(),
        },
        SnapWarning::PointCountMismatch {
            chunk_index: 3,
            sent: 1000,
            received: 998,
        },
        SnapWarning::Geometry {
            chunk_index: 4,
            detail: "edge 7 carries no shape index range".to_owned(),
        },
        SnapWarning::OsmChangesetMismatch {
            first: 185_397_177,
            later: 1_783_810_896,
        },
        SnapWarning::Server {
            chunk_index: 5,
            warnings: vec![json!({"code": 1, "text": "synthetic"})],
        },
    ]
}

/// The persisted schema, pinned: any diff here is a storage format change
/// and needs a deliberate compatibility decision (older blobs must keep
/// decoding).
#[test]
fn snapshot_stored_result_schema() {
    // The fixture must stay exhaustive: a new variant without a pinned
    // stored spelling fails here instead of silently under-covering.
    assert_eq!(exhaustive_warnings().len(), SnapWarning::COUNT);
    let stored = json!({
        "result": exhaustive_result(),
        "warnings": exhaustive_warnings(),
    });
    let pretty = serde_json::to_string_pretty(&stored).expect("serialize");
    insta::assert_snapshot!("stored_result_schema", pretty);
}

/// Serializing and re-decoding a fully populated run is lossless.
#[test]
fn stored_result_roundtrips() {
    let result = exhaustive_result();
    let json = serde_json::to_string(&result).expect("serialize");
    let decoded: SnapResult = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(decoded, result);

    let warnings = exhaustive_warnings();
    let json = serde_json::to_string(&warnings).expect("serialize");
    let decoded: Vec<SnapWarning> = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(decoded, warnings);
}

/// Fields added after a result was stored decode as their defaults: a blob
/// carrying only the required fields (the forward-compatibility contract
/// `#[serde(default)]` establishes) still decodes.
#[test]
fn minimal_stored_result_decodes_with_defaults() {
    let minimal = json!({
        "params": { "costing": "auto" },
    });
    let decoded: SnapResult = serde_json::from_value(minimal).expect("minimal blob decodes");
    assert_eq!(decoded.points, Vec::new());
    assert_eq!(decoded.segments, Vec::new());
    assert_eq!(decoded.edges, Vec::new());
    assert_eq!(decoded.kind_counts, SnapKindCounts::default());
    assert_eq!(decoded.confidence_score, None);
    assert_eq!(decoded.osm_changeset, None);
    assert_eq!(decoded.params, SnapParams::new(Costing::Auto));
    assert_eq!(decoded.gps_accuracy_sent_m, None);
    assert!(!decoded.partial);
}

/// The open-world edge enums roundtrip their `Unknown` catch-all: it
/// serializes as `"unknown"`, which the `#[serde(other)]` arm folds back
/// into `Unknown` - a stored result never fails on an unmodeled class.
#[test]
fn unknown_edge_attribute_variants_roundtrip() {
    let road: RoadClass =
        serde_json::from_value(serde_json::to_value(RoadClass::Unknown).expect("serialize"))
            .expect("deserialize");
    assert_eq!(road, RoadClass::Unknown);
    let surface: Surface =
        serde_json::from_value(serde_json::to_value(Surface::Unknown).expect("serialize"))
            .expect("deserialize");
    assert_eq!(surface, Surface::Unknown);
}

//! Validate the typed wire format against the live-captured fixtures.
//!
//! Every success and error fixture must parse into the typed structs, and
//! every well-formed captured request must roundtrip through
//! [`TraceAttributesRequest`] byte-for-byte (as JSON values) - proving the
//! types model exactly what the capture harness sent and the server returned.

use std::collections::BTreeSet;
use std::fs;

use serde::Deserialize;
use serde::de::IntoDeserializer;
use serde::de::value::{Error as DeError, StrDeserializer};
use serde_json::{Value, json};
use strum::EnumCount;

use gt_snap::wire::{
    Costing, ErrorCode, ErrorResponse, FilterAction, RoadClass, SnapPointKind, SpeedLimit, Surface,
    TraceAttributesRequest, TraceAttributesResponse, TraceOptions,
};
use gt_snap::{DEFAULT_SERVER_URL, FIXTURE_SCENARIOS, fixtures_dir, server_host};

/// The fixture scenarios whose response is a successful match.
const SUCCESS_SCENARIOS: &[&str] = &[
    "clean_drive",
    "clean_drive_tuned",
    "clean_drive_unfiltered",
    "dense_10hz",
    "partially_snappable",
    "teleport_gap",
];

/// The fixture scenarios whose response is a Valhalla JSON error.
const ERROR_SCENARIOS: &[&str] = &[
    "bad_request",
    "option_out_of_bounds",
    "oversized",
    "unsnappable",
];

/// The fixture scenarios whose response is not JSON at all (rejected by the
/// reverse proxy before Valhalla sees them).
const HTML_ERROR_SCENARIOS: &[&str] = &["too_large_body"];

/// Every fixture scenario must be classified into exactly one of the three
/// lists above, so adding a scenario to [`FIXTURE_SCENARIOS`] without
/// classifying (and thereby parsing) it here fails loudly - the same
/// discipline `EnumCount` applies to the wire-name tables.
#[test]
fn every_scenario_is_classified_exactly_once() {
    let mut classified: Vec<&str> = SUCCESS_SCENARIOS
        .iter()
        .chain(ERROR_SCENARIOS)
        .chain(HTML_ERROR_SCENARIOS)
        .copied()
        .collect();
    classified.sort_unstable();
    let mut expected: Vec<&str> = FIXTURE_SCENARIOS.to_vec();
    expected.sort_unstable();
    assert_eq!(classified, expected);
}

fn read_fixture(name: &str) -> Result<String, String> {
    let path = fixtures_dir().join(name);
    fs::read_to_string(&path).map_err(|err| format!("reading {}: {err}", path.display()))
}

/// A compact, order-stable digest of a parsed response, sized for snapshot
/// review (full responses run to hundreds of points).
#[derive(Debug)]
#[expect(dead_code, reason = "fields exist to be rendered by Debug snapshots")]
struct ResponseDigest {
    points: usize,
    snapped: usize,
    interpolated: usize,
    unsnapped: usize,
    discontinuity_indices: Vec<usize>,
    first_point: Option<String>,
    edges: usize,
    road_classes: BTreeSet<String>,
    surfaces: BTreeSet<String>,
    speed_limits: BTreeSet<String>,
    shape_chars: usize,
    osm_changeset: Option<u64>,
    confidence_score: Option<f64>,
    warnings: usize,
}

impl ResponseDigest {
    fn of(response: &TraceAttributesResponse) -> Self {
        let count = |kind| {
            response
                .snapped_points
                .iter()
                .filter(|p| p.kind == kind)
                .count()
        };
        Self {
            points: response.snapped_points.len(),
            snapped: count(SnapPointKind::Snapped),
            interpolated: count(SnapPointKind::Interpolated),
            unsnapped: count(SnapPointKind::Unsnapped),
            discontinuity_indices: response
                .snapped_points
                .iter()
                .enumerate()
                .filter(|(_, p)| p.begin_route_discontinuity || p.end_route_discontinuity)
                .map(|(i, _)| i)
                .collect(),
            first_point: response.snapped_points.first().map(|p| format!("{p:?}")),
            edges: response.edges.len(),
            road_classes: response
                .edges
                .iter()
                .filter_map(|e| e.road_class.map(|c| format!("{c:?}")))
                .collect(),
            surfaces: response
                .edges
                .iter()
                .filter_map(|e| e.surface.map(|s| format!("{s:?}")))
                .collect(),
            speed_limits: response
                .edges
                .iter()
                .filter_map(|e| e.speed_limit.map(SpeedLimit::display))
                .collect(),
            shape_chars: response.shape.as_deref().map_or(0, str::len),
            osm_changeset: response.osm_changeset,
            confidence_score: response.confidence_score,
            warnings: response.warnings.len(),
        }
    }
}

#[test]
fn success_fixtures_parse() {
    for &scenario in SUCCESS_SCENARIOS {
        let body = read_fixture(&format!("{scenario}.response.json")).expect("fixture");
        let response: TraceAttributesResponse =
            serde_json::from_str(&body).expect("success fixture must parse");
        insta::assert_debug_snapshot!(scenario, ResponseDigest::of(&response));
    }
}

#[test]
fn error_fixtures_parse() {
    let digests: Vec<(String, ErrorResponse)> = ERROR_SCENARIOS
        .iter()
        .map(|&scenario| {
            let body = read_fixture(&format!("{scenario}.response.json")).expect("fixture");
            let error: ErrorResponse =
                serde_json::from_str(&body).expect("error fixture must parse");
            (scenario.to_owned(), error)
        })
        .collect();
    insta::assert_debug_snapshot!(digests);
}

#[test]
fn proxy_html_error_parses_as_neither_type() {
    for &scenario in HTML_ERROR_SCENARIOS {
        let body = read_fixture(&format!("{scenario}.response.json")).expect("fixture");
        serde_json::from_str::<TraceAttributesResponse>(&body)
            .expect_err("the proxy's HTML error page must not parse as a success response");
        serde_json::from_str::<ErrorResponse>(&body)
            .expect_err("the proxy's HTML error page must not parse as a Valhalla error");
    }
}

/// Every well-formed captured request roundtrips through the typed request:
/// parse, re-serialize, compare as JSON values. Proves the type models every
/// field the capture harness sends (it builds requests from these types, so
/// drift in either direction fails here).
#[test]
fn captured_requests_roundtrip_through_typed_request() {
    for &scenario in FIXTURE_SCENARIOS.iter().filter(|&&s| s != "bad_request") {
        let body = read_fixture(&format!("{scenario}.request.json")).expect("fixture");
        let original: Value = serde_json::from_str(&body).expect("fixture JSON");
        let typed: TraceAttributesRequest =
            serde_json::from_value(original.clone()).expect("typed parse");
        let reserialized = serde_json::to_value(&typed).expect("re-serialize");
        assert_eq!(original, reserialized, "{scenario} request drifted");
    }
}

/// The deliberately malformed request (no shape) must NOT parse: `shape` is
/// mandatory on the typed request.
#[test]
fn bad_request_fixture_is_not_a_valid_typed_request() {
    let body = read_fixture("bad_request.request.json").expect("fixture");
    serde_json::from_str::<TraceAttributesRequest>(&body)
        .expect_err("a request without a shape must not be expressible");
}

/// The `trace_options` payload shape: each present option serializes under
/// Valhalla's field name, absent options serialize to nothing (which is why
/// captured fixture requests without `trace_options` still roundtrip
/// unchanged).
#[test]
fn trace_options_serialize_only_present_options() {
    let all = TraceOptions {
        gps_accuracy: Some(12.5),
        search_radius: Some(25.0),
        turn_penalty_factor: Some(300.0),
    };
    assert_eq!(
        serde_json::to_value(all).expect("serialize"),
        json!({
            "gps_accuracy": 12.5,
            "search_radius": 25.0,
            "turn_penalty_factor": 300.0,
        })
    );
    let none = TraceOptions {
        gps_accuracy: None,
        search_radius: None,
        turn_penalty_factor: None,
    };
    assert_eq!(serde_json::to_value(none).expect("serialize"), json!({}));
}

/// No live exemplar of a `warnings` array exists (the server rejects an
/// out-of-range option), so its parsing is pinned synthetically.
#[test]
fn warnings_array_is_preserved_raw() {
    let response: TraceAttributesResponse = serde_json::from_str(
        r#"{"matched_points": [], "warnings": [{"level": "warn", "message": "synthetic"}]}"#,
    )
    .expect("synthetic warnings body");
    assert_eq!(response.warnings.len(), 1);
    assert_eq!(response.warnings[0]["message"], "synthetic");
}

/// Locks the wire spelling of every closed `enum`, exhaustively via
/// `EnumCount` (see `gt_types::metrics::tests::wire_names_are_stable`).
#[test]
fn wire_names_are_stable() {
    let kinds = [
        (SnapPointKind::Snapped, "matched"),
        (SnapPointKind::Interpolated, "interpolated"),
        (SnapPointKind::Unsnapped, "unmatched"),
    ];
    assert_eq!(kinds.len(), SnapPointKind::COUNT);
    for (kind, wire) in kinds {
        let de: StrDeserializer<'_, DeError> = wire.into_deserializer();
        assert_eq!(SnapPointKind::deserialize(de), Ok(kind), "{wire:?}");
        assert_eq!(kind.to_string(), wire);
    }

    let costings = [
        (Costing::Auto, "auto"),
        (Costing::Bicycle, "bicycle"),
        (Costing::Pedestrian, "pedestrian"),
    ];
    assert_eq!(costings.len(), Costing::COUNT);
    for (costing, wire) in costings {
        let de: StrDeserializer<'_, DeError> = wire.into_deserializer();
        assert_eq!(Costing::deserialize(de), Ok(costing), "{wire:?}");
        assert_eq!(costing.to_string(), wire);
    }

    let actions = [
        (FilterAction::Include, "include"),
        (FilterAction::Exclude, "exclude"),
    ];
    assert_eq!(actions.len(), FilterAction::COUNT);
    for (action, wire) in actions {
        let de: StrDeserializer<'_, DeError> = wire.into_deserializer();
        assert_eq!(FilterAction::deserialize(de), Ok(action), "{wire:?}");
        assert_eq!(action.to_string(), wire);
    }
}

/// Pin the costing display spellings so a variant rename cannot silently
/// change the settings combo. The table length is asserted against
/// `EnumCount` so a new variant cannot be forgotten here.
#[test]
fn costing_display_name_is_canonical_spelling() {
    let expected = [
        (Costing::Auto, "Auto"),
        (Costing::Bicycle, "Bicycle"),
        (Costing::Pedestrian, "Pedestrian"),
    ];
    assert_eq!(expected.len(), Costing::COUNT);
    for (costing, name) in expected {
        assert_eq!(costing.display_name(), name);
    }
}

/// Pins the UI spelling of every road class shown on snapped-track hover,
/// exhaustively like [`costing_display_name_is_canonical_spelling`].
#[test]
fn road_class_display_name_is_canonical_spelling() {
    let expected = [
        (RoadClass::Motorway, "Motorway"),
        (RoadClass::Trunk, "Trunk"),
        (RoadClass::Primary, "Primary"),
        (RoadClass::Secondary, "Secondary"),
        (RoadClass::Tertiary, "Tertiary"),
        (RoadClass::Unclassified, "Unclassified"),
        (RoadClass::Residential, "Residential"),
        (RoadClass::ServiceOther, "Service or other"),
        (RoadClass::Unknown, "Unknown"),
    ];
    assert_eq!(expected.len(), RoadClass::COUNT);
    for (road_class, name) in expected {
        assert_eq!(road_class.display_name(), name);
    }
}

/// Pins the UI spelling of every surface shown on snapped-track hover.
#[test]
fn surface_display_name_is_canonical_spelling() {
    let expected = [
        (Surface::PavedSmooth, "Paved smooth"),
        (Surface::Paved, "Paved"),
        (Surface::PavedRough, "Paved rough"),
        (Surface::Compacted, "Compacted"),
        (Surface::Dirt, "Dirt"),
        (Surface::Gravel, "Gravel"),
        (Surface::Path, "Path"),
        (Surface::Impassable, "Impassable"),
        (Surface::Unknown, "Unknown"),
    ];
    assert_eq!(expected.len(), Surface::COUNT);
    for (surface, name) in expected {
        assert_eq!(surface.display_name(), name);
    }
}

/// `server_host` is the granularity of the app's upload-consent bookkeeping:
/// scheme, port, and path changes keep consent, a host change re-prompts, and
/// URLs without a parsable host never count as consented.
#[test]
fn server_host_extracts_the_host_and_only_the_host() {
    assert_eq!(
        server_host(DEFAULT_SERVER_URL).as_deref(),
        Some("valhalla1.openstreetmap.de")
    );
    assert_eq!(
        server_host("http://localhost:8002/some/path").as_deref(),
        Some("localhost")
    );
    assert_eq!(server_host("not a url"), None);
    assert_eq!(server_host(""), None);
    // A host-less URL must not count as a host either.
    assert_eq!(server_host("file:///tmp/x"), None);
}

/// Open-world `enum` types: known wire names parse to their variant, anything
/// else lands on `Unknown` and the response still parses.
#[test]
fn open_enums_absorb_unknown_wire_values() {
    let road_classes = [
        (RoadClass::Motorway, "motorway"),
        (RoadClass::Trunk, "trunk"),
        (RoadClass::Primary, "primary"),
        (RoadClass::Secondary, "secondary"),
        (RoadClass::Tertiary, "tertiary"),
        (RoadClass::Unclassified, "unclassified"),
        (RoadClass::Residential, "residential"),
        (RoadClass::ServiceOther, "service_other"),
        (RoadClass::Unknown, "some_future_class"),
    ];
    assert_eq!(road_classes.len(), RoadClass::COUNT);
    for (class, wire) in road_classes {
        let de: StrDeserializer<'_, DeError> = wire.into_deserializer();
        assert_eq!(RoadClass::deserialize(de), Ok(class), "{wire:?}");
    }

    let surfaces = [
        (Surface::PavedSmooth, "paved_smooth"),
        (Surface::Paved, "paved"),
        (Surface::PavedRough, "paved_rough"),
        (Surface::Compacted, "compacted"),
        (Surface::Dirt, "dirt"),
        (Surface::Gravel, "gravel"),
        (Surface::Path, "path"),
        (Surface::Impassable, "impassable"),
        (Surface::Unknown, "some_future_surface"),
    ];
    assert_eq!(surfaces.len(), Surface::COUNT);
    for (surface, wire) in surfaces {
        let de: StrDeserializer<'_, DeError> = wire.into_deserializer();
        assert_eq!(Surface::deserialize(de), Ok(surface), "{wire:?}");
    }
}

/// The wire's "no edge association" sentinel value (`u64::MAX`, captured on
/// interpolated points in `dense_10hz`) folds into `None` and never escapes
/// the wire layer.
#[test]
fn edge_index_sentinel_folds_into_none() {
    let response: TraceAttributesResponse = serde_json::from_str(
        r#"{"matched_points": [
            {"lat": 55.0, "lon": 12.0, "type": "interpolated", "edge_index": 18446744073709551615},
            {"lat": 55.0, "lon": 12.0, "type": "matched", "edge_index": 3}
        ]}"#,
    )
    .expect("synthetic body");
    assert_eq!(response.snapped_points[0].edge_index, None);
    assert_eq!(response.snapped_points[1].edge_index, Some(3));
}

/// Valhalla reports derestricted roads (autobahn stretches) as the string
/// `"unlimited"` where a km/h number normally sits. Both wire shapes parse
/// and serialize back unchanged so cached results round-trip. Any other
/// string is an error.
#[rstest::rstest]
#[case::kmh("50", SpeedLimit::Kmh(50), "50 km/h")]
#[case::unlimited(r#""unlimited""#, SpeedLimit::Unlimited, "Unlimited")]
fn speed_limit_parses_both_wire_shapes(
    #[case] json: &str,
    #[case] expected: SpeedLimit,
    #[case] display: &str,
) {
    let parsed: SpeedLimit = serde_json::from_str(json).expect("parses");
    assert_eq!(parsed, expected);
    assert_eq!(parsed.display(), display);
    assert_eq!(
        serde_json::to_string(&parsed).expect("serializes"),
        json,
        "cached results must round-trip the wire shape"
    );
}

#[test]
fn speed_limit_rejects_unknown_strings() {
    serde_json::from_str::<SpeedLimit>(r#""none""#)
        .expect_err("an undocumented string must fail loudly, not guess");
}

/// The failing body shape from the field: a success response whose edge
/// has `"speed_limit": "unlimited"` parses.
#[test]
fn response_with_unlimited_speed_limit_parses() {
    let response: TraceAttributesResponse = serde_json::from_str(
        r#"{"matched_points": [{"lat": 55.0, "lon": 12.0, "type": "matched", "edge_index": 0}],
            "edges": [{"names": ["A 7"], "speed_limit": "unlimited"}]}"#,
    )
    .expect("a derestricted edge must not fail the chunk");
    assert_eq!(response.edges[0].speed_limit, Some(SpeedLimit::Unlimited));
}

/// Error codes roundtrip through their raw u32, including unknown ones.
#[test]
fn error_codes_roundtrip() {
    let known = [
        (ErrorCode::MissingShape, 114),
        (ErrorCode::TooManyShapePoints, 153),
        (ErrorCode::TraceOptionOutOfBounds, 158),
        (ErrorCode::OffNetwork, 444),
        (ErrorCode::Other(999), 999),
    ];
    for (code, raw) in known {
        assert_eq!(ErrorCode::from(raw), code);
        assert_eq!(u32::from(code), raw);
    }
}

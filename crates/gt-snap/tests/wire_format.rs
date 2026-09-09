//! Validate the typed wire format against the live-captured responses.
//!
//! Every success and error capture must parse into the typed structs, and
//! every well-formed captured request must roundtrip through
//! [`TraceAttributesRequest`] byte-for-byte (as JSON values) - proving the
//! types model exactly what the capture harness sent and the server returned.

#[path = "wire_format/open_world_values.rs"]
mod open_world_values;
#[path = "wire_format/wire_names.rs"]
mod wire_names;

use std::collections::BTreeSet;
use std::fs;

use serde_json::{Value, json};

use gt_snap::wire::{
    ErrorResponse, SnapPointKind, SpeedLimit, TraceAttributesRequest, TraceAttributesResponse,
    TraceOptions,
};
use gt_snap::{CAPTURE_SCENARIOS, DEFAULT_SERVER_URL};

/// The capture scenarios whose response is a successful match, each with a
/// digest baseline of its own.
const SUCCESS_SCENARIOS: &[&str] = &[
    "clean_drive",
    "clean_drive_tuned",
    "dense_10hz",
    "partially_snappable",
    "teleport_gap",
];

/// The same drive as `clean_drive`, captured without the edge-attribute
/// filter the app sends. Its response holds every edge attribute Valhalla
/// knows. The test below asserts it against `clean_drive`.
const UNFILTERED_SCENARIO: &str = "clean_drive_unfiltered";

/// The capture scenarios whose response is a Valhalla JSON error.
const ERROR_SCENARIOS: &[&str] = &[
    "bad_request",
    "option_out_of_bounds",
    "oversized",
    "unsnappable",
];

/// The capture scenarios whose response is not JSON at all (rejected by the
/// reverse proxy before Valhalla sees them).
const HTML_ERROR_SCENARIOS: &[&str] = &["too_large_body"];

/// Every capture scenario must be classified into exactly one of the three
/// lists above, so adding a scenario to [`CAPTURE_SCENARIOS`] without
/// classifying (and thereby parsing) it here fails loudly - the same
/// discipline `EnumCount` applies to the wire-name tables.
#[test]
fn every_scenario_is_classified_exactly_once() {
    let mut classified: Vec<&str> = SUCCESS_SCENARIOS
        .iter()
        .chain(ERROR_SCENARIOS)
        .chain(HTML_ERROR_SCENARIOS)
        .copied()
        .chain([UNFILTERED_SCENARIO])
        .collect();
    classified.sort_unstable();
    let mut expected: Vec<&str> = CAPTURE_SCENARIOS.to_vec();
    expected.sort_unstable();
    assert_eq!(classified, expected);
}

fn read_capture(name: &str) -> Result<String, String> {
    let path = gt_snap::captures_dir().join(name);
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
fn success_captures_parse() {
    for &scenario in SUCCESS_SCENARIOS {
        let body = read_capture(&format!("{scenario}.response.json")).expect("capture");
        let response: TraceAttributesResponse =
            serde_json::from_str(&body).expect("success capture must parse");
        insta::assert_debug_snapshot!(scenario, ResponseDigest::of(&response));
    }
}

/// The filtered and unfiltered captures of one drive parse to the same value:
/// the typed response reads only the fields the app requests, whatever else
/// the server wrote into each.
#[test]
fn the_filtered_and_unfiltered_captures_of_one_drive_parse_to_the_same_response() {
    let filtered = read_capture("clean_drive.response.json").expect("capture");
    let unfiltered =
        read_capture(&format!("{UNFILTERED_SCENARIO}.response.json")).expect("capture");

    assert_eq!(
        serde_json::from_str::<TraceAttributesResponse>(&filtered).expect("the capture parses"),
        serde_json::from_str::<TraceAttributesResponse>(&unfiltered).expect("the capture parses"),
    );
}

#[test]
fn error_fixtures_parse() {
    let digests: Vec<(String, ErrorResponse)> = ERROR_SCENARIOS
        .iter()
        .map(|&scenario| {
            let body = read_capture(&format!("{scenario}.response.json")).expect("capture");
            let error: ErrorResponse =
                serde_json::from_str(&body).expect("error capture must parse");
            (scenario.to_owned(), error)
        })
        .collect();
    insta::assert_debug_snapshot!(digests);
}

#[test]
fn proxy_html_error_parses_as_neither_type() {
    for &scenario in HTML_ERROR_SCENARIOS {
        let body = read_capture(&format!("{scenario}.response.json")).expect("capture");
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
    for &scenario in CAPTURE_SCENARIOS.iter().filter(|&&s| s != "bad_request") {
        let body = read_capture(&format!("{scenario}.request.json")).expect("capture");
        let original: Value = serde_json::from_str(&body).expect("capture JSON");
        let typed: TraceAttributesRequest =
            serde_json::from_value(original.clone()).expect("typed parse");
        let reserialized = serde_json::to_value(&typed).expect("re-serialize");
        assert_eq!(original, reserialized, "{scenario} request drifted");
    }
}

/// The deliberately malformed request (no shape) must NOT parse: `shape` is
/// mandatory on the typed request.
#[test]
fn bad_request_capture_is_not_a_valid_typed_request() {
    let body = read_capture("bad_request.request.json").expect("capture");
    serde_json::from_str::<TraceAttributesRequest>(&body)
        .expect_err("a request without a shape must not be expressible");
}

/// The `trace_options` payload shape: each present option serializes under
/// Valhalla's field name, absent options serialize to nothing (which is why
/// captured requests without `trace_options` still roundtrip
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

/// `server_host` is the granularity of the app's upload-consent bookkeeping:
/// scheme, port, and path changes keep consent, a host change re-prompts, and
/// URLs without a parsable host never count as consented.
#[test]
fn server_host_extracts_the_host_and_only_the_host() {
    assert_eq!(
        gt_snap::server_host(DEFAULT_SERVER_URL).as_deref(),
        Some("valhalla1.openstreetmap.de")
    );
    assert_eq!(
        gt_snap::server_host("http://localhost:8002/some/path").as_deref(),
        Some("localhost")
    );
    assert_eq!(gt_snap::server_host("not a url"), None);
    assert_eq!(gt_snap::server_host(""), None);
    // A host-less URL must not count as a host either.
    assert_eq!(gt_snap::server_host("file:///tmp/x"), None);
}

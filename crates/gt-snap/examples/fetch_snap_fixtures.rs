//! Capture live map-matching fixtures from a Valhalla server.
//!
//! Sends one request per fixture scenario ([`gt_snap::FIXTURE_SCENARIOS`]) to
//! the live server and writes each exchange as a `NAME.request.json` /
//! `NAME.response.json` pair under `tests/fixtures/`, plus a `capture.json`
//! with capture metadata (date, server, per-scenario HTTP status).
//!
//! Fixtures are frozen once committed - matching output drifts as the
//! OpenStreetMap data updates - so re-running this tool is an
//! explicit act and the resulting diff is reviewed like code.
//! See `docs/snap/design.md` ("Testing") and `docs/snap/implementation-plan.md`.
//!
//! Usage: `just snap-fixtures [SCENARIO...]`, or
//! `cargo run -p gt-snap --example fetch_snap_fixtures -- [SCENARIO...]`.
//! Naming scenarios captures only those (an additive capture: entries for
//! untouched scenarios are kept in `capture.json`). No arguments re-captures
//! everything.
//! Point it at a self-hosted server with `GEOTRACE_SNAP_SERVER=http://localhost:8002`.

// Examples favour brevity: the core's robustness restriction lints (no
// unwrap/expect/panic/indexing, no std::env::temp_dir) are not enforced on
// demonstration code, mirroring how clippy.toml relaxes them inside tests.
#![allow(
    clippy::restriction,
    clippy::cognitive_complexity,
    clippy::disallowed_methods,
    clippy::allow_attributes,
    reason = "capture tool: development-only code"
)]

use std::collections::BTreeMap;
use std::error::Error;
use std::time::Duration;
use std::{env, fs, thread};

use serde_json::{Value, json};

use gt_snap::request_plan::{SEARCH_RADIUS_RANGE_M, SnapParams};
use gt_snap::wire::{Costing, ShapePoint, TraceAttributesRequest, TraceOptions};
use gt_snap::{
    CLIENT_ID_HEADER, DEFAULT_SERVER_URL, FIXTURE_SCENARIOS, REQUEST_INTERVAL,
    TRACE_ATTRIBUTES_PATH, fixtures_dir,
};

/// Fixed base timestamp for synthetic traces: 2026-01-01T12:00:00Z.
/// Fixed so re-captures diff cleanly.
const BASE_TIME_UNIX: i64 = 1_767_268_800;

/// Coordinate rounding sent to the server: six decimals is about 0.1 m,
/// tighter than any GNSS receiver and keeps request bodies small.
const COORD_DECIMALS: i32 = 6;

/// Observed FOSSGIS per-request shape point limit (`error_code` 153 names it).
/// One point past it captures the limit error.
const OBSERVED_POINT_LIMIT: usize = 16_000;

/// A lat/lon pair, degrees.
type Coord = (f64, f64);

/// H.C. Andersens Boulevard toward Langebro, Copenhagen. Anchors sampled from
/// the server's own `/route` geometry, so every interpolated point lies on
/// the street: the clean-snap reference route.
const BOULEVARD_ROUTE: &[Coord] = &[
    (55.678_74, 12.564_49),
    (55.677_32, 12.566_07),
    (55.675_76, 12.567_77),
    (55.674_34, 12.570_53),
    (55.672_76, 12.573_62),
    (55.671_80, 12.575_22),
];

/// A street run in Østerbro, ~4 km from the boulevard (beyond the default
/// 2 km breakage distance): the far side of the teleport gap. Sampled from
/// the server's `/route` geometry like [`BOULEVARD_ROUTE`].
const OSTERBRO_ROUTE: &[Coord] = &[
    (55.706_09, 12.580_53),
    (55.706_34, 12.582_36),
    (55.706_61, 12.584_86),
];

/// A short dense stretch on the boulevard for the 10 Hz scenario.
const BOULEVARD_DENSE: &[Coord] = &[(55.676_00, 12.567_20), (55.674_20, 12.569_90)];

/// Mid-channel of the inner harbor south of Langebro: mostly water, but close
/// enough to bridges and quays that some points still snap - the
/// partially-snappable mix of matched/unmatched the plot must surface.
const HARBOR_LINE: &[Coord] = &[(55.666_80, 12.572_30), (55.663_60, 12.575_80)];

/// Open sea in the Øresund, kilometers from any mapped way: guaranteed fully
/// off-network, and stable against future OSM edits near the shore.
const SEA_LINE: &[Coord] = &[(55.680_00, 12.660_00), (55.674_00, 12.674_00)];

/// Roskilde direction: the far end of the deliberately oversized traces.
const ROSKILDE: Coord = (55.642, 12.081);

/// Amplitude of the deterministic cross-track jitter, degrees latitude
/// (about 3 m) - realistic GNSS noise without randomness, so re-captures
/// send byte-identical requests.
const JITTER_DEG: f64 = 3.0e-5;

/// The tuned scenario's non-default trace options: values inside the
/// server-accepted ranges but distinct from any default, so the fixture pins
/// both the tuned-request serialization and the server accepting it.
const TUNED_PARAMS: SnapParams = SnapParams {
    costing: Costing::Auto,
    search_radius_m: Some(25.0),
    turn_penalty_factor: Some(300.0),
    gps_accuracy_override_m: Some(10.0),
};

fn main() -> Result<(), Box<dyn Error>> {
    let server = env::var("GEOTRACE_SNAP_SERVER").unwrap_or_else(|_| DEFAULT_SERVER_URL.to_owned());
    let url = format!("{server}{TRACE_ATTRIBUTES_PATH}");
    let dir = fixtures_dir();
    fs::create_dir_all(&dir)?;

    // Positional arguments select a scenario subset for an additive capture.
    // Without them the capture covers every scenario.
    let args: Vec<String> = env::args().skip(1).collect();
    let selected: Vec<&str> = if args.is_empty() {
        FIXTURE_SCENARIOS.to_vec()
    } else {
        args.iter()
            .map(|name| {
                FIXTURE_SCENARIOS
                    .iter()
                    .copied()
                    .find(|&s| s == name)
                    .unwrap_or_else(|| panic!("unknown fixture scenario {name:?}"))
            })
            .collect()
    };

    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(60))
        .build()?;

    // Start from the existing per-scenario metadata so a subset capture
    // keeps the untouched scenarios' recorded entries.
    let mut summaries_by_name: BTreeMap<String, Value> =
        match fs::read_to_string(dir.join("capture.json")) {
            Ok(existing) => serde_json::from_str::<Value>(&existing)?
                .get("scenarios")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|s| Some((s.get("name")?.as_str()?.to_owned(), s.clone())))
                .collect(),
            Err(_) => BTreeMap::new(),
        };

    for (i, &name) in selected.iter().enumerate() {
        if i > 0 {
            // The public server's fair-use limit: 1 request per user per second.
            thread::sleep(REQUEST_INTERVAL);
        }
        let request = scenario_request(name);
        fs::write(
            dir.join(format!("{name}.request.json")),
            format!("{}\n", serde_json::to_string_pretty(&request)?),
        )?;

        let (header_name, header_value) = CLIENT_ID_HEADER;
        let response = client
            .post(&url)
            .header(header_name, header_value)
            .json(&request)
            .send()?;
        let status = response.status().as_u16();
        let body = response.text()?;

        // Pretty-print JSON bodies so fixture diffs are reviewable. Keep
        // anything unparsable (e.g. the reverse proxy's HTML 413) verbatim.
        let (body_pretty, osm_changeset) = match serde_json::from_str::<Value>(&body) {
            Ok(value) => {
                let changeset = value.get("osm_changeset").cloned();
                (
                    format!("{}\n", serde_json::to_string_pretty(&value)?),
                    changeset,
                )
            }
            Err(_) => (body, None),
        };
        fs::write(dir.join(format!("{name}.response.json")), body_pretty)?;

        println!("{name}: HTTP {status}");
        summaries_by_name.insert(
            name.to_owned(),
            json!({
                "name": name,
                "http_status": status,
                "osm_changeset": osm_changeset,
            }),
        );
    }

    // Emit in canonical scenario order regardless of capture order.
    let summaries: Vec<Value> = FIXTURE_SCENARIOS
        .iter()
        .filter_map(|&name| summaries_by_name.get(name).cloned())
        .collect();
    let capture = json!({
        "captured_at": chrono::Utc::now().to_rfc3339(),
        "server": server,
        "scenarios": summaries,
    });
    fs::write(
        dir.join("capture.json"),
        format!("{}\n", serde_json::to_string_pretty(&capture)?),
    )?;
    println!("Fixtures written to {}", dir.display());
    Ok(())
}

/// The request body for one named scenario.
///
/// Well-formed scenarios are built through the production wire types
/// ([`TraceAttributesRequest`]), so the fixtures exercise exactly the
/// serialization the client will send. Only the deliberately malformed
/// `bad_request` stays a raw JSON literal.
///
/// Panics on an unknown name: the scenario list and this table must stay in
/// sync, and the fixture validation test pins both to [`FIXTURE_SCENARIOS`].
fn scenario_request(name: &str) -> Value {
    let typed = |request: TraceAttributesRequest| {
        serde_json::to_value(request).expect("serializing a request never fails")
    };
    match name {
        // ~40 points at 1 Hz along the boulevard with small deterministic
        // jitter: every point should come back `matched`.
        "clean_drive" => typed(TraceAttributesRequest::new(
            Costing::Auto,
            trace(40, BOULEVARD_ROUTE, Some(1.0)),
        )),
        // The clean drive with every advanced trace option set, built through
        // the production parameter path ([`SnapParams::trace_options`]): pins the
        // tuned-request serialization and the server accepting in-range
        // values.
        "clean_drive_tuned" => {
            let mut request = TraceAttributesRequest::new(
                TUNED_PARAMS.costing,
                trace(40, BOULEVARD_ROUTE, Some(1.0)),
            );
            request.trace_options = TUNED_PARAMS.trace_options(None);
            typed(request)
        }
        // The same trace without an attribute filter: documents everything
        // the server can return, so the deliberate filter subset stays an
        // informed choice.
        "clean_drive_unfiltered" => typed(TraceAttributesRequest::unfiltered(
            Costing::Auto,
            trace(40, BOULEVARD_ROUTE, Some(1.0)),
        )),
        // 150 points at ~1.5 m spacing (10 Hz driving) with no timestamps:
        // spacing below the default interpolation distance, so most points
        // come back `interpolated`.
        "dense_10hz" => typed(TraceAttributesRequest::new(
            Costing::Auto,
            trace(150, BOULEVARD_DENSE, None),
        )),
        // A line across the harbor: some points snap to bridges and quays,
        // the rest are unmatched - the mixed result the error plot surfaces.
        "partially_snappable" => typed(TraceAttributesRequest::new(
            Costing::Auto,
            trace(20, HARBOR_LINE, Some(1.0)),
        )),
        // Open sea: fully off the road network. Captured reality check: the
        // server rejects the whole request with 400 and `error_code` 444.
        "unsnappable" => typed(TraceAttributesRequest::new(
            Costing::Auto,
            trace(20, SEA_LINE, Some(1.0)),
        )),
        // Two clean street segments ~4 km apart, far beyond the default
        // breakage distance. Captured reality check: the server does NOT set
        // route discontinuity flags for the jump - it marks the boundary
        // points `unmatched` and continues, with `edge_index` running on
        // across the physically impossible junction. Snapped-track splitting
        // must therefore treat unmatched runs as breaks too. The only flag
        // exemplar is in `partially_snappable`.
        "teleport_gap" => {
            let mut shape = trace(20, BOULEVARD_ROUTE, Some(1.0));
            shape.extend(trace_from(20, OSTERBRO_ROUTE, Some(1.0), 20));
            typed(TraceAttributesRequest::new(Costing::Auto, shape))
        }
        // No shape at all: captures the server's real 400 error payload
        // (`error_code` 114). Deliberately malformed, so not expressible
        // through the typed request.
        "bad_request" => json!({ "costing": "auto" }),
        // One trace option just past the server's bound: captures the real
        // rejection (400, `error_code` 158) that the client-side clamps in
        // [`SnapParams`] exist to prevent. Raw [`TraceOptions`], bypassing
        // the clamping deliberately.
        "option_out_of_bounds" => {
            let mut request =
                TraceAttributesRequest::new(Costing::Auto, trace(2, BOULEVARD_ROUTE, Some(1.0)));
            request.trace_options = Some(TraceOptions {
                gps_accuracy: None,
                search_radius: Some(SEARCH_RADIUS_RANGE_M.end() + 1.0),
                turn_penalty_factor: None,
            });
            typed(request)
        }
        // One point past the server's shape limit: captures the limit error
        // (400, `error_code` 153), which names the maximum - the observation
        // that set the chunk-size constant.
        "oversized" => typed(TraceAttributesRequest::new(
            Costing::Auto,
            trace(
                OBSERVED_POINT_LIMIT + 1,
                &[BOULEVARD_ROUTE[0], ROSKILDE],
                Some(1.0),
            ),
        )),
        // A body so large the reverse proxy rejects it before Valhalla sees
        // it: captures the HTML 413 page - the client must survive non-JSON
        // error bodies.
        "too_large_body" => typed(TraceAttributesRequest::new(
            Costing::Auto,
            trace(30_000, &[BOULEVARD_ROUTE[0], ROSKILDE], Some(1.0)),
        )),
        other => panic!("unknown fixture scenario {other:?}"),
    }
}

/// `count` shape points spread evenly along the anchor chain `route`, with
/// deterministic cross-track jitter. `seconds_per_point` adds `time` values
/// from [`BASE_TIME_UNIX`]. `None` omits timestamps.
fn trace(count: usize, route: &[Coord], seconds_per_point: Option<f64>) -> Vec<ShapePoint> {
    trace_from(count, route, seconds_per_point, 0)
}

/// Like [`trace`], but with timestamps continuing from point index `time_offset`.
fn trace_from(
    count: usize,
    route: &[Coord],
    seconds_per_point: Option<f64>,
    time_offset: usize,
) -> Vec<ShapePoint> {
    (0..count)
        .map(|i| {
            let t = i as f64 / (count - 1).max(1) as f64;
            let (lat, lon) = point_along(route, t);
            // A -1, 0, +1 zigzag: realistic noise, deterministic re-capture.
            let jitter = JITTER_DEG * ((i % 3) as f64 - 1.0);
            ShapePoint {
                lat: round_coord(lat + jitter),
                lon: round_coord(lon),
                time: seconds_per_point
                    .map(|step| BASE_TIME_UNIX + ((time_offset + i) as f64 * step) as i64),
            }
        })
        .collect()
}

/// The point a fraction `t` (0..=1) along the anchor chain, measured by
/// cumulative flat-earth segment length - accurate enough for fixture
/// geometry at city scale.
fn point_along(route: &[Coord], t: f64) -> Coord {
    assert!(route.len() >= 2, "route needs at least two anchors");
    let seg_len = |a: Coord, b: Coord| {
        let dlat = b.0 - a.0;
        // Compress longitude toward the pole so segment lengths are
        // proportional to ground distance.
        let dlon = (b.1 - a.1) * a.0.to_radians().cos();
        dlat.hypot(dlon)
    };
    let total: f64 = route.windows(2).map(|w| seg_len(w[0], w[1])).sum();
    let mut remaining = t.clamp(0.0, 1.0) * total;
    for w in route.windows(2) {
        let len = seg_len(w[0], w[1]);
        if remaining <= len || len == 0.0 {
            let f = if len == 0.0 { 0.0 } else { remaining / len };
            return (
                w[0].0 + (w[1].0 - w[0].0) * f,
                w[0].1 + (w[1].1 - w[0].1) * f,
            );
        }
        remaining -= len;
    }
    *route.last().unwrap_or(&route[0])
}

fn round_coord(value: f64) -> f64 {
    let scale = 10f64.powi(COORD_DECIMALS);
    (value * scale).round() / scale
}

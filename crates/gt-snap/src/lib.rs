//! Snap-to-road matching against a Valhalla map-matching server.
//!
//! Matches recorded tracks against the OpenStreetMap road network and treats
//! the matched geometry as the reference the receiver "should" have produced:
//! the snapped track drawn on the map, and the per-point snap error plotted
//! against time. See `docs/snap/design.md` for the full design.
//!
//! This crate currently ships the fixture capture harness
//! (`examples/fetch_fixtures.rs`, wrapped by `just snap-fixtures`) and the
//! live-captured API fixtures under `tests/fixtures/` that the wire types are
//! developed against. The typed client, chunking, and stitching land on top.

use std::path::PathBuf;
use std::time::Duration;

pub mod request_plan;
pub mod snapped_track;
pub mod stitch;
pub mod transport;
pub mod wire;

/// Base URL of the default map-matching server: the public Valhalla instance
/// hosted by FOSSGIS e.V. Free under a fair-usage policy - published apps must
/// identify themselves (see [`CLIENT_ID_HEADER`]) and stay within
/// [`REQUEST_INTERVAL`].
pub const DEFAULT_SERVER_URL: &str = "https://valhalla1.openstreetmap.de";

/// Path of the sole endpoint this crate uses, appended to the server base URL.
pub const TRACE_ATTRIBUTES_PATH: &str = "/trace_attributes";

/// Identifying request header (name, value) required by the FOSSGIS usage
/// policy for published apps.
pub const CLIENT_ID_HEADER: (&str, &str) = ("X-Client-Id", "geotrace");

/// Minimum spacing between requests to the public server: its rate limit is
/// 1 request per user per second, enforced client-side so we never rely on
/// the server to throttle us.
pub const REQUEST_INTERVAL: Duration = Duration::from_secs(1);

/// The host component of a server base URL, or `None` when the URL does not
/// parse or has no host.
///
/// This is the granularity at which the app tracks upload consent: recorded
/// location data leaves the machine, so acknowledgment is per host and a URL
/// change to a different host must re-prompt.
pub fn server_host(url: &str) -> Option<String> {
    let parsed = reqwest::Url::parse(url).ok()?;
    parsed.host_str().map(str::to_owned)
}

/// The canonical fixture scenarios captured from the live server.
///
/// Each entry names a `NAME.request.json` / `NAME.response.json` pair under
/// [`fixtures_dir`]. The set is deliberately one scenario per server behavior
/// the client must handle; the capture harness builds the matching requests.
pub const FIXTURE_SCENARIOS: &[&str] = &[
    "clean_drive",
    "clean_drive_unfiltered",
    "dense_10hz",
    "partially_snappable",
    "unsnappable",
    "teleport_gap",
    "bad_request",
    "oversized",
    "too_large_body",
];

/// Directory holding the captured request/response fixture pairs.
///
/// Resolved from the crate manifest dir, so it is only meaningful for
/// development tooling (the capture harness and tests) running inside the
/// workspace - never in the shipped application.
pub fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
}

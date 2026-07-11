//! Typed wire format for the Valhalla `trace_attributes` endpoint.
//!
//! Mirrors the wire exactly: serde attributes carry Valhalla's field and
//! value names, while Rust type names use the project's snap vocabulary
//! (Valhalla's "matched"/"unmatched" become snapped/unsnapped, see
//! `docs/snap/design.md`).
//! Only fields with a consumer are modeled; serde skips everything else in
//! the responses (`alternate_paths`, `raw_score`, `units`, ...).
//!
//! Every type is validated against the live-captured fixtures under
//! `tests/fixtures/` - captured by `examples/fetch_fixtures.rs`, which builds
//! its requests from these same types so the fixtures always exercise the
//! production serialization.

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

/// The response attributes production requests ask for: the matched-point
/// group, the matched shape, the map-data version and run confidence for
/// cache metadata and the snap status, and the edge attribute subset shown on
/// snapped-track hover (see the feature inventory in docs/snap/design.md).
///
/// Captured reality: top-level fields like `osm_changeset` and
/// `confidence_score` are dropped by the server unless explicitly listed.
pub const INCLUDED_ATTRIBUTES: &[&str] = &[
    "matched.point",
    "matched.type",
    "matched.edge_index",
    "matched.distance_along_edge",
    "matched.distance_from_trace_point",
    "matched.begin_route_discontinuity",
    "matched.end_route_discontinuity",
    "shape",
    "osm_changeset",
    "confidence_score",
    "edge.names",
    "edge.way_id",
    "edge.road_class",
    "edge.speed_limit",
    "edge.surface",
    "edge.begin_shape_index",
    "edge.end_shape_index",
];

/// One input shape point: a recorded position, optionally timestamped.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ShapePoint {
    pub lat: f64,
    pub lon: f64,
    /// Unix seconds. Optional on the wire; timestamps aid the matcher's
    /// candidate selection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time: Option<i64>,
}

/// The Valhalla costing model a snap run matches against.
///
/// Deliberately only one per distinct road network (see the design doc's
/// costing decision); the remaining Valhalla costings are auto variants that
/// differ only via special-lane tags.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    strum::Display,
    strum::EnumString,
    strum::EnumCount,
    Serialize,
    Deserialize,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum Costing {
    Auto,
    Bicycle,
    Pedestrian,
}

/// Whether an attribute filter includes or excludes the listed attributes.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    strum::Display,
    strum::EnumString,
    strum::EnumCount,
    Serialize,
    Deserialize,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum FilterAction {
    Include,
    Exclude,
}

/// The `filters` request object trimming the response to a deliberate subset.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AttributeFilter {
    pub action: FilterAction,
    pub attributes: Vec<String>,
}

impl AttributeFilter {
    /// The filter production requests send: include exactly
    /// [`INCLUDED_ATTRIBUTES`].
    pub fn production() -> Self {
        Self {
            action: FilterAction::Include,
            attributes: INCLUDED_ATTRIBUTES.iter().map(|&a| a.to_owned()).collect(),
        }
    }
}

/// A `trace_attributes` request body.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TraceAttributesRequest {
    pub costing: Costing,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filters: Option<AttributeFilter>,
    pub shape: Vec<ShapePoint>,
}

impl TraceAttributesRequest {
    /// A production request: the given points with the production attribute
    /// filter attached.
    pub fn new(costing: Costing, shape: Vec<ShapePoint>) -> Self {
        Self {
            costing,
            filters: Some(AttributeFilter::production()),
            shape,
        }
    }

    /// A request without an attribute filter, returning everything the
    /// server has. Used only by the capture harness's unfiltered reference
    /// scenario.
    pub fn unfiltered(costing: Costing, shape: Vec<ShapePoint>) -> Self {
        Self {
            costing,
            filters: None,
            shape,
        }
    }
}

/// Per-point match kind, mirroring Valhalla's `matched` / `interpolated` /
/// `unmatched` wire names.
///
/// Interpolated is the common case on slow urban recordings, not an anomaly:
/// at 1 Hz every point below 36 km/h falls under the default 10 m
/// interpolation distance. Interpolated points carry full error values.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    strum::Display,
    strum::EnumString,
    strum::EnumCount,
    Serialize,
    Deserialize,
)]
pub enum SnapPointKind {
    #[serde(rename = "matched")]
    #[strum(serialize = "matched")]
    Snapped,
    #[serde(rename = "interpolated")]
    #[strum(serialize = "interpolated")]
    Interpolated,
    #[serde(rename = "unmatched")]
    #[strum(serialize = "unmatched")]
    Unsnapped,
}

/// One snapped point of the response, 1:1 with the request's shape points.
///
/// Field names follow the wire (`matched_points[]` entries); notably
/// `distance_from_trace_point` is the snap error in meters. Unsnapped points
/// carry neither an error nor an edge.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
pub struct SnappedPoint {
    pub lat: f64,
    pub lon: f64,
    #[serde(rename = "type")]
    pub kind: SnapPointKind,
    /// Index into [`TraceAttributesResponse::edges`], or `None` when the
    /// point has no edge association. Captured reality: the wire encodes
    /// "no edge" as `u64::MAX` on interpolated points (not by omitting the
    /// field); that sentinel is folded into `None` here and never escapes
    /// the wire layer.
    #[serde(default, deserialize_with = "edge_index_from_wire")]
    pub edge_index: Option<u64>,
    /// Position along the matched edge as a 0..=1 fraction.
    #[serde(default)]
    pub distance_along_edge: Option<f64>,
    /// The snap error: meters from the recorded point to this snapped
    /// position.
    #[serde(default)]
    pub distance_from_trace_point: Option<f64>,
    /// Captured reality: rarely set - large teleports produce unmatched
    /// boundary points instead, so track splitting must not rely on these
    /// flags alone.
    #[serde(default)]
    pub begin_route_discontinuity: bool,
    #[serde(default)]
    pub end_route_discontinuity: bool,
}

/// The wire value Valhalla uses for "this point has no edge association".
const EDGE_INDEX_NONE: u64 = u64::MAX;

/// Fold the wire's no-edge sentinel into `None` (see
/// [`SnappedPoint::edge_index`]).
fn edge_index_from_wire<'de, D>(deserializer: D) -> Result<Option<u64>, D::Error>
where
    D: Deserializer<'de>,
{
    let raw = Option::<u64>::deserialize(deserializer)?;
    Ok(raw.filter(|&index| index != EDGE_INDEX_NONE))
}

/// Valhalla's road classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, strum::EnumCount, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoadClass {
    Motorway,
    Trunk,
    Primary,
    Secondary,
    Tertiary,
    Unclassified,
    Residential,
    ServiceOther,
    /// Forward compatibility: a class this client does not know yet.
    #[serde(other)]
    Unknown,
}

/// Valhalla's surface classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, strum::EnumCount, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Surface {
    PavedSmooth,
    Paved,
    PavedRough,
    Compacted,
    Dirt,
    Gravel,
    Path,
    Impassable,
    /// Forward compatibility: a surface this client does not know yet.
    #[serde(other)]
    Unknown,
}

/// One matched edge, trimmed to the requested attribute subset.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Edge {
    #[serde(default)]
    pub names: Vec<String>,
    #[serde(default)]
    pub way_id: Option<u64>,
    #[serde(default)]
    pub road_class: Option<RoadClass>,
    /// km/h.
    #[serde(default)]
    pub speed_limit: Option<u32>,
    #[serde(default)]
    pub surface: Option<Surface>,
    /// Index range this edge covers in the decoded response shape.
    /// Drives the snapped-track segment split. Captured reality: consecutive
    /// edges' ranges are NOT always contiguous even on an unbroken route, so
    /// index gaps alone never indicate a break.
    #[serde(default)]
    pub begin_shape_index: Option<usize>,
    #[serde(default)]
    pub end_shape_index: Option<usize>,
}

/// A successful `trace_attributes` response, trimmed to the modeled subset.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct TraceAttributesResponse {
    /// 1:1 with the request's shape points.
    #[serde(rename = "matched_points", default)]
    pub snapped_points: Vec<SnappedPoint>,
    #[serde(default)]
    pub edges: Vec<Edge>,
    /// The snapped track geometry as an encoded polyline (6-digit precision).
    #[serde(default)]
    pub shape: Option<String>,
    /// OSM data version the match was computed against; cache metadata.
    #[serde(default)]
    pub osm_changeset: Option<u64>,
    /// Per-run trust indicator, 0..=1.
    #[serde(default)]
    pub confidence_score: Option<f64>,
    /// Kept raw: no live exemplar of a warning exists (out-of-range options
    /// reject instead of warning), so guessing a shape would be worse than
    /// passing the values through to the log.
    #[serde(default)]
    pub warnings: Vec<Value>,
}

/// A Valhalla error body. Always JSON with a stable numeric code - but note
/// the reverse proxy can reject requests before Valhalla sees them, with
/// non-JSON bodies (captured: an HTML 413 page).
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ErrorResponse {
    pub error: String,
    pub error_code: ErrorCode,
    pub status: String,
    pub status_code: u16,
}

/// The error codes observed from the live server, plus a catch-all for
/// everything not yet seen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(from = "u32")]
pub enum ErrorCode {
    /// 114: request lacks `shape` (or `encoded_polyline`).
    MissingShape,
    /// 153: more shape points than the server accepts per request. The
    /// error text names the limit (16 000 on the FOSSGIS instance).
    TooManyShapePoints,
    /// 158: a trace option is out of bounds. Captured reality: out-of-range
    /// options are rejected, never clamped.
    TraceOptionOutOfBounds,
    /// 444: the matcher found no path - every point is off the road network.
    /// Not a failure for stitching: it maps to all-unsnapped points.
    OffNetwork,
    /// A code this client does not know yet, kept verbatim.
    Other(u32),
}

impl From<u32> for ErrorCode {
    fn from(code: u32) -> Self {
        match code {
            114 => Self::MissingShape,
            153 => Self::TooManyShapePoints,
            158 => Self::TraceOptionOutOfBounds,
            444 => Self::OffNetwork,
            other => Self::Other(other),
        }
    }
}

impl From<ErrorCode> for u32 {
    fn from(code: ErrorCode) -> Self {
        match code {
            ErrorCode::MissingShape => 114,
            ErrorCode::TooManyShapePoints => 153,
            ErrorCode::TraceOptionOutOfBounds => 158,
            ErrorCode::OffNetwork => 444,
            ErrorCode::Other(other) => other,
        }
    }
}

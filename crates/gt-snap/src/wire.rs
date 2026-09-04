//! Typed wire format for the Valhalla `trace_attributes` endpoint.
//!
//! Mirrors the wire exactly: serde attributes carry Valhalla's field and
//! value names, while Rust type names use the project's snap vocabulary
//! (Valhalla's "matched"/"unmatched" become snapped/unsnapped, see
//! `docs/snap/design.md`).
//! Only fields with a consumer are modeled. Serde skips everything else in
//! the responses (`alternate_paths`, `raw_score`, `units`, ...).
//!
//! Every type is validated against the live-captured fixtures under
//! `tests/fixtures/` - captured by `examples/fetch_snap_fixtures.rs`, which builds
//! its requests from these same types so the fixtures always exercise the
//! production serialization.

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

/// The response attributes production requests specify: the matched-point
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
    /// Unix seconds. Optional on the wire. Timestamps aid the matcher's
    /// candidate selection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time: Option<i64>,
}

/// The Valhalla costing model a snap run matches against.
///
/// One per distinct road network (see the design doc's costing decision).
/// The remaining Valhalla costings are auto variants that differ only via
/// special-lane tags.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    strum::Display,
    strum::EnumString,
    strum::EnumCount,
    strum::EnumIter,
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

impl Costing {
    /// Canonical human-readable name shown in the UI, e.g. the costing combo.
    ///
    /// "Auto" is Valhalla's name for the motor-vehicle road network, kept
    /// as-is so the UI matches the vocabulary of the Valhalla docs.
    pub fn display_name(self) -> &'static str {
        match self {
            Costing::Auto => "Auto",
            Costing::Bicycle => "Bicycle",
            Costing::Pedestrian => "Pedestrian",
        }
    }
}

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

/// The `trace_options` tuning parameters this client sets.
///
/// Captured reality: out-of-range values are rejected (400, code 158), not
/// clamped, so senders must bound values client-side - production requests
/// are built through `request_plan::SnapParams`, which clamps to the
/// empirically pinned ranges (the `option_out_of_bounds` fixture is the
/// rejection exemplar).
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct TraceOptions {
    /// Expected GNSS accuracy in meters. Bounds how far off-road the matcher
    /// may place a point.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gps_accuracy: Option<f64>,
    /// Meters around each input point searched for candidate road edges.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub search_radius: Option<f64>,
    /// Cost multiplier penalizing route reversals. Raising it smooths
    /// wandering matches at intersections.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_penalty_factor: Option<f64>,
}

/// A `trace_attributes` request body.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TraceAttributesRequest {
    pub costing: Costing,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filters: Option<AttributeFilter>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace_options: Option<TraceOptions>,
    pub shape: Vec<ShapePoint>,
}

impl TraceAttributesRequest {
    /// A production request: the points in `shape` with the production
    /// attribute filter attached.
    pub fn new(costing: Costing, shape: Vec<ShapePoint>) -> Self {
        Self {
            costing,
            filters: Some(AttributeFilter::production()),
            trace_options: None,
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
            trace_options: None,
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
/// Field names follow the wire (`matched_points[]` entries). Notably
/// `distance_from_trace_point` is the snap error in meters. Unsnapped points
/// have neither an error nor an edge.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
pub struct SnappedPoint {
    pub lat: f64,
    pub lon: f64,
    #[serde(rename = "type")]
    pub kind: SnapPointKind,
    /// Index into [`TraceAttributesResponse::edges`], or `None` when the
    /// point has no edge association. Captured reality: the wire encodes
    /// "no edge" as `u64::MAX` on interpolated points (not by omitting the
    /// field). That sentinel value is folded into `None` here and never escapes
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

/// Fold the wire's no-edge sentinel value into `None` (see
/// [`SnappedPoint::edge_index`]).
fn edge_index_from_wire<'de, D>(deserializer: D) -> Result<Option<u64>, D::Error>
where
    D: Deserializer<'de>,
{
    let raw = Option::<u64>::deserialize(deserializer)?;
    Ok(raw.filter(|&index| index != EDGE_INDEX_NONE))
}

/// Valhalla's road classification.
///
/// `Serialize` exists for persisting cached snap results, not for requests.
/// [`Unknown`](Self::Unknown) serializes as `"unknown"`, which the
/// `#[serde(other)]` catch-all folds back into `Unknown` on decode, so
/// roundtrips are stable even for classes this client does not model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, strum::EnumCount, Serialize, Deserialize)]
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

impl RoadClass {
    /// Canonical human-readable name shown in the UI (snapped-track hover).
    /// Single source of truth for this type's display spelling, like
    /// [`Costing::display_name`].
    pub fn display_name(self) -> &'static str {
        match self {
            RoadClass::Motorway => "Motorway",
            RoadClass::Trunk => "Trunk",
            RoadClass::Primary => "Primary",
            RoadClass::Secondary => "Secondary",
            RoadClass::Tertiary => "Tertiary",
            RoadClass::Unclassified => "Unclassified",
            RoadClass::Residential => "Residential",
            RoadClass::ServiceOther => "Service or other",
            RoadClass::Unknown => "Unknown",
        }
    }
}

/// Valhalla's surface classification.
///
/// `Serialize` exists for persisting cached snap results. The `Unknown`
/// roundtrip works like [`RoadClass`]'s.
#[derive(Debug, Clone, Copy, PartialEq, Eq, strum::EnumCount, Serialize, Deserialize)]
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

impl Surface {
    /// Canonical human-readable name shown in the UI (snapped-track hover).
    pub fn display_name(self) -> &'static str {
        match self {
            Surface::PavedSmooth => "Paved smooth",
            Surface::Paved => "Paved",
            Surface::PavedRough => "Paved rough",
            Surface::Compacted => "Compacted",
            Surface::Dirt => "Dirt",
            Surface::Gravel => "Gravel",
            Surface::Path => "Path",
            Surface::Impassable => "Impassable",
            Surface::Unknown => "Unknown",
        }
    }
}

/// A posted speed limit. Valhalla reports most edges as a km/h number, but
/// a derestricted road (a German autobahn stretch) comes back as the string
/// `"unlimited"`.
///
/// `Serialize` mirrors the wire shape (number or `"unlimited"`), so cached
/// results round-trip unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpeedLimit {
    /// km/h.
    Kmh(u32),
    /// Explicitly derestricted - `"unlimited"` on the wire.
    Unlimited,
}

/// The one non-numeric value Valhalla documents for `edge.speed_limit`.
const SPEED_LIMIT_UNLIMITED: &str = "unlimited";

impl SpeedLimit {
    /// Hover-text rendering: `"120 km/h"` or `"Unlimited"`.
    pub fn display(self) -> String {
        match self {
            SpeedLimit::Kmh(kmh) => format!("{kmh} km/h"),
            SpeedLimit::Unlimited => "Unlimited".to_owned(),
        }
    }
}

impl Serialize for SpeedLimit {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            SpeedLimit::Kmh(kmh) => serializer.serialize_u32(*kmh),
            SpeedLimit::Unlimited => serializer.serialize_str(SPEED_LIMIT_UNLIMITED),
        }
    }
}

impl<'de> Deserialize<'de> for SpeedLimit {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        /// The two wire shapes, discriminated by JSON type.
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Wire {
            Kmh(u32),
            Text(String),
        }
        match Wire::deserialize(deserializer)? {
            Wire::Kmh(kmh) => Ok(SpeedLimit::Kmh(kmh)),
            Wire::Text(text) if text == SPEED_LIMIT_UNLIMITED => Ok(SpeedLimit::Unlimited),
            Wire::Text(text) => Err(serde::de::Error::invalid_value(
                serde::de::Unexpected::Str(&text),
                &"a km/h number or \"unlimited\"",
            )),
        }
    }
}

/// One matched edge, trimmed to the attributes [`AttributeFilter::production`]
/// lists.
///
/// `Serialize` exists for persisting cached snap results, not for requests.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Edge {
    #[serde(default)]
    pub names: Vec<String>,
    #[serde(default)]
    pub way_id: Option<u64>,
    #[serde(default)]
    pub road_class: Option<RoadClass>,
    #[serde(default)]
    pub speed_limit: Option<SpeedLimit>,
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
    /// OSM data version the match was computed against.
    #[serde(default)]
    pub osm_changeset: Option<u64>,
    /// Per-run confidence, 0..=1.
    #[serde(default)]
    pub confidence_score: Option<f64>,
    /// Kept raw: no live exemplar of a warning exists to model a shape.
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
    /// Not a failure for merging: it maps to all-unsnapped points.
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

//! The wire values the server may extend: an unknown enum spelling, the
//! `u64::MAX` edge index, a speed limit given as a string, and an error code.

use serde::Deserialize;
use serde::de::IntoDeserializer;
use serde::de::value::{Error as DeError, StrDeserializer};
use strum::EnumCount;

use gt_snap::wire::{ErrorCode, RoadClass, SpeedLimit, Surface, TraceAttributesResponse};

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

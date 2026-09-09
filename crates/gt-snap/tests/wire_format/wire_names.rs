//! The wire and display spellings of the closed `enum` types, each table
//! asserted against `EnumCount` so a new variant cannot be forgotten.

use serde::Deserialize;
use serde::de::IntoDeserializer;
use serde::de::value::{Error as DeError, StrDeserializer};
use strum::EnumCount;

use gt_snap::wire::{Costing, FilterAction, RoadClass, SnapPointKind, Surface};

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

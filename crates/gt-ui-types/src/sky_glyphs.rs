//! The persisted choice between the two sky-glyph variants.

/// Which sky-glyph variant the map overlay draws.
///
/// Serialized into the map settings. The wire names are pinned by
/// `wire_names_are_stable` below.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Default,
    strum::Display,
    strum::EnumString,
    strum::EnumCount,
    strum::EnumIter,
    serde::Serialize,
    serde::Deserialize,
)]
#[strum(serialize_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum SkyGlyphVariant {
    /// The minimal variant: an annulus with one bead per fix satellite at
    /// its azimuth.
    #[default]
    Ring,
    /// The detailed variant: a miniature sky plot with dots placed by
    /// azimuth and elevation.
    Disc,
}

impl SkyGlyphVariant {
    /// The variant's sentence-case UI label.
    pub fn label(self) -> &'static str {
        match self {
            Self::Ring => "Ring",
            Self::Disc => "Disc",
        }
    }
}

#[cfg(test)]
mod tests {
    use serde::Deserialize;
    use serde::de::IntoDeserializer;
    use serde::de::value::{Error as DeError, StrDeserializer};
    use std::str::FromStr;
    use strum::EnumCount;

    use super::SkyGlyphVariant;

    #[test]
    fn wire_names_are_stable() {
        let expected = [
            (SkyGlyphVariant::Ring, "ring"),
            (SkyGlyphVariant::Disc, "disc"),
        ];
        assert_eq!(expected.len(), SkyGlyphVariant::COUNT);
        for (variant, wire) in expected {
            let de: StrDeserializer<'_, DeError> = wire.into_deserializer();
            assert_eq!(
                SkyGlyphVariant::deserialize(de),
                Ok(variant),
                "deserializing {wire:?}"
            );
            assert_eq!(variant.to_string(), wire);
            assert_eq!(SkyGlyphVariant::from_str(wire), Ok(variant));
        }
    }
}

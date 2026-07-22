//! Pre-tessellated icon meshes embedded at build time.
//!
//! The build script tessellates every SVG in `assets/icons/` into normalized
//! triangle meshes (one per size bucket, see [gt_icon_tessellate]) and embeds
//! them as a postcard blob.
//! [IconMeshLibrary::embedded] decodes that blob once into per-icon
//! [IconTessellation]s; the mesh-based renderers build on top of it in later
//! steps, while the texture-based icon path in the private `icons` module
//! keeps rendering until they land.

use std::collections::BTreeMap;

use gt_icon_tessellate::IconTessellation;
use gt_types::MarkerIcon;

/// Identifies one marker icon SVG asset.
///
/// The wire name (strum snake_case) is the asset's file stem in
/// `assets/icons/`, which is also how the embedded blob keys its meshes.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    strum::Display,
    strum::EnumString,
    strum::EnumIter,
    strum::EnumCount,
)]
#[strum(serialize_all = "snake_case")]
pub enum IconId {
    Check,
    CircleMarker,
    ConnectionLost,
    Cross,
    Download,
    Error,
    Gear,
    GhostFix,
    Lightning,
    LogPin,
    Pin,
    Refresh,
    Satellite,
    SatelliteLost,
    Upload,
    Warning,
    Wrench,
}

impl From<MarkerIcon> for IconId {
    fn from(icon: MarkerIcon) -> Self {
        match icon {
            MarkerIcon::Pin => Self::Pin,
            MarkerIcon::Cross => Self::Cross,
            MarkerIcon::Circle => Self::CircleMarker,
            MarkerIcon::Lightning => Self::Lightning,
            MarkerIcon::Warning => Self::Warning,
            MarkerIcon::Error => Self::Error,
            MarkerIcon::Check => Self::Check,
            MarkerIcon::Log => Self::LogPin,
            MarkerIcon::Satellite => Self::Satellite,
            MarkerIcon::SatelliteLost => Self::SatelliteLost,
            MarkerIcon::Gear => Self::Gear,
            MarkerIcon::Refresh => Self::Refresh,
            MarkerIcon::Download => Self::Download,
            MarkerIcon::Upload => Self::Upload,
            MarkerIcon::Wrench => Self::Wrench,
        }
    }
}

/// The postcard blob baked by the build script: sorted
/// `(file stem, tessellation)` pairs for every icon asset.
static ICON_MESH_BYTES: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/icon_meshes.postcard"));

#[derive(Debug, thiserror::Error)]
pub enum IconMeshLibraryError {
    #[error("failed to decode the embedded icon meshes")]
    Decode(#[from] postcard::Error),
    #[error("embedded icon meshes contain unknown icon {name:?}")]
    UnknownIcon { name: String },
    #[error("embedded icon meshes are missing {icon}")]
    MissingIcon { icon: IconId },
    #[error("embedded icon meshes contain {icon} twice")]
    DuplicateIcon { icon: IconId },
}

/// All pre-tessellated icon meshes, one [IconTessellation] per [IconId].
///
/// One named field per icon keeps the library total by construction: adding
/// an [IconId] variant fails compilation here until its mesh is wired up,
/// and [IconMeshLibrary::tessellation] stays infallible.
#[derive(Debug, Clone)]
pub struct IconMeshLibrary {
    check: IconTessellation,
    circle_marker: IconTessellation,
    connection_lost: IconTessellation,
    cross: IconTessellation,
    download: IconTessellation,
    error: IconTessellation,
    gear: IconTessellation,
    ghost_fix: IconTessellation,
    lightning: IconTessellation,
    log_pin: IconTessellation,
    pin: IconTessellation,
    refresh: IconTessellation,
    satellite: IconTessellation,
    satellite_lost: IconTessellation,
    upload: IconTessellation,
    warning: IconTessellation,
    wrench: IconTessellation,
}

impl IconMeshLibrary {
    /// Decode the meshes embedded by the build script.
    ///
    /// The embedded blob is generated from the same assets and types at build
    /// time, so an error here means a corrupted binary; the
    /// `embedded_meshes_decode_for_every_icon` test guards the bake in CI.
    pub fn embedded() -> Result<Self, IconMeshLibraryError> {
        Self::decode(ICON_MESH_BYTES)
    }

    fn decode(bytes: &[u8]) -> Result<Self, IconMeshLibraryError> {
        let entries: Vec<(String, IconTessellation)> = postcard::from_bytes(bytes)?;
        let mut by_icon: BTreeMap<IconId, IconTessellation> = BTreeMap::new();
        for (name, tessellation) in entries {
            let icon: IconId = name
                .parse()
                .map_err(|_unknown| IconMeshLibraryError::UnknownIcon { name })?;
            if by_icon.insert(icon, tessellation).is_some() {
                return Err(IconMeshLibraryError::DuplicateIcon { icon });
            }
        }
        let mut take = |icon: IconId| {
            by_icon
                .remove(&icon)
                .ok_or(IconMeshLibraryError::MissingIcon { icon })
        };
        Ok(Self {
            check: take(IconId::Check)?,
            circle_marker: take(IconId::CircleMarker)?,
            connection_lost: take(IconId::ConnectionLost)?,
            cross: take(IconId::Cross)?,
            download: take(IconId::Download)?,
            error: take(IconId::Error)?,
            gear: take(IconId::Gear)?,
            ghost_fix: take(IconId::GhostFix)?,
            lightning: take(IconId::Lightning)?,
            log_pin: take(IconId::LogPin)?,
            pin: take(IconId::Pin)?,
            refresh: take(IconId::Refresh)?,
            satellite: take(IconId::Satellite)?,
            satellite_lost: take(IconId::SatelliteLost)?,
            upload: take(IconId::Upload)?,
            warning: take(IconId::Warning)?,
            wrench: take(IconId::Wrench)?,
        })
    }

    /// The pre-tessellated meshes for `icon`.
    pub fn tessellation(&self, icon: IconId) -> &IconTessellation {
        match icon {
            IconId::Check => &self.check,
            IconId::CircleMarker => &self.circle_marker,
            IconId::ConnectionLost => &self.connection_lost,
            IconId::Cross => &self.cross,
            IconId::Download => &self.download,
            IconId::Error => &self.error,
            IconId::Gear => &self.gear,
            IconId::GhostFix => &self.ghost_fix,
            IconId::Lightning => &self.lightning,
            IconId::LogPin => &self.log_pin,
            IconId::Pin => &self.pin,
            IconId::Refresh => &self.refresh,
            IconId::Satellite => &self.satellite,
            IconId::SatelliteLost => &self.satellite_lost,
            IconId::Upload => &self.upload,
            IconId::Warning => &self.warning,
            IconId::Wrench => &self.wrench,
        }
    }
}

#[cfg(test)]
mod tests {
    use gt_icon_tessellate::{BucketMesh, IconMeshTemplate, SIZE_BUCKETS_PX, TemplateVertex};
    use strum::{EnumCount as _, IntoEnumIterator as _};
    use vec1::Vec1;

    use super::*;

    /// The CI guard for the build-script bake: every icon must decode from
    /// the embedded blob with a mesh for every size bucket.
    #[test]
    fn embedded_meshes_decode_for_every_icon() {
        let library = IconMeshLibrary::embedded().unwrap();
        for icon in IconId::iter() {
            let tessellation = library.tessellation(icon);
            assert_eq!(
                tessellation.buckets().len(),
                SIZE_BUCKETS_PX.len(),
                "{icon}: bucket count"
            );
            assert!(
                tessellation
                    .buckets()
                    .iter()
                    .all(|bucket| !bucket.mesh.indices.is_empty()),
                "{icon}: empty bucket mesh"
            );
        }
    }

    #[test]
    fn wire_names_are_stable() {
        let expected = [
            (IconId::Check, "check"),
            (IconId::CircleMarker, "circle_marker"),
            (IconId::ConnectionLost, "connection_lost"),
            (IconId::Cross, "cross"),
            (IconId::Download, "download"),
            (IconId::Error, "error"),
            (IconId::Gear, "gear"),
            (IconId::GhostFix, "ghost_fix"),
            (IconId::Lightning, "lightning"),
            (IconId::LogPin, "log_pin"),
            (IconId::Pin, "pin"),
            (IconId::Refresh, "refresh"),
            (IconId::Satellite, "satellite"),
            (IconId::SatelliteLost, "satellite_lost"),
            (IconId::Upload, "upload"),
            (IconId::Warning, "warning"),
            (IconId::Wrench, "wrench"),
        ];
        assert_eq!(expected.len(), IconId::COUNT);
        for (icon, name) in expected {
            assert_eq!(icon.to_string(), name);
            assert_eq!(name.parse::<IconId>().unwrap(), icon);
        }
    }

    #[test]
    fn every_marker_icon_maps_to_its_icon_id() {
        let expected = [
            (MarkerIcon::Pin, IconId::Pin),
            (MarkerIcon::Cross, IconId::Cross),
            (MarkerIcon::Circle, IconId::CircleMarker),
            (MarkerIcon::Lightning, IconId::Lightning),
            (MarkerIcon::Warning, IconId::Warning),
            (MarkerIcon::Error, IconId::Error),
            (MarkerIcon::Check, IconId::Check),
            (MarkerIcon::Log, IconId::LogPin),
            (MarkerIcon::Satellite, IconId::Satellite),
            (MarkerIcon::SatelliteLost, IconId::SatelliteLost),
            (MarkerIcon::Gear, IconId::Gear),
            (MarkerIcon::Refresh, IconId::Refresh),
            (MarkerIcon::Download, IconId::Download),
            (MarkerIcon::Upload, IconId::Upload),
            (MarkerIcon::Wrench, IconId::Wrench),
        ];
        assert_eq!(expected.len(), MarkerIcon::iter().count());
        for (marker, icon) in expected {
            assert_eq!(IconId::from(marker), icon);
        }
    }

    /// A minimal placeholder tessellation for exercising the decode errors.
    fn dummy_tessellation() -> IconTessellation {
        IconTessellation::new(Vec1::new(BucketMesh {
            bucket_px: 4.0,
            mesh: IconMeshTemplate {
                vertices: vec![TemplateVertex {
                    pos: [0.0, 0.0],
                    color: [0, 0, 0, 255],
                }],
                indices: Vec::new(),
            },
        }))
    }

    fn encode(entries: &[(String, IconTessellation)]) -> Vec<u8> {
        postcard::to_allocvec(&entries).unwrap()
    }

    #[test]
    fn missing_icon_is_rejected() {
        let bytes = encode(&[("check".to_owned(), dummy_tessellation())]);
        assert!(matches!(
            IconMeshLibrary::decode(&bytes),
            Err(IconMeshLibraryError::MissingIcon { .. })
        ));
    }

    #[test]
    fn unknown_icon_is_rejected() {
        let bytes = encode(&[("bogus".to_owned(), dummy_tessellation())]);
        assert!(matches!(
            IconMeshLibrary::decode(&bytes),
            Err(IconMeshLibraryError::UnknownIcon { name }) if name == "bogus"
        ));
    }

    #[test]
    fn duplicate_icon_is_rejected() {
        let bytes = encode(&[
            ("check".to_owned(), dummy_tessellation()),
            ("check".to_owned(), dummy_tessellation()),
        ]);
        assert!(matches!(
            IconMeshLibrary::decode(&bytes),
            Err(IconMeshLibraryError::DuplicateIcon {
                icon: IconId::Check
            })
        ));
    }

    #[test]
    fn corrupt_bytes_are_rejected() {
        assert!(matches!(
            IconMeshLibrary::decode(&[0xFF, 0xFF, 0xFF, 0xFF]),
            Err(IconMeshLibraryError::Decode(_))
        ));
    }
}

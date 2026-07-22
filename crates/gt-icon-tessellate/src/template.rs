//! The pre-tessellated icon mesh types shared between the build-time
//! tessellation pipeline and the runtime renderer.

use serde::{Deserialize, Serialize};
use vec1::Vec1;

/// Anti-alias fringe width baked into every template, in physical pixels at
/// the bucket's nominal on-screen size.
///
/// Matches egui's feathering width, and like egui's feathering the ramp is
/// centered on the true edge (the solid geometry is inset by half a feather),
/// so shapes keep their perceived size instead of growing a bright halo.
pub const FEATHER_PX: f32 = 1.0;

/// Physical-pixel icon sizes (full extent of the larger viewbox axis) the
/// templates are baked at, ascending.
///
/// Roughly log-spaced so that scaling an instance to the nearest bucket stays
/// within a factor of about 1.25, bounding both the curve-tolerance error and
/// the anti-alias fringe error.
pub const SIZE_BUCKETS_PX: [f32; 9] = [4.0, 6.0, 8.0, 12.0, 16.0, 24.0, 32.0, 48.0, 64.0];

/// A vertex of a pre-tessellated icon mesh.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct TemplateVertex {
    /// Position in normalized icon space: the SVG viewbox maps to `[-1, 1]`
    /// on both axes.
    ///
    /// A non-square viewbox is stretched, matching how the rasterized icons
    /// were stretched into their square draw rects.
    /// Anti-alias fringe vertices stick out slightly beyond `[-1, 1]`.
    pub pos: [f32; 2],
    /// Straight-alpha sRGB color baked from the SVG paint.
    ///
    /// The outer edge of the anti-alias fringe ramps the alpha down to zero.
    /// Renderers multiply this with a per-instance tint.
    pub color: [u8; 4],
}

/// A triangle mesh for one icon at one size bucket.
///
/// Triangles are wound in SVG paint order, so drawing them with indices in
/// order reproduces the icon's element stacking.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IconMeshTemplate {
    pub vertices: Vec<TemplateVertex>,
    pub indices: Vec<u32>,
}

/// An [IconMeshTemplate] together with the physical-pixel size its curve
/// tolerance and anti-alias fringe were baked for.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BucketMesh {
    pub bucket_px: f32,
    pub mesh: IconMeshTemplate,
}

/// All size buckets of one icon, ascending by [BucketMesh::bucket_px].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IconTessellation {
    buckets: Vec1<BucketMesh>,
}

impl IconTessellation {
    /// Wrap a non-empty list of bucket meshes, ascending by `bucket_px`.
    ///
    /// The ascending order is part of the type's contract, and construction
    /// is a one-time build/decode path, so it is enforced in release too.
    pub fn new(buckets: Vec1<BucketMesh>) -> Self {
        assert!(
            buckets
                .windows(2)
                .all(|pair| matches!(pair, [a, b] if a.bucket_px < b.bucket_px)),
            "bucket meshes must be ascending by bucket_px"
        );
        Self { buckets }
    }

    pub fn buckets(&self) -> &Vec1<BucketMesh> {
        &self.buckets
    }

    /// The template whose bucket is nearest to `target_px` in log space.
    ///
    /// `target_px` is the icon's intended full on-screen extent in physical
    /// pixels (points times `pixels_per_point`).
    pub fn mesh_for(&self, target_px: f32) -> &IconMeshTemplate {
        let target_log = target_px.max(f32::MIN_POSITIVE).ln();
        let mut best = self.buckets.first();
        for candidate in self.buckets.iter().skip(1) {
            let candidate_dist = (candidate.bucket_px.ln() - target_log).abs();
            let best_dist = (best.bucket_px.ln() - target_log).abs();
            if candidate_dist < best_dist {
                best = candidate;
            }
        }
        &best.mesh
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;
    use vec1::Vec1;

    use super::*;

    /// A distinguishable dummy template: `ordinal` many vertices.
    fn dummy_bucket(ordinal: usize, bucket_px: f32) -> BucketMesh {
        BucketMesh {
            bucket_px,
            mesh: IconMeshTemplate {
                vertices: vec![
                    TemplateVertex {
                        pos: [0.0, 0.0],
                        color: [0, 0, 0, 0],
                    };
                    ordinal
                ],
                indices: Vec::new(),
            },
        }
    }

    fn tessellation() -> IconTessellation {
        let buckets: Vec<BucketMesh> = SIZE_BUCKETS_PX
            .iter()
            .enumerate()
            .map(|(ordinal, &px)| dummy_bucket(ordinal, px))
            .collect();
        IconTessellation::new(Vec1::try_from_vec(buckets).unwrap())
    }

    #[rstest]
    #[case::exact_hit(24.0, 5)]
    #[case::log_midpoint_rounds_up(5.0, 1)]
    #[case::between_buckets(20.0, 5)]
    #[case::below_smallest(0.5, 0)]
    #[case::above_largest(200.0, 8)]
    #[case::zero_clamps_to_smallest(0.0, 0)]
    #[case::negative_clamps_to_smallest(-3.0, 0)]
    fn mesh_for_picks_nearest_bucket_in_log_space(
        #[case] target_px: f32,
        #[case] expected_ordinal: usize,
    ) {
        let tess = tessellation();
        let mesh = tess.mesh_for(target_px);
        assert_eq!(mesh.vertices.len(), expected_ordinal);
    }

    #[test]
    fn size_buckets_are_ascending_and_positive() {
        assert!(
            SIZE_BUCKETS_PX
                .windows(2)
                .all(|pair| matches!(pair, [a, b] if 0.0 < *a && a < b))
        );
    }
}

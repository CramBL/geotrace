//! Tessellation of the marker icon SVGs into normalized triangle meshes.
//!
//! The `gt-map` build script bakes every icon into an [IconTessellation] (one
//! [IconMeshTemplate] per size bucket) that the app renders at runtime as
//! instanced vertex-colored meshes.
//! Because the geometry is vector-tessellated with an egui-style anti-alias
//! fringe, icons stay crisp at any draw size, `pixels_per_point`, and zoom.
//!
//! By default this crate carries only the template types plus their serde
//! representation.
//! The `tessellate` feature (on by default, disabled by the `gt-map` runtime
//! dependency) adds the [tessellate] module with the actual SVG-to-mesh
//! pipeline on top of usvg and lyon.

mod template;
#[cfg(feature = "tessellate")]
pub mod tessellate;

pub use template::{
    BucketMesh, FEATHER_PX, IconMeshTemplate, IconTessellation, SIZE_BUCKETS_PX, TemplateVertex,
};
#[cfg(feature = "tessellate")]
pub use tessellate::IconTessellateError;

//! The SVG-to-mesh tessellation pipeline.
//!
//! [tessellate_icon] parses an SVG with usvg, walks its flattened tree in
//! paint order, and tessellates every fill and stroke with lyon into one
//! [IconMeshTemplate] per size bucket, each with a baked anti-alias fringe
//! (see the private `fringe` submodule).
//!
//! Only the subset of SVG used by the icon assets is supported: plain-color
//! fills and strokes, groups without opacity/clip/mask/filter, no text or
//! images.
//! Anything else is a hard error so a bad asset fails the build instead of
//! rendering wrong.

mod fringe;

use lyon_tessellation::math::Point;
use lyon_tessellation::path::Path as LyonPath;
use lyon_tessellation::{
    BuffersBuilder, FillOptions, FillRule, FillTessellator, FillVertex, LineCap, LineJoin,
    StrokeOptions, StrokeTessellator, StrokeVertex, TessellationError, VertexBuffers,
};
use usvg::tiny_skia_path::{Path as SkiaPath, PathSegment};
use vec1::Vec1;

use crate::template::{
    BucketMesh, FEATHER_PX, IconMeshTemplate, IconTessellation, SIZE_BUCKETS_PX, TemplateVertex,
};

/// Curve flattening tolerance, in bucket pixels.
///
/// Applied after the per-bucket scale in [bucket_mesh], i.e. in already-scaled
/// bucket-pixel space, so the same absolute tolerance yields proportionally
/// finer curves for larger buckets.
const TOLERANCE_PX: f32 = 0.1;

#[derive(Debug, thiserror::Error)]
pub enum IconTessellateError {
    #[error("failed to parse SVG")]
    SvgParse(#[from] usvg::Error),
    #[error("unsupported SVG node kind {kind:?}: only groups and paths are supported")]
    UnsupportedNode { kind: &'static str },
    #[error("unsupported paint: only plain colors are supported")]
    UnsupportedPaint,
    #[error("unsupported group: opacity, clip paths, masks, and filters are not supported")]
    UnsupportedGroup,
    #[error("dashed strokes are not supported")]
    DashedStroke,
    #[error("path tessellation failed: {0:?}")]
    Tessellation(TessellationError),
    #[error("tessellated mesh has a non-manifold boundary, cannot build the anti-alias fringe")]
    FringeBoundary,
    #[error("mesh exceeds the u32 index range")]
    TooManyVertices,
}

/// One paint operation (a fill or a stroke) of one SVG path, in paint order.
struct Element {
    path: SkiaPath,
    abs_transform: usvg::Transform,
    color: [u8; 4],
    tint_slot: u8,
    op: PaintOp,
}

/// The SVG `id` that assigns an element's vertices to the secondary tint
/// slot; every other element uses the primary slot. See
/// [crate::TemplateVertex::tint_slot].
const SECONDARY_TINT_ID: &str = "tint2";

enum PaintOp {
    Fill { rule: FillRule },
    Stroke(StrokeStyle),
}

struct StrokeStyle {
    /// Width as written in the SVG; its meaning is determined per bake by
    /// [StrokeWidthUnit] (see [stroke_width_px]).
    width: f32,
    cap: LineCap,
    join: LineJoin,
    miter_limit: f32,
}

/// How an SVG's `stroke-width` values are interpreted when baking.
///
/// SVG's own mechanism for screen-constant strokes
/// (`vector-effect="non-scaling-stroke"`) is parsed but not resolved by usvg,
/// so the interpretation is chosen per asset at the bake site instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StrokeWidthUnit {
    /// User units: strokes scale with the icon, like any other geometry.
    #[default]
    UserUnits,
    /// Physical pixels: strokes keep the same on-screen width at every
    /// bucket, matching painter-drawn outlines whose width is in pixels
    /// rather than a fraction of the glyph.
    PhysicalPixels,
}

/// Tessellate one SVG icon into a mesh per size bucket in [SIZE_BUCKETS_PX].
pub fn tessellate_icon(svg_bytes: &[u8]) -> Result<IconTessellation, IconTessellateError> {
    tessellate_icon_with(svg_bytes, StrokeWidthUnit::default())
}

/// [tessellate_icon] with an explicit stroke-width interpretation.
pub fn tessellate_icon_with(
    svg_bytes: &[u8],
    stroke_width_unit: StrokeWidthUnit,
) -> Result<IconTessellation, IconTessellateError> {
    let tree = usvg::Tree::from_data(svg_bytes, &usvg::Options::default())?;
    let mut elements = Vec::new();
    collect_elements(tree.root(), &mut elements)?;
    let (svg_w, svg_h) = (tree.size().width(), tree.size().height());

    let [first_bucket, rest @ ..] = SIZE_BUCKETS_PX;
    let mut buckets = Vec1::new(BucketMesh {
        bucket_px: first_bucket,
        mesh: bucket_mesh(&elements, svg_w, svg_h, first_bucket, stroke_width_unit)?,
    });
    for bucket_px in rest {
        buckets.push(BucketMesh {
            bucket_px,
            mesh: bucket_mesh(&elements, svg_w, svg_h, bucket_px, stroke_width_unit)?,
        });
    }
    Ok(IconTessellation::new(buckets))
}

fn collect_elements(
    group: &usvg::Group,
    out: &mut Vec<Element>,
) -> Result<(), IconTessellateError> {
    ensure_plain_group(group)?;
    for node in group.children() {
        match node {
            usvg::Node::Group(child) => collect_elements(child, out)?,
            usvg::Node::Path(path) => collect_path(path, out)?,
            usvg::Node::Image(_) => {
                return Err(IconTessellateError::UnsupportedNode { kind: "image" });
            }
            usvg::Node::Text(_) => {
                return Err(IconTessellateError::UnsupportedNode { kind: "text" });
            }
        }
    }
    Ok(())
}

fn ensure_plain_group(group: &usvg::Group) -> Result<(), IconTessellateError> {
    let plain = group.opacity() == usvg::Opacity::ONE
        && group.clip_path().is_none()
        && group.mask().is_none()
        && group.filters().is_empty();
    if plain {
        Ok(())
    } else {
        Err(IconTessellateError::UnsupportedGroup)
    }
}

fn collect_path(path: &usvg::Path, out: &mut Vec<Element>) -> Result<(), IconTessellateError> {
    if !path.is_visible() {
        return Ok(());
    }
    let tint_slot = u8::from(path.id() == SECONDARY_TINT_ID);
    let fill = path
        .fill()
        .map(|fill| fill_element(path, fill, tint_slot))
        .transpose()?;
    let stroke = path
        .stroke()
        .map(|stroke| stroke_element(path, stroke, tint_slot))
        .transpose()?;
    let (first, second) = match path.paint_order() {
        usvg::PaintOrder::FillAndStroke => (fill, stroke),
        usvg::PaintOrder::StrokeAndFill => (stroke, fill),
    };
    out.extend(first);
    out.extend(second);
    Ok(())
}

fn fill_element(
    path: &usvg::Path,
    fill: &usvg::Fill,
    tint_slot: u8,
) -> Result<Element, IconTessellateError> {
    let rule = match fill.rule() {
        usvg::FillRule::NonZero => FillRule::NonZero,
        usvg::FillRule::EvenOdd => FillRule::EvenOdd,
    };
    Ok(Element {
        path: path.data().clone(),
        abs_transform: path.abs_transform(),
        color: paint_color(fill.paint(), fill.opacity())?,
        tint_slot,
        op: PaintOp::Fill { rule },
    })
}

fn stroke_element(
    path: &usvg::Path,
    stroke: &usvg::Stroke,
    tint_slot: u8,
) -> Result<Element, IconTessellateError> {
    if stroke.dasharray().is_some() {
        return Err(IconTessellateError::DashedStroke);
    }
    let cap = match stroke.linecap() {
        usvg::LineCap::Butt => LineCap::Butt,
        usvg::LineCap::Round => LineCap::Round,
        usvg::LineCap::Square => LineCap::Square,
    };
    let join = match stroke.linejoin() {
        usvg::LineJoin::Miter => LineJoin::Miter,
        usvg::LineJoin::MiterClip => LineJoin::MiterClip,
        usvg::LineJoin::Round => LineJoin::Round,
        usvg::LineJoin::Bevel => LineJoin::Bevel,
    };
    Ok(Element {
        path: path.data().clone(),
        abs_transform: path.abs_transform(),
        color: paint_color(stroke.paint(), stroke.opacity())?,
        tint_slot,
        op: PaintOp::Stroke(StrokeStyle {
            width: stroke.width().get(),
            cap,
            join,
            miter_limit: stroke.miterlimit().get(),
        }),
    })
}

fn paint_color(
    paint: &usvg::Paint,
    opacity: usvg::Opacity,
) -> Result<[u8; 4], IconTessellateError> {
    let usvg::Paint::Color(color) = paint else {
        return Err(IconTessellateError::UnsupportedPaint);
    };
    let alpha = (opacity.get() * 255.0).round() as i32;
    let alpha = u8::try_from(alpha).unwrap_or(u8::MAX);
    Ok([color.red, color.green, color.blue, alpha])
}

/// Tessellate all elements at one bucket size and normalize the result so the
/// viewbox maps to `[-1, 1]` on both axes.
fn bucket_mesh(
    elements: &[Element],
    svg_w: f32,
    svg_h: f32,
    bucket_px: f32,
    stroke_width_unit: StrokeWidthUnit,
) -> Result<IconMeshTemplate, IconTessellateError> {
    let scale = bucket_px / svg_w.max(svg_h);
    let mut mesh = IconMeshTemplate {
        vertices: Vec::new(),
        indices: Vec::new(),
    };
    for element in elements {
        let path = to_lyon_path(&element.path, element.abs_transform, scale);
        let mut buffers = tessellate_element(&path, &element.op, scale, stroke_width_unit)?;
        let ramp = edge_ramp(&element.op, scale, stroke_width_unit);
        let color = premultiply(scale_alpha(element.color, ramp.alpha_factor));
        // The fringe pass insets the solid boundary vertices, so it runs
        // before the solid vertices are copied out. Its inner ring
        // references the solid vertices by index, so it needs to know where
        // this element's solid block lands in the mesh.
        let solid_base = u32::try_from(mesh.vertices.len())
            .map_err(|_overflow| IconTessellateError::TooManyVertices)?;
        let fringe = fringe::fringe_mesh(
            &mut buffers.vertices,
            &buffers.indices,
            ramp.inset_px,
            ramp.outset_px,
            solid_base,
            element.tint_slot,
        )?;
        let solid: Vec<TemplateVertex> = buffers
            .vertices
            .iter()
            .map(|&pos| TemplateVertex {
                pos,
                color,
                tint_slot: element.tint_slot,
            })
            .collect();
        append_indexed(&mut mesh, solid, &buffers.indices)?;
        // Fringe indices are already in final mesh space (solid refs plus
        // outer-ring refs starting at the current length).
        debug_assert_eq!(
            mesh.vertices.len(),
            fringe.outer_base as usize,
            "fringe outer ring must start where the solid block ends"
        );
        mesh.vertices.extend(fringe.outer_vertices);
        mesh.indices.extend(fringe.indices);
    }
    normalize(&mut mesh, svg_w * scale, svg_h * scale);
    Ok(mesh)
}

/// Convert a straight-alpha sRGB color to premultiplied form, the encoding
/// egui vertex colors use, so renderers can copy template colors without a
/// per-vertex conversion.
fn premultiply(color: [u8; 4]) -> [u8; 4] {
    let [r, g, b, a] = color;
    let mul = |channel: u8| ((u16::from(channel) * u16::from(a) + 127) / 255) as u8;
    [mul(r), mul(g), mul(b), a]
}

/// How an element's edge is anti-aliased: the solid geometry is inset by
/// `inset_px` and the alpha ramp spans from there to `outset_px` outside the
/// true edge, centering the perceived edge on the outline.
struct EdgeRamp {
    inset_px: f32,
    outset_px: f32,
    /// Alpha multiplier for the whole element: strokes thinner than one
    /// feather dim instead of fattening into blobs, like epaint's thin lines.
    alpha_factor: f32,
}

fn edge_ramp(op: &PaintOp, scale: f32, stroke_width_unit: StrokeWidthUnit) -> EdgeRamp {
    let half_feather = FEATHER_PX / 2.0;
    let symmetric = EdgeRamp {
        inset_px: half_feather,
        outset_px: half_feather,
        alpha_factor: 1.0,
    };
    match op {
        PaintOp::Fill { .. } => symmetric,
        PaintOp::Stroke(style) => {
            let width_px = stroke_width_px(style.width, scale, stroke_width_unit);
            if width_px >= FEATHER_PX {
                symmetric
            } else {
                // A half-feather inset would collapse a sub-feather stroke;
                // inset only a quarter of its width, keep the ramp one
                // feather wide, and trade the lost coverage for lower alpha.
                let inset_px = width_px / 4.0;
                EdgeRamp {
                    inset_px,
                    outset_px: FEATHER_PX - inset_px,
                    alpha_factor: width_px / FEATHER_PX,
                }
            }
        }
    }
}

fn scale_alpha(color: [u8; 4], factor: f32) -> [u8; 4] {
    let [r, g, b, a] = color;
    let scaled = (f32::from(a) * factor).round();
    [r, g, b, u8::try_from(scaled as i32).unwrap_or(u8::MAX)]
}

/// An element stroke's on-screen width at one bucket.
fn stroke_width_px(svg_width: f32, scale: f32, unit: StrokeWidthUnit) -> f32 {
    match unit {
        StrokeWidthUnit::UserUnits => svg_width * scale,
        StrokeWidthUnit::PhysicalPixels => svg_width,
    }
}

fn tessellate_element(
    path: &LyonPath,
    op: &PaintOp,
    scale: f32,
    stroke_width_unit: StrokeWidthUnit,
) -> Result<VertexBuffers<[f32; 2], u32>, IconTessellateError> {
    let mut buffers = VertexBuffers::new();
    match op {
        PaintOp::Fill { rule } => {
            FillTessellator::new()
                .tessellate_path(
                    path,
                    &FillOptions::tolerance(TOLERANCE_PX).with_fill_rule(*rule),
                    &mut BuffersBuilder::new(&mut buffers, |vertex: FillVertex<'_>| {
                        vertex.position().to_array()
                    }),
                )
                .map_err(IconTessellateError::Tessellation)?;
        }
        PaintOp::Stroke(style) => {
            StrokeTessellator::new()
                .tessellate_path(
                    path,
                    &StrokeOptions::tolerance(TOLERANCE_PX)
                        .with_line_width(stroke_width_px(style.width, scale, stroke_width_unit))
                        .with_line_cap(style.cap)
                        .with_line_join(style.join)
                        .with_miter_limit(style.miter_limit),
                    &mut BuffersBuilder::new(&mut buffers, |vertex: StrokeVertex<'_, '_>| {
                        vertex.position().to_array()
                    }),
                )
                .map_err(IconTessellateError::Tessellation)?;
        }
    }
    Ok(buffers)
}

/// Convert a tiny-skia path to a lyon path, applying the node's absolute
/// transform and the bucket scale.
fn to_lyon_path(path: &SkiaPath, transform: usvg::Transform, scale: f32) -> LyonPath {
    let map = |mut p: usvg::tiny_skia_path::Point| -> Point {
        transform.map_point(&mut p);
        Point::new(p.x * scale, p.y * scale)
    };
    let mut builder = LyonPath::builder();
    let mut open = false;
    for segment in path.segments() {
        match segment {
            PathSegment::MoveTo(p) => {
                if open {
                    builder.end(false);
                }
                builder.begin(map(p));
                open = true;
            }
            PathSegment::LineTo(p) => {
                if open {
                    builder.line_to(map(p));
                }
            }
            PathSegment::QuadTo(ctrl, to) => {
                if open {
                    builder.quadratic_bezier_to(map(ctrl), map(to));
                }
            }
            PathSegment::CubicTo(ctrl1, ctrl2, to) => {
                if open {
                    builder.cubic_bezier_to(map(ctrl1), map(ctrl2), map(to));
                }
            }
            PathSegment::Close => {
                if open {
                    builder.end(true);
                }
                open = false;
            }
        }
    }
    if open {
        builder.end(false);
    }
    builder.build()
}

/// Append `vertices` and their local `indices` to `mesh`, offsetting indices.
fn append_indexed(
    mesh: &mut IconMeshTemplate,
    vertices: Vec<TemplateVertex>,
    indices: &[u32],
) -> Result<(), IconTessellateError> {
    let base = u32::try_from(mesh.vertices.len())
        .map_err(|_overflow| IconTessellateError::TooManyVertices)?;
    mesh.vertices.extend(vertices);
    for &index in indices {
        let offset = base
            .checked_add(index)
            .ok_or(IconTessellateError::TooManyVertices)?;
        mesh.indices.push(offset);
    }
    Ok(())
}

/// Map bucket-pixel positions into normalized icon space: each viewbox axis
/// to `[-1, 1]` (non-square viewboxes stretch, see [TemplateVertex::pos]).
fn normalize(mesh: &mut IconMeshTemplate, extent_x_px: f32, extent_y_px: f32) {
    let half_x = extent_x_px / 2.0;
    let half_y = extent_y_px / 2.0;
    for vertex in &mut mesh.vertices {
        let [x, y] = vertex.pos;
        vertex.pos = [(x - half_x) / half_x, (y - half_y) / half_y];
    }
}

#[cfg(test)]
mod tests {
    use std::fmt::Write as _;
    use std::fs;
    use std::path::PathBuf;

    use rstest::rstest;

    use super::*;
    use crate::template::FEATHER_PX;

    /// Every icon asset, sorted. Kept in sync with `assets/icons/` by
    /// [icon_names_match_assets_dir]; the rstest cases below must mirror it.
    const ICON_NAMES: [&str; 18] = [
        "check",
        "circle_marker",
        "connection_lost",
        "cross",
        "download",
        "error",
        "gear",
        "ghost_fix",
        "lightning",
        "log_pin",
        "nav_arrow",
        "pin",
        "refresh",
        "satellite",
        "satellite_lost",
        "upload",
        "warning",
        "wrench",
    ];

    fn icons_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/icons")
    }

    fn icon_svg(name: &str) -> Vec<u8> {
        fs::read(icons_dir().join(format!("{name}.svg"))).unwrap()
    }

    #[test]
    fn icon_names_match_assets_dir() {
        let mut stems: Vec<String> = fs::read_dir(icons_dir())
            .unwrap()
            .map(|entry| {
                let path = entry.unwrap().path();
                assert_eq!(
                    path.extension().and_then(|ext| ext.to_str()),
                    Some("svg"),
                    "unexpected non-SVG file in assets/icons: {path:?}"
                );
                path.file_stem().unwrap().to_str().unwrap().to_owned()
            })
            .collect();
        stems.sort();
        assert_eq!(
            stems, ICON_NAMES,
            "assets/icons drifted: update ICON_NAMES and the rstest cases in this module"
        );
    }

    fn mesh_stats(tess: &IconTessellation) -> String {
        let mut stats = String::new();
        for bucket in tess.buckets() {
            let mesh = &bucket.mesh;
            let fringe_vertices = mesh
                .vertices
                .iter()
                .filter(|vertex| vertex.color[3] == 0)
                .count();
            let (mut min_x, mut min_y, mut max_x, mut max_y) =
                (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
            for vertex in &mesh.vertices {
                let [x, y] = vertex.pos;
                min_x = min_x.min(x);
                min_y = min_y.min(y);
                max_x = max_x.max(x);
                max_y = max_y.max(y);
            }
            writeln!(
                stats,
                "bucket {:>4}: vertices {:>5}, triangles {:>5}, fringe vertices {:>5}, bbox [{min_x:.2} {min_y:.2} {max_x:.2} {max_y:.2}]",
                bucket.bucket_px,
                mesh.vertices.len(),
                mesh.indices.len() / 3,
                fringe_vertices,
            )
            .unwrap();
        }
        stats
    }

    #[rstest]
    #[case::check("check")]
    #[case::circle_marker("circle_marker")]
    #[case::connection_lost("connection_lost")]
    #[case::cross("cross")]
    #[case::download("download")]
    #[case::error("error")]
    #[case::gear("gear")]
    #[case::ghost_fix("ghost_fix")]
    #[case::lightning("lightning")]
    #[case::log_pin("log_pin")]
    #[case::nav_arrow("nav_arrow")]
    #[case::pin("pin")]
    #[case::refresh("refresh")]
    #[case::satellite("satellite")]
    #[case::satellite_lost("satellite_lost")]
    #[case::upload("upload")]
    #[case::warning("warning")]
    #[case::wrench("wrench")]
    fn tessellation_stats_are_stable(#[case] name: &str) {
        let tess = tessellate_icon(&icon_svg(name)).unwrap();
        insta::assert_snapshot!(format!("stats_{name}"), mesh_stats(&tess));
    }

    #[rstest]
    #[case::check("check")]
    #[case::circle_marker("circle_marker")]
    #[case::connection_lost("connection_lost")]
    #[case::cross("cross")]
    #[case::download("download")]
    #[case::error("error")]
    #[case::gear("gear")]
    #[case::ghost_fix("ghost_fix")]
    #[case::lightning("lightning")]
    #[case::log_pin("log_pin")]
    #[case::nav_arrow("nav_arrow")]
    #[case::pin("pin")]
    #[case::refresh("refresh")]
    #[case::satellite("satellite")]
    #[case::satellite_lost("satellite_lost")]
    #[case::upload("upload")]
    #[case::warning("warning")]
    #[case::wrench("wrench")]
    fn mesh_invariants_hold(#[case] name: &str) {
        let tess = tessellate_icon(&icon_svg(name)).unwrap();
        for bucket in tess.buckets() {
            let mesh = &bucket.mesh;
            let label = format!("{name} at bucket {}", bucket.bucket_px);

            assert!(!mesh.indices.is_empty(), "{label}: empty mesh");
            assert_eq!(mesh.indices.len() % 3, 0, "{label}: dangling indices");
            let vertex_count = mesh.vertices.len() as u32;
            assert!(
                mesh.indices.iter().all(|&index| index < vertex_count),
                "{label}: index out of range"
            );
            assert!(
                mesh.vertices
                    .iter()
                    .all(|vertex| vertex.pos.iter().all(|coord| coord.is_finite())),
                "{label}: non-finite vertex position"
            );

            // Both solid geometry and a fringe must be present. Solid alpha
            // can sit below 255 at small buckets, where sub-feather strokes
            // are dimmed instead of fattened.
            assert!(
                mesh.vertices.iter().any(|vertex| vertex.color[3] == 0),
                "{label}: no fringe vertices"
            );
            assert!(
                mesh.vertices.iter().any(|vertex| vertex.color[3] > 0),
                "{label}: no solid vertices"
            );

            // Loose sanity bound: the viewbox is [-1, 1], icons may overhang
            // it by design (edge strokes, round caps: wrench reaches ~1.13),
            // and the mitered fringe adds up to ~6 feather widths after the
            // smaller-axis stretch. Catches runaway transforms, not pixels.
            let allowance = 1.25 + 6.0 * FEATHER_PX / bucket.bucket_px;
            assert!(
                mesh.vertices
                    .iter()
                    .all(|vertex| vertex.pos.iter().all(|coord| coord.abs() <= allowance)),
                "{label}: vertex outside the expected bounds"
            );
        }
    }

    /// Loop instead of rstest cases: this is an exhaustiveness check over all
    /// icons at once, not a per-icon scenario.
    #[test]
    fn tessellation_is_deterministic() {
        for name in ICON_NAMES {
            let svg = icon_svg(name);
            let first = tessellate_icon(&svg).unwrap();
            let second = tessellate_icon(&svg).unwrap();
            assert_eq!(first, second, "{name}: non-deterministic tessellation");
        }
    }

    #[test]
    fn template_postcard_roundtrip() {
        let tess = tessellate_icon(&icon_svg("pin")).unwrap();
        let bytes = postcard::to_allocvec(&tess).unwrap();
        let decoded: IconTessellation = postcard::from_bytes(&bytes).unwrap();
        assert_eq!(tess, decoded);
    }

    /// A single horizontal stroke through the viewbox center: its normalized
    /// y extent is the stroke's half thickness, isolating the width behavior.
    const HORIZONTAL_STROKE_SVG: &[u8] =
        br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24">
  <line x1="4" y1="12" x2="20" y2="12" stroke="white" stroke-width="4"/>
</svg>"#;

    fn normalized_half_thickness(tess: &IconTessellation, bucket_px: f32) -> f32 {
        tess.mesh_for(bucket_px)
            .vertices
            .iter()
            .map(|vertex| vertex.pos[1].abs())
            .fold(0.0_f32, f32::max)
    }

    #[test]
    fn stroke_width_units_scale_as_documented() {
        let user = tessellate_icon_with(HORIZONTAL_STROKE_SVG, StrokeWidthUnit::UserUnits).unwrap();
        let physical =
            tessellate_icon_with(HORIZONTAL_STROKE_SVG, StrokeWidthUnit::PhysicalPixels).unwrap();

        // User units: the stroke is a constant fraction of the glyph, so the
        // normalized thickness is bucket-independent (up to the fringe).
        let user_small = normalized_half_thickness(&user, 24.0);
        let user_large = normalized_half_thickness(&user, 64.0);
        assert!(
            (user_small - user_large).abs() < 0.05,
            "user-unit thickness drifted: {user_small} vs {user_large}"
        );

        // Physical pixels: the on-screen width is constant, so the
        // normalized thickness shrinks as the bucket grows (24 -> 64 px is
        // a 8/3 factor; assert well past the halfway point to stay robust
        // against the fringe).
        let physical_small = normalized_half_thickness(&physical, 24.0);
        let physical_large = normalized_half_thickness(&physical, 64.0);
        assert!(
            physical_small > physical_large * 1.8,
            "physical-pixel thickness did not stay constant on screen: \
             {physical_small} vs {physical_large}"
        );
        // At the bucket where one SVG user unit is one pixel the two modes
        // must agree exactly.
        let user_at_unity = normalized_half_thickness(&user, 24.0);
        let physical_at_unity = normalized_half_thickness(&physical, 24.0);
        assert!(
            (user_at_unity - physical_at_unity).abs() < 1e-6,
            "modes must coincide at scale 1: {user_at_unity} vs {physical_at_unity}"
        );
    }
}

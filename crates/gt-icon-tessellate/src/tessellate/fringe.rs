//! Anti-alias fringe construction.
//!
//! egui gets smooth edges by tessellating a thin "feathering" strip along
//! every shape outline whose alpha ramps to zero.
//! Pre-tessellated meshes bypass egui's tessellator, so the same fringe is
//! baked here: the boundary of the tessellated element is extracted from its
//! triangle topology and extruded outward by [FEATHER_PX].
//!
//! Boundary edges are directed edges that appear in exactly one triangle.
//! Each one remembers the opposite vertex of its triangle, which sits on the
//! interior side, so the outward direction is exact and local: no winding
//! conventions, point-in-polygon tests, or hole special cases are needed.

use std::collections::BTreeMap;

use super::IconTessellateError;
use crate::template::TemplateVertex;

/// Fringe geometry in final mesh index space.
///
/// The ramp's inner ring is the element's inset solid boundary itself,
/// referenced by index (`solid_base + boundary vertex index`) instead of
/// duplicated, so the fringe only adds the transparent outer ring.
pub(super) struct FringeMesh {
    pub outer_vertices: Vec<TemplateVertex>,
    /// Triangle indices in final mesh space: inner refs point into the
    /// element's solid block, outer refs start at [FringeMesh::outer_base].
    pub indices: Vec<u32>,
    /// Where the caller must append [FringeMesh::outer_vertices]:
    /// immediately after the element's solid block.
    pub outer_base: u32,
}

/// Build the anti-alias fringe for one tessellated element, insetting the
/// solid geometry so the alpha ramp straddles the true edge.
///
/// `positions` and `indices` are the element's triangle mesh in bucket-pixel
/// space; `solid_base` is where the element's solid vertices will land in
/// the final mesh.
/// Boundary vertices in `positions` are moved `inset_px` inward, and the
/// fringe ramps from full alpha there to zero at `outset_px` outside the
/// original edge.
/// Centering the ramp like this (egui's feathering does the same) keeps the
/// perceived edge on the true outline; a ramp that only grows outward makes
/// thin strokes read noticeably fatter.
pub(super) fn fringe_mesh(
    positions: &mut [[f32; 2]],
    indices: &[u32],
    inset_px: f32,
    outset_px: f32,
    solid_base: u32,
    tint_slot: u8,
) -> Result<FringeMesh, IconTessellateError> {
    let solid_len =
        u32::try_from(positions.len()).map_err(|_overflow| IconTessellateError::TooManyVertices)?;
    let outer_base = solid_base
        .checked_add(solid_len)
        .ok_or(IconTessellateError::TooManyVertices)?;
    let mut fringe = FringeMesh {
        outer_vertices: Vec::new(),
        indices: Vec::new(),
        outer_base,
    };
    // Loops never share vertices (each boundary vertex has exactly one
    // outgoing boundary edge), so per-loop insetting cannot double-move one.
    for ring in boundary_loops(indices)? {
        append_ring_strip(
            &ring,
            positions,
            inset_px,
            outset_px,
            solid_base,
            tint_slot,
            &mut fringe,
        )?;
    }
    Ok(fringe)
}

/// A directed boundary edge's payload: the edge's end vertex and the opposite
/// (interior-side) vertex of the triangle it came from.
#[derive(Clone, Copy)]
struct BoundaryEdge {
    to: u32,
    opposite: u32,
}

/// Extract the boundary of the triangle mesh as closed vertex loops.
///
/// Each loop entry pairs a vertex with the opposite vertex of its outgoing
/// boundary edge.
fn boundary_loops(indices: &[u32]) -> Result<Vec<Vec<(u32, BoundaryEdge)>>, IconTessellateError> {
    // Count directed edges; interior edges appear once per direction.
    let mut edges: BTreeMap<(u32, u32), (u32, u32)> = BTreeMap::new();
    for triangle in indices.chunks_exact(3) {
        let &[a, b, c] = triangle else { continue };
        if a == b || b == c || a == c {
            continue;
        }
        for (from, to, opposite) in [(a, b, c), (b, c, a), (c, a, b)] {
            let entry = edges.entry((from, to)).or_insert((0, opposite));
            entry.0 += 1;
        }
    }

    // Boundary edge: no reverse partner. A duplicated boundary edge or a
    // vertex with two outgoing boundary edges means the topology is not a set
    // of simple loops.
    let mut successors: BTreeMap<u32, BoundaryEdge> = BTreeMap::new();
    for (&(from, to), &(count, opposite)) in &edges {
        if edges.contains_key(&(to, from)) {
            continue;
        }
        if count != 1 {
            return Err(IconTessellateError::FringeBoundary);
        }
        let edge = BoundaryEdge { to, opposite };
        if successors.insert(from, edge).is_some() {
            return Err(IconTessellateError::FringeBoundary);
        }
    }

    let mut loops = Vec::new();
    while let Some((&start, _)) = successors.first_key_value() {
        let mut ring = Vec::new();
        let mut current = start;
        loop {
            let Some(edge) = successors.remove(&current) else {
                return Err(IconTessellateError::FringeBoundary);
            };
            ring.push((current, edge));
            current = edge.to;
            if current == start {
                break;
            }
        }
        if ring.len() >= 3 {
            loops.push(ring);
        }
    }
    Ok(loops)
}

/// Outward unit normal of the boundary edge `a -> b`, i.e. pointing away from
/// the triangle's `opposite` vertex. `None` for degenerate edges.
fn edge_outward_normal(a: [f32; 2], b: [f32; 2], opposite: [f32; 2]) -> Option<[f32; 2]> {
    let direction = [b[0] - a[0], b[1] - a[1]];
    let length = direction[0].hypot(direction[1]);
    if length <= f32::EPSILON {
        return None;
    }
    let mut normal = [direction[1] / length, -direction[0] / length];
    let mid = [(a[0] + b[0]) * 0.5, (a[1] + b[1]) * 0.5];
    let toward_opposite = [opposite[0] - mid[0], opposite[1] - mid[1]];
    if normal[0] * toward_opposite[0] + normal[1] * toward_opposite[1] > 0.0 {
        normal = [-normal[0], -normal[1]];
    }
    Some(normal)
}

/// Per-vertex ramp normals: the miter-averaged outward unit normals of the
/// two adjacent boundary edges, with miter growth capped at 2x for sharp
/// corners. Scaled by the caller's inset/outset widths.
fn ring_normals(
    ring: &[(u32, BoundaryEdge)],
    positions: &[[f32; 2]],
) -> Result<Vec<[f32; 2]>, IconTessellateError> {
    let position = |index: u32| -> Result<[f32; 2], IconTessellateError> {
        positions
            .get(index as usize)
            .copied()
            .ok_or(IconTessellateError::FringeBoundary)
    };

    let len = ring.len();
    let mut edge_normals = Vec::with_capacity(len);
    for (i, &(from, edge)) in ring.iter().enumerate() {
        let next_index = if i + 1 == len { 0 } else { i + 1 };
        let &(next_from, _) = ring
            .get(next_index)
            .ok_or(IconTessellateError::FringeBoundary)?;
        debug_assert_eq!(edge.to, next_from, "ring edges must chain");
        edge_normals.push(edge_outward_normal(
            position(from)?,
            position(edge.to)?,
            position(edge.opposite)?,
        ));
    }

    let mut normals = Vec::with_capacity(len);
    for i in 0..len {
        let prev_index = if i == 0 { len - 1 } else { i - 1 };
        let incoming = edge_normals.get(prev_index).copied().flatten();
        let outgoing = edge_normals.get(i).copied().flatten();
        let normal = match (incoming, outgoing) {
            (Some(a), Some(b)) => miter_normal(a, b),
            (Some(n), None) | (None, Some(n)) => n,
            (None, None) => [0.0, 0.0],
        };
        normals.push(normal);
    }
    Ok(normals)
}

/// Average two adjacent edge normals into a miter direction whose length
/// compensates the corner angle, capped at 2x.
fn miter_normal(a: [f32; 2], b: [f32; 2]) -> [f32; 2] {
    let sum = [a[0] + b[0], a[1] + b[1]];
    let length = sum[0].hypot(sum[1]);
    if length <= f32::EPSILON {
        // The boundary reverses direction; fall back to one side's normal.
        return b;
    }
    let average = [sum[0] / length, sum[1] / length];
    let cos_half_angle = average[0] * b[0] + average[1] * b[1];
    let scale = 1.0 / cos_half_angle.clamp(0.5, 1.0);
    [average[0] * scale, average[1] * scale]
}

/// Emit the fringe strip for one boundary loop and inset the loop's solid
/// vertices: the inner ring is the moved solid boundary itself (referenced
/// by index, `inset_px` inside the original edge at full alpha) and the
/// outer ring `outset_px` outside at alpha zero.
fn append_ring_strip(
    ring: &[(u32, BoundaryEdge)],
    positions: &mut [[f32; 2]],
    inset_px: f32,
    outset_px: f32,
    solid_base: u32,
    tint_slot: u8,
    fringe: &mut FringeMesh,
) -> Result<(), IconTessellateError> {
    let normals = ring_normals(ring, positions)?;
    let ring_outer_base = fringe
        .outer_base
        .checked_add(
            u32::try_from(fringe.outer_vertices.len())
                .map_err(|_overflow| IconTessellateError::TooManyVertices)?,
        )
        .ok_or(IconTessellateError::TooManyVertices)?;
    // The ramp interpolates premultiplied colors, so fully transparent is
    // all zeros regardless of the paint color.
    let transparent = [0, 0, 0, 0];

    for (&(from, _), normal) in ring.iter().zip(&normals) {
        let pos = positions
            .get(from as usize)
            .copied()
            .ok_or(IconTessellateError::FringeBoundary)?;
        let inner = [pos[0] - normal[0] * inset_px, pos[1] - normal[1] * inset_px];
        let outer = [
            pos[0] + normal[0] * outset_px,
            pos[1] + normal[1] * outset_px,
        ];
        if let Some(solid) = positions.get_mut(from as usize) {
            *solid = inner;
        }
        fringe.outer_vertices.push(TemplateVertex {
            pos: outer,
            color: transparent,
            tint_slot,
        });
    }

    let len =
        u32::try_from(ring.len()).map_err(|_overflow| IconTessellateError::TooManyVertices)?;
    for (i, &(from, edge)) in ring.iter().enumerate() {
        let i = i as u32;
        let j = (i + 1) % len;
        let inner_i = solid_base + from;
        let inner_j = solid_base + edge.to;
        let outer_i = ring_outer_base + i;
        let outer_j = ring_outer_base + j;
        fringe
            .indices
            .extend([inner_i, outer_i, outer_j, inner_i, outer_j, inner_j]);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    type TestMesh = (Vec<[f32; 2]>, Vec<u32>);

    fn single_triangle() -> TestMesh {
        (vec![[0.0, 0.0], [10.0, 0.0], [0.0, 10.0]], vec![0, 1, 2])
    }

    /// A thin sliver whose sharp corner exercises the miter cap.
    fn spike_triangle() -> TestMesh {
        (vec![[0.0, 0.0], [20.0, 0.0], [20.0, 1.0]], vec![0, 1, 2])
    }

    /// A square annulus: outer boundary plus a hole boundary.
    fn annulus() -> TestMesh {
        let positions = vec![
            [0.0, 0.0],
            [10.0, 0.0],
            [10.0, 10.0],
            [0.0, 10.0],
            [3.0, 3.0],
            [7.0, 3.0],
            [7.0, 7.0],
            [3.0, 7.0],
        ];
        let mut indices = Vec::new();
        for i in 0..4u32 {
            let j = (i + 1) % 4;
            let (a, b, c, d) = (i, j, 4 + j, 4 + i);
            indices.extend([a, b, c, a, c, d]);
        }
        (positions, indices)
    }

    fn point_in_mesh(p: [f32; 2], positions: &[[f32; 2]], indices: &[u32]) -> bool {
        indices.chunks_exact(3).any(|triangle| {
            let a = positions[triangle[0] as usize];
            let b = positions[triangle[1] as usize];
            let c = positions[triangle[2] as usize];
            let sign = |p0: [f32; 2], p1: [f32; 2]| {
                (p1[0] - p0[0]) * (p[1] - p0[1]) - (p1[1] - p0[1]) * (p[0] - p0[0])
            };
            let (s0, s1, s2) = (sign(a, b), sign(b, c), sign(c, a));
            (s0 >= 0.0 && s1 >= 0.0 && s2 >= 0.0) || (s0 <= 0.0 && s1 <= 0.0 && s2 <= 0.0)
        })
    }

    #[rstest]
    #[case::single_triangle(single_triangle(), vec![3])]
    #[case::spike_triangle(spike_triangle(), vec![3])]
    #[case::annulus_with_hole(annulus(), vec![4, 4])]
    fn boundary_loops_are_found(#[case] mesh: TestMesh, #[case] expected_sizes: Vec<usize>) {
        let loops = boundary_loops(&mesh.1).unwrap();
        let mut sizes: Vec<usize> = loops.iter().map(|ring| ring.len()).collect();
        sizes.sort_unstable();
        assert_eq!(sizes, expected_sizes);
    }

    const INSET: f32 = 0.5;
    const OUTSET: f32 = 0.5;
    /// Where the element's solid block nominally starts in these tests, so
    /// index-space arithmetic is exercised with a non-zero base.
    const SOLID_BASE: u32 = 100;

    #[rstest]
    #[case::single_triangle(single_triangle())]
    #[case::spike_triangle(spike_triangle())]
    #[case::annulus_with_hole(annulus())]
    fn fringe_strip_invariants(#[case] mesh: TestMesh) {
        let (mut positions, indices) = mesh;
        let originals = positions.clone();
        let fringe = fringe_mesh(&mut positions, &indices, INSET, OUTSET, SOLID_BASE, 0).unwrap();

        assert_eq!(fringe.outer_base, SOLID_BASE + positions.len() as u32);
        assert!(!fringe.outer_vertices.is_empty());
        assert!(
            fringe
                .outer_vertices
                .iter()
                .all(|vertex| vertex.color == [0, 0, 0, 0]),
            "outer ring must be premultiplied transparent"
        );
        // Each boundary edge emits two triangles: one solid (inner) and one
        // outer index per corner, referencing the two index spaces.
        assert_eq!(fringe.indices.len() % 6, 0);

        let ramp = INSET + OUTSET;
        for chunk in fringe.indices.chunks_exact(6) {
            let (inner_i, outer_i) = (chunk[0], chunk[1]);
            assert!(inner_i >= SOLID_BASE && inner_i < fringe.outer_base);
            assert!(outer_i >= fringe.outer_base);
            let inner = positions[(inner_i - SOLID_BASE) as usize];
            let outer = fringe.outer_vertices[(outer_i - fringe.outer_base) as usize].pos;

            let offset = [outer[0] - inner[0], outer[1] - inner[1]];
            let length = offset[0].hypot(offset[1]);
            assert!(
                (ramp * 0.99..=ramp * 2.0 + 1e-4).contains(&length),
                "ramp width {length} outside [ramp, 2x miter cap]"
            );

            // The ramp must straddle the original edge: with inset == outset
            // its midpoint is exactly the original boundary vertex (also at
            // mitered corners, where both ends share the same miter vector),
            // and the outer end lies outside the original mesh.
            // (The inner end can legitimately exit razor-thin geometry like
            // the spike's tip, so only the outer side is asserted.)
            let midpoint = [(inner[0] + outer[0]) / 2.0, (inner[1] + outer[1]) / 2.0];
            assert!(
                originals
                    .iter()
                    .any(|p| (p[0] - midpoint[0]).hypot(p[1] - midpoint[1]) < 1e-3),
                "ramp midpoint {midpoint:?} is off the original boundary"
            );
            assert!(
                !point_in_mesh(outer, &originals, &indices),
                "fringe outer ring reaches into the original mesh at {outer:?}"
            );
        }
    }

    #[test]
    fn non_manifold_duplicate_edge_is_rejected() {
        let mut positions = vec![[0.0, 0.0], [10.0, 0.0], [0.0, 10.0], [10.0, 10.0]];
        // Both triangles use the directed edge (0, 1).
        let indices = vec![0, 1, 2, 0, 1, 3];
        assert!(matches!(
            fringe_mesh(&mut positions, &indices, INSET, OUTSET, 0, 0),
            Err(IconTessellateError::FringeBoundary)
        ));
    }

    #[test]
    fn degenerate_triangles_yield_empty_fringe() {
        let mut positions = vec![[0.0, 0.0], [10.0, 0.0]];
        let indices = vec![0, 1, 1];
        let fringe = fringe_mesh(&mut positions, &indices, INSET, OUTSET, 0, 0).unwrap();
        assert!(fringe.outer_vertices.is_empty());
        assert!(fringe.indices.is_empty());
    }
}

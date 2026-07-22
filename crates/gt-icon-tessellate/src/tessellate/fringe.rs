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
use crate::template::{FEATHER_PX, TemplateVertex};

/// Fringe geometry, with indices local to its own vertex list.
pub(super) struct FringeMesh {
    pub vertices: Vec<TemplateVertex>,
    pub indices: Vec<u32>,
}

/// Build the anti-alias fringe for one tessellated element.
///
/// `positions` and `indices` are the element's triangle mesh in bucket-pixel
/// space, `color` its baked paint color; the fringe ramps that color's alpha
/// from the element's value at the boundary to zero at [FEATHER_PX] outward.
pub(super) fn fringe_mesh(
    positions: &[[f32; 2]],
    indices: &[u32],
    color: [u8; 4],
) -> Result<FringeMesh, IconTessellateError> {
    let mut fringe = FringeMesh {
        vertices: Vec::new(),
        indices: Vec::new(),
    };
    for ring in boundary_loops(indices)? {
        append_ring_strip(&ring, positions, color, &mut fringe)?;
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

/// Per-vertex fringe offsets: the miter-averaged outward normals of the two
/// adjacent boundary edges, scaled to keep the fringe [FEATHER_PX] wide
/// (miter growth capped at 2x for sharp corners).
fn ring_offsets(
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

    let mut offsets = Vec::with_capacity(len);
    for i in 0..len {
        let prev_index = if i == 0 { len - 1 } else { i - 1 };
        let incoming = edge_normals.get(prev_index).copied().flatten();
        let outgoing = edge_normals.get(i).copied().flatten();
        let offset = match (incoming, outgoing) {
            (Some(a), Some(b)) => miter_normal(a, b),
            (Some(n), None) | (None, Some(n)) => n,
            (None, None) => [0.0, 0.0],
        };
        offsets.push([offset[0] * FEATHER_PX, offset[1] * FEATHER_PX]);
    }
    Ok(offsets)
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

/// Emit the fringe strip for one boundary loop: an inner ring of full-alpha
/// vertices on the boundary and an outer ring offset outward with alpha zero.
fn append_ring_strip(
    ring: &[(u32, BoundaryEdge)],
    positions: &[[f32; 2]],
    color: [u8; 4],
    fringe: &mut FringeMesh,
) -> Result<(), IconTessellateError> {
    let offsets = ring_offsets(ring, positions)?;
    let base = u32::try_from(fringe.vertices.len())
        .map_err(|_overflow| IconTessellateError::TooManyVertices)?;
    let transparent = [color[0], color[1], color[2], 0];

    for (&(from, _), offset) in ring.iter().zip(&offsets) {
        let pos = positions
            .get(from as usize)
            .copied()
            .ok_or(IconTessellateError::FringeBoundary)?;
        fringe.vertices.push(TemplateVertex { pos, color });
        fringe.vertices.push(TemplateVertex {
            pos: [pos[0] + offset[0], pos[1] + offset[1]],
            color: transparent,
        });
    }

    let len =
        u32::try_from(ring.len()).map_err(|_overflow| IconTessellateError::TooManyVertices)?;
    for i in 0..len {
        let j = (i + 1) % len;
        let inner_i = base + 2 * i;
        let outer_i = inner_i + 1;
        let inner_j = base + 2 * j;
        let outer_j = inner_j + 1;
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

    #[rstest]
    #[case::single_triangle(single_triangle())]
    #[case::spike_triangle(spike_triangle())]
    #[case::annulus_with_hole(annulus())]
    fn fringe_strip_invariants(#[case] mesh: TestMesh) {
        let (positions, indices) = mesh;
        let color = [10, 20, 30, 255];
        let fringe = fringe_mesh(&positions, &indices, color).unwrap();

        assert!(!fringe.vertices.is_empty());
        assert_eq!(fringe.vertices.len() % 2, 0, "inner/outer pairs");
        assert_eq!(fringe.indices.len() % 3, 0);
        let vertex_count = fringe.vertices.len() as u32;
        assert!(fringe.indices.iter().all(|&index| index < vertex_count));

        for pair in fringe.vertices.chunks_exact(2) {
            let (inner, outer) = (pair[0], pair[1]);
            assert_eq!(inner.color, color);
            assert_eq!(
                outer.color,
                [10, 20, 30, 0],
                "outer edge must be transparent"
            );
            assert!(
                positions.contains(&inner.pos),
                "inner ring must lie on the boundary"
            );

            let offset = [outer.pos[0] - inner.pos[0], outer.pos[1] - inner.pos[1]];
            let length = offset[0].hypot(offset[1]);
            assert!(
                (FEATHER_PX * 0.99..=FEATHER_PX * 2.0 + 1e-4).contains(&length),
                "offset length {length} outside [feather, 2x miter cap]"
            );

            // Probing halfway along the offset must land outside the solid
            // mesh: the fringe points outward, never into the geometry.
            let probe = [
                inner.pos[0] + offset[0] * 0.5,
                inner.pos[1] + offset[1] * 0.5,
            ];
            assert!(
                !point_in_mesh(probe, &positions, &indices),
                "fringe points into the mesh at {:?}",
                inner.pos
            );
        }
    }

    #[test]
    fn non_manifold_duplicate_edge_is_rejected() {
        let positions = vec![[0.0, 0.0], [10.0, 0.0], [0.0, 10.0], [10.0, 10.0]];
        // Both triangles use the directed edge (0, 1).
        let indices = vec![0, 1, 2, 0, 1, 3];
        assert!(matches!(
            fringe_mesh(&positions, &indices, [0, 0, 0, 255]),
            Err(IconTessellateError::FringeBoundary)
        ));
    }

    #[test]
    fn degenerate_triangles_yield_empty_fringe() {
        let positions = vec![[0.0, 0.0], [10.0, 0.0]];
        let indices = vec![0, 1, 1];
        let fringe = fringe_mesh(&positions, &indices, [0, 0, 0, 255]).unwrap();
        assert!(fringe.vertices.is_empty());
        assert!(fringe.indices.is_empty());
    }
}

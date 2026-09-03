//! The per-pass icon instance collector and its CPU mesh backend.
//!
//! Renderers push [IconInstance]s into an [IconMeshBatch], which transforms
//! the pre-tessellated templates (scale, rotation, tint) and appends
//! everything into one untextured [epaint::Mesh].
//! egui merges untextured vertex-colored meshes into its surrounding draw
//! batches, so a whole pass of icons costs no extra draw calls, and the
//! baked anti-alias fringe scales with the geometry.

use egui::epaint;
use egui::{Color32, Pos2, Vec2};

use crate::icon_mesh::gpu::{self, GpuIconInstance, IconDrawCallback, InstanceGroup};
use crate::icon_mesh::{IconId, IconMeshLibrary};

/// One icon to draw this pass.
#[derive(Debug, Clone, Copy)]
pub struct IconInstance {
    pub icon: IconId,
    pub center: Pos2,
    /// Half extents in points: the icon spans `2 * half_extents` around
    /// `center`.
    ///
    /// The template's stretched square viewbox maps to this rect, so extents
    /// matching the SVG's aspect ratio (the pins are 18x24) draw undistorted,
    /// while [Vec2::splat] stretches them into a square.
    pub half_extents: Vec2,
    /// Unit direction the icon's "up" aligns to. `None` draws it upright.
    pub direction: Option<Vec2>,
    /// Per-slot tints multiplied onto the template's baked colors, like a
    /// texture tint: [Color32::WHITE] keeps the SVG colors, alpha fades.
    /// Slot 0 is the default. Slot 1 covers template elements marked
    /// `id="tint2"` in the SVG (the nav arrow's rim). Single-slot icons
    /// simply repeat the tint.
    pub tints: [Color32; 2],
}

/// Collects the icon instances of one renderer pass.
///
/// Create it where the pass starts drawing icons and [IconMeshBatch::paint]
/// it at the same point, so layering against non-icon shapes is unchanged.
/// Passes whose icons interleave with painter primitives call
/// [IconMeshBatch::barrier] before painting each primitive, which flushes
/// the collected icons so stacking is preserved exactly.
///
/// Two backends:
/// - CPU ([IconMeshBatch::new]): instances are transformed into one
///   untextured [epaint::Mesh] that egui merges into its draw batches.
/// - GPU ([IconMeshBatch::gpu_when_available]): instances are collected and
///   flushed as one instanced-draw paint callback per segment (32 bytes per
///   instance). Segments smaller than [gpu::GPU_MIN_INSTANCES] fall back to
///   the CPU mesh, so barrier-heavy zoomed-in frames do not spray tiny draw
///   calls.
///
/// A batch without a library (the embedded meshes failed to decode, reported
/// at startup) accepts pushes and paints nothing, so renderers need no
/// per-call-site `Option` boilerplate.
pub struct IconMeshBatch<'a> {
    library: Option<&'a IconMeshLibrary>,
    pixels_per_point: f32,
    backend: Backend,
}

enum Backend {
    Cpu(epaint::Mesh),
    Gpu(Vec<IconInstance>),
}

impl<'a> IconMeshBatch<'a> {
    /// A CPU-backed batch. `pixels_per_point` (`ui.pixels_per_point()`)
    /// picks each instance's size bucket from its physical on-screen extent.
    pub fn new(library: Option<&'a IconMeshLibrary>, pixels_per_point: f32) -> Self {
        Self {
            library,
            pixels_per_point,
            backend: Backend::Cpu(epaint::Mesh::default()),
        }
    }

    /// A batch that renders through the GPU-instanced pipeline when it is
    /// installed in this context (see [gpu::install]), falling back to the
    /// CPU mesh backend otherwise.
    pub fn gpu_when_available(ui: &egui::Ui, library: Option<&'a IconMeshLibrary>) -> Self {
        let backend = if gpu::is_installed(ui.ctx()) {
            Backend::Gpu(Vec::new())
        } else {
            Backend::Cpu(epaint::Mesh::default())
        };
        Self {
            library,
            pixels_per_point: ui.pixels_per_point(),
            backend,
        }
    }

    pub fn push(&mut self, instance: IconInstance) {
        if self.library.is_none() {
            return;
        }
        match &mut self.backend {
            Backend::Gpu(pending) => pending.push(instance),
            Backend::Cpu(_) => self.push_to_mesh(instance),
        }
    }

    fn push_to_mesh(&mut self, instance: IconInstance) {
        let Some(library) = self.library else {
            return;
        };
        let Backend::Cpu(mesh) = &mut self.backend else {
            return;
        };
        let target_px = physical_extent_px(instance.half_extents, self.pixels_per_point);
        let template = library.tessellation(instance.icon).mesh_for(target_px);

        debug_assert!(
            u32::try_from(mesh.vertices.len() + template.vertices.len()).is_ok(),
            "icon batch exceeds the u32 index range"
        );
        let base = mesh.vertices.len() as u32;
        // Hoist the per-instance work out of the vertex loop: the rotation
        // collapses into one 2x2 matrix (identity for unrotated icons), and
        // a white tint is the identity multiply, so the common untinted case
        // skips the color math entirely.
        let [col_x, col_y] = rotation_columns(instance.direction, instance.half_extents);
        let tint_is_white = instance.tints == [Color32::WHITE; 2];
        // extend with exact-size iterators: one reserve per template.
        mesh.vertices.extend(template.vertices.iter().map(|vertex| {
            let [px, py] = vertex.pos;
            epaint::Vertex {
                pos: instance.center + col_x * px + col_y * py,
                uv: epaint::WHITE_UV,
                color: if tint_is_white {
                    premultiplied(vertex.color)
                } else {
                    let [primary, secondary] = instance.tints;
                    let tint = if vertex.tint_slot == 0 {
                        primary
                    } else {
                        secondary
                    };
                    tinted_color(vertex.color, tint)
                },
            }
        }));
        mesh.indices
            .extend(template.indices.iter().map(|&index| base + index));
    }

    /// Flush the collected icons so a painter primitive drawn next stacks
    /// above them, exactly as with immediate painting. Keeps the backend.
    pub fn barrier(&mut self, painter: &egui::Painter) {
        self.flush(painter);
    }

    /// Add the collected icons to `painter`. An empty batch paints nothing.
    pub fn paint(mut self, painter: &egui::Painter) {
        self.flush(painter);
    }

    fn flush(&mut self, painter: &egui::Painter) {
        match &mut self.backend {
            Backend::Cpu(mesh) => {
                let mesh = std::mem::take(mesh);
                if !mesh.is_empty() {
                    painter.add(egui::Shape::Mesh(mesh.into()));
                }
            }
            Backend::Gpu(pending) => {
                let pending = std::mem::take(pending);
                if pending.is_empty() {
                    return;
                }
                if pending.len() < gpu::GPU_MIN_INSTANCES {
                    // Small segment: the CPU mesh is cheaper than a
                    // dedicated buffer and draw call.
                    let mut cpu = IconMeshBatch::new(self.library, self.pixels_per_point);
                    for instance in pending {
                        cpu.push(instance);
                    }
                    cpu.paint(painter);
                    return;
                }
                let Some(library) = self.library else {
                    return;
                };
                let groups = self.group_instances(library, &pending);
                painter.add(egui_wgpu::Callback::new_paint_callback(
                    painter.clip_rect(),
                    IconDrawCallback::new(groups),
                ));
            }
        }
    }

    /// Group instances by (icon, bucket) in first-seen order, preserving the
    /// relative order of same-template icons. Distinct templates only reorder
    /// where they overlap, which the barriers rule out.
    fn group_instances(
        &self,
        library: &IconMeshLibrary,
        pending: &[IconInstance],
    ) -> Vec<InstanceGroup> {
        let mut groups: Vec<InstanceGroup> = Vec::new();
        for instance in pending {
            let target_px = physical_extent_px(instance.half_extents, self.pixels_per_point);
            let bucket = library
                .tessellation(instance.icon)
                .bucket_ordinal_for(target_px);
            let [col_x, col_y] = rotation_columns(instance.direction, instance.half_extents);
            let gpu_instance = GpuIconInstance {
                center: [instance.center.x, instance.center.y],
                col_x: [col_x.x, col_x.y],
                col_y: [col_y.x, col_y.y],
                tints: [
                    gpu::pack_color32(instance.tints[0]),
                    gpu::pack_color32(instance.tints[1]),
                ],
            };
            match groups
                .iter_mut()
                .find(|group| group.icon == instance.icon && group.bucket == bucket)
            {
                Some(group) => group.instances.push(gpu_instance),
                None => groups.push(InstanceGroup {
                    icon: instance.icon,
                    bucket,
                    instances: vec![gpu_instance],
                }),
            }
        }
        groups
    }
}

/// Rotate `offset` so the template's "up" direction `(0, -1)` aligns with
/// `direction` (a unit vector).
fn rotate_up_to(offset: Vec2, direction: Vec2) -> Vec2 {
    Vec2::new(
        -offset.x * direction.y - offset.y * direction.x,
        offset.x * direction.x - offset.y * direction.y,
    )
}

/// The columns of the combined scale-then-rotate map applied to normalized
/// template positions: `pos = center + col_x * px + col_y * py`.
/// Expressed through [`rotate_up_to`] on the axis vectors so the two stay one
/// definition.
fn rotation_columns(direction: Option<Vec2>, half_extents: Vec2) -> [Vec2; 2] {
    let x_axis = Vec2::new(half_extents.x, 0.0);
    let y_axis = Vec2::new(0.0, half_extents.y);
    match direction {
        Some(direction) => [
            rotate_up_to(x_axis, direction),
            rotate_up_to(y_axis, direction),
        ],
        None => [x_axis, y_axis],
    }
}

/// A template's baked color as [Color32]. Both are premultiplied, so this
/// is a plain reinterpretation.
fn premultiplied(template: [u8; 4]) -> Color32 {
    let [r, g, b, a] = template;
    Color32::from_rgba_premultiplied(r, g, b, a)
}

/// An icon's full physical on-screen extent (larger axis), which selects its
/// size bucket - matching the template's larger viewbox axis, so an aspect-true
/// draw uses the bucket whose curve tolerance and fringe were baked for it.
fn physical_extent_px(half_extents_pt: Vec2, pixels_per_point: f32) -> f32 {
    half_extents_pt.max_elem() * 2.0 * pixels_per_point
}

/// Multiply a template's baked premultiplied color with a premultiplied
/// tint.
///
/// [Color32]'s `Mul` is a componentwise gamma-space multiply, exactly what
/// egui's shader does when tinting a textured quad.
fn tinted_color(template: [u8; 4], tint: Color32) -> Color32 {
    premultiplied(template) * tint
}

impl IconMeshBatch<'_> {
    /// The CPU backend's collected mesh, for tests.
    #[cfg(test)]
    fn cpu_mesh(&self) -> &epaint::Mesh {
        match &self.backend {
            Backend::Cpu(mesh) => mesh,
            Backend::Gpu(_) => panic!("test expected the CPU backend"),
        }
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    #[rstest]
    #[case::up_is_identity(Vec2::new(0.0, -1.0), Vec2::new(3.0, 4.0), Vec2::new(3.0, 4.0))]
    #[case::down_flips(Vec2::new(0.0, 1.0), Vec2::new(3.0, 4.0), Vec2::new(-3.0, -4.0))]
    #[case::right_rotates_quarter(Vec2::new(1.0, 0.0), Vec2::new(0.0, -1.0), Vec2::new(1.0, 0.0))]
    fn rotate_up_to_matches_convention(
        #[case] direction: Vec2,
        #[case] offset: Vec2,
        #[case] expected: Vec2,
    ) {
        let rotated = rotate_up_to(offset, direction);
        assert!(
            (rotated - expected).length() < 1e-6,
            "{rotated:?} != {expected:?}"
        );
    }

    #[rstest]
    #[case::white_preserves([200, 100, 50, 255], Color32::WHITE, Color32::from_rgb(200, 100, 50))]
    #[case::black_zeroes_rgb([200, 100, 50, 255], Color32::BLACK, Color32::from_rgba_premultiplied(0, 0, 0, 255))]
    #[case::transparent_template_stays_transparent([0, 0, 0, 0], Color32::WHITE, Color32::TRANSPARENT)]
    fn tinted_color_matches_texture_tinting(
        #[case] template: [u8; 4],
        #[case] tint: Color32,
        #[case] expected: Color32,
    ) {
        assert_eq!(tinted_color(template, tint), expected);
    }

    #[test]
    fn alpha_tint_fades_premultiplied_components() {
        let faded = tinted_color([200, 100, 50, 255], Color32::WHITE.gamma_multiply(0.5));
        assert!(faded.a() < 255 && faded.a() > 100);
        assert!(faded.r() < 200);
    }

    #[test]
    fn push_uses_the_physical_size_bucket() {
        let library = crate::icon_mesh::IconMeshLibrary::embedded().unwrap();
        let mut batch = IconMeshBatch::new(Some(&library), 2.0);
        batch.push(IconInstance {
            icon: IconId::Pin,
            center: Pos2::ZERO,
            half_extents: Vec2::splat(10.0),
            direction: None,
            tints: [Color32::WHITE; 2],
        });
        // Half extent 10 pt at 2 pixels per point = 40 physical px.
        let expected = library.tessellation(IconId::Pin).mesh_for(40.0);
        assert_eq!(batch.cpu_mesh().vertices.len(), expected.vertices.len());
        assert_eq!(batch.cpu_mesh().indices.len(), expected.indices.len());
    }

    #[test]
    fn non_square_half_extents_scale_axes_independently() {
        let library = crate::icon_mesh::IconMeshLibrary::embedded().unwrap();
        let mut batch = IconMeshBatch::new(Some(&library), 1.0);
        batch.push(IconInstance {
            icon: IconId::Pin,
            center: Pos2::ZERO,
            half_extents: Vec2::new(9.0, 12.0),
            direction: None,
            tints: [Color32::WHITE; 2],
        });
        let max_x = batch
            .cpu_mesh()
            .vertices
            .iter()
            .map(|vertex| vertex.pos.x.abs())
            .fold(0.0_f32, f32::max);
        let max_y = batch
            .cpu_mesh()
            .vertices
            .iter()
            .map(|vertex| vertex.pos.y.abs())
            .fold(0.0_f32, f32::max);
        // The pin fills most of its viewbox on both axes, so the mesh must
        // reach close to each half extent and clearly further in y than in x
        // (allowing a couple of points of fringe overhang beyond the viewbox).
        assert!((7.0..=11.5).contains(&max_x), "max_x = {max_x}");
        assert!((10.5..=14.5).contains(&max_y), "max_y = {max_y}");
        assert!(max_y > max_x, "y extent must exceed x extent");
    }

    #[test]
    fn batch_without_library_stays_empty() {
        let mut batch = IconMeshBatch::new(None, 1.0);
        batch.push(IconInstance {
            icon: IconId::Pin,
            center: Pos2::ZERO,
            half_extents: Vec2::splat(10.0),
            direction: None,
            tints: [Color32::WHITE; 2],
        });
        assert!(batch.cpu_mesh().is_empty());
    }

    #[test]
    fn empty_batch_paints_nothing() {
        let library = crate::icon_mesh::IconMeshLibrary::embedded().unwrap();
        let batch = IconMeshBatch::new(Some(&library), 1.0);
        assert!(batch.cpu_mesh().is_empty());
    }

    /// Grouping for the instanced draws: one group per (icon, bucket) in
    /// first-seen order, instances in push order within each group.
    #[test]
    fn group_instances_groups_by_template_in_first_seen_order() {
        let library = crate::icon_mesh::IconMeshLibrary::embedded().unwrap();
        let batch = IconMeshBatch::new(Some(&library), 1.0);
        let instance = |icon: IconId, half_extent: f32, x: f32| IconInstance {
            icon,
            center: Pos2::new(x, 0.0),
            half_extents: Vec2::splat(half_extent),
            direction: None,
            tints: [Color32::WHITE; 2],
        };
        // Ghost first, then arrows, then a differently sized ghost (its own
        // bucket, so its own group), then another ghost at the first size.
        let pending = [
            instance(IconId::GhostFix, 9.0, 0.0),
            instance(IconId::NavArrow, 9.0, 1.0),
            instance(IconId::NavArrow, 9.0, 2.0),
            instance(IconId::GhostFix, 30.0, 3.0),
            instance(IconId::GhostFix, 9.0, 4.0),
        ];
        let groups = batch.group_instances(&library, &pending);

        let shape: Vec<(IconId, usize)> = groups
            .iter()
            .map(|group| (group.icon, group.instances.len()))
            .collect();
        assert_eq!(
            shape,
            vec![
                (IconId::GhostFix, 2),
                (IconId::NavArrow, 2),
                (IconId::GhostFix, 1),
            ],
            "first-seen group order with per-bucket splits"
        );
        assert_ne!(
            groups[0].bucket, groups[2].bucket,
            "different physical sizes must land in different buckets"
        );
        // Push order within a group: x encodes it (exact copies of the
        // pushed values, so a plain ordering check suffices).
        let xs: Vec<i32> = groups[0]
            .instances
            .iter()
            .map(|instance| instance.center[0] as i32)
            .collect();
        assert_eq!(xs, vec![0, 4]);
    }
}

#[cfg(test)]
mod gpu_projection_tests {
    use super::*;
    use crate::icon_mesh::IconMeshLibrary;

    #[derive(Clone, Copy)]
    enum Backend {
        Cpu,
        Gpu,
    }

    /// Draw an 8x8 grid of identical icons - 64 instances, comfortably above
    /// [gpu::GPU_MIN_INSTANCES] so the GPU backend takes the instanced-draw
    /// path - inside `clip`, using `backend`, and return the rendered frame.
    ///
    /// The painter handed to [IconMeshBatch::paint] is clipped to `clip`, so on
    /// the GPU path the paint callback's rect (hence egui-wgpu's render-pass
    /// viewport) is exactly `clip`.
    fn render_icon_grid(size: egui::Vec2, clip: egui::Rect, backend: Backend) -> image::RgbaImage {
        let library = IconMeshLibrary::embedded().unwrap();
        let mut harness = crate::test_harness::builder().size(size).ui(move |ui| {
            ui.painter()
                .rect_filled(ui.max_rect(), 0.0, Color32::from_rgb(30, 30, 30));

            let (cols, rows) = (8usize, 8usize);
            let cell_w = clip.width() / cols as f32;
            let cell_h = clip.height() / rows as f32;
            let mut batch = match backend {
                Backend::Cpu => IconMeshBatch::new(Some(&library), ui.pixels_per_point()),
                Backend::Gpu => IconMeshBatch::gpu_when_available(ui, Some(&library)),
            };
            for row in 0..rows {
                for col in 0..cols {
                    batch.push(IconInstance {
                        icon: IconId::Pin,
                        center: egui::pos2(
                            clip.min.x + (col as f32 + 0.5) * cell_w,
                            clip.min.y + (row as f32 + 0.5) * cell_h,
                        ),
                        half_extents: Vec2::splat(8.0),
                        direction: None,
                        tints: [Color32::WHITE; 2],
                    });
                }
            }
            let painter = ui.painter().with_clip_rect(clip);
            batch.paint(&painter);
        });
        harness.run();
        harness.inner.render().expect("failed to render frame")
    }

    /// Count pixels differing by more than a per-channel tolerance, as a
    /// fraction of the frame. The two icon backends rasterize the same baked
    /// template through different pipelines (epaint's mesh vs our instanced
    /// shader), so a handful of edge pixels per icon legitimately differ. A
    /// placement divergence differs across whole icons instead.
    fn diff_fraction(a: &image::RgbaImage, b: &image::RgbaImage) -> f64 {
        assert_eq!(a.dimensions(), b.dimensions(), "frame sizes must match");
        let (w, h) = a.dimensions();
        let mut differing = 0u64;
        for y in 0..h {
            for x in 0..w {
                let pa = a.get_pixel(x, y).0;
                let pb = b.get_pixel(x, y).0;
                let max_delta = (0..4)
                    .map(|i| i32::from(pa[i]).abs_diff(i32::from(pb[i])))
                    .max()
                    .unwrap_or(0);
                if max_delta > 24 {
                    differing += 1;
                }
            }
        }
        f64::from(u32::try_from(differing).unwrap_or(u32::MAX)) / f64::from(w * h)
    }

    /// Baseline: with the icon painter clipped to the whole frame, the paint
    /// callback's viewport equals the framebuffer, so the GPU instanced path
    /// and the CPU mesh path must produce the same image. This isolates the
    /// next test's failure to the *viewport*, not to a pipeline-parity issue.
    #[test]
    fn gpu_and_cpu_match_when_clip_fills_the_frame() {
        let size = egui::vec2(400.0, 320.0);
        let full = egui::Rect::from_min_size(egui::Pos2::ZERO, size);
        let cpu = render_icon_grid(size, full, Backend::Cpu);
        let gpu = render_icon_grid(size, full, Backend::Gpu);
        let frac = diff_fraction(&cpu, &gpu);
        assert!(
            frac < 0.01,
            "full-frame GPU vs CPU icons differ by {:.2}% - the two pipelines \
             should rasterize the shared template identically",
            frac * 100.0
        );
    }

    /// Regression test for icons "placed incorrectly when zooming out".
    ///
    /// The map widget is always inset by the side panels, so the icon
    /// painter's clip rect is a sub-rect of the framebuffer. egui-wgpu sets a
    /// paint callback's render-pass viewport to that clip rect, so the
    /// instanced shader must map screen points into NDC relative to the clip
    /// rect - not the full framebuffer. Zooming out pushes the visible-icon
    /// count past [gpu::GPU_MIN_INSTANCES], switching from the (correct) CPU
    /// mesh path to the GPU instanced draw. If the shader assumes the whole
    /// framebuffer, every icon is offset and scaled into a corner.
    ///
    /// The CPU path is unaffected by the viewport (its vertices are absolute
    /// screen positions), so it is the correct reference. The two must match.
    #[test]
    fn gpu_instanced_icons_match_cpu_placement_in_inset_viewport() {
        let size = egui::vec2(400.0, 320.0);
        // A sub-rect standing in for the map widget inset by the side panels.
        let inset = egui::Rect::from_min_size(egui::pos2(150.0, 90.0), egui::vec2(220.0, 200.0));
        let cpu = render_icon_grid(size, inset, Backend::Cpu);
        let gpu = render_icon_grid(size, inset, Backend::Gpu);
        let frac = diff_fraction(&cpu, &gpu);
        assert!(
            frac < 0.01,
            "GPU instanced icons are misplaced under an inset clip rect: \
             {:.1}% of pixels differ from the CPU reference. The instanced \
             shader maps NDC to the full framebuffer, but egui-wgpu set the \
             render-pass viewport to the callback's clip rect.",
            frac * 100.0
        );
    }
}

#[cfg(test)]
mod snapshot_tests {
    use std::f32::consts::FRAC_1_SQRT_2;

    use strum::IntoEnumIterator as _;

    use super::*;
    use crate::icon_mesh::IconMeshLibrary;

    /// Every icon at several sizes plus a rotated, a tinted, a faded, and a
    /// non-square variant - the mesh-pipeline counterpart of
    /// `all_marker_icons`.
    #[test]
    fn icon_mesh_grid_renders_correctly() {
        let icons: Vec<IconId> = IconId::iter().collect();
        let library = IconMeshLibrary::embedded().unwrap();
        let cell = 44.0_f32;
        let margin = 30.0_f32;
        let variants: [(Vec2, Option<Vec2>, Color32); 7] = [
            (Vec2::splat(4.0), None, Color32::WHITE),
            (Vec2::splat(10.0), None, Color32::WHITE),
            (Vec2::splat(16.0), None, Color32::WHITE),
            (
                Vec2::splat(10.0),
                Some(Vec2::new(FRAC_1_SQRT_2, -FRAC_1_SQRT_2)),
                Color32::WHITE,
            ),
            (Vec2::splat(10.0), None, Color32::from_rgb(100, 200, 255)),
            (Vec2::splat(10.0), None, Color32::WHITE.gamma_multiply(0.4)),
            // Aspect-true pin proportions: the stretch path the pins use.
            (Vec2::new(9.0, 12.0), None, Color32::WHITE),
        ];
        let width = margin * 2.0 + variants.len() as f32 * cell;
        let height = margin * 2.0 + icons.len() as f32 * cell;

        let mut harness = crate::test_harness::builder()
            .size(egui::vec2(width, height))
            .ui(move |ui| {
                ui.painter()
                    .rect_filled(ui.max_rect(), 0.0, Color32::from_rgb(30, 30, 30));

                let mut batch = IconMeshBatch::new(Some(&library), ui.pixels_per_point());
                for (row, &icon) in icons.iter().enumerate() {
                    let y = margin + (row as f32 + 0.5) * cell;
                    for (col, (half_extents, direction, tint)) in variants.into_iter().enumerate() {
                        batch.push(IconInstance {
                            icon,
                            center: egui::pos2(margin + (col as f32 + 0.5) * cell, y),
                            half_extents,
                            direction,
                            tints: [tint; 2],
                        });
                    }
                }
                batch.paint(ui.painter());
            });

        harness.run();
        // Loose: mesh edges rasterize a few pixels differently between the
        // Linux baseline and the macOS CI runner's Metal backend.
        harness.snapshot_loose("icon_mesh_grid");
    }
}

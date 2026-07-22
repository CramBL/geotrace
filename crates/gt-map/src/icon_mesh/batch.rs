//! The per-pass icon instance collector and its CPU mesh backend.
//!
//! Renderers push [IconInstance]s into an [IconMeshBatch], which transforms
//! the pre-tessellated templates (scale, rotation, tint) and appends
//! everything into one untextured [epaint::Mesh].
//! egui merges untextured vertex-colored meshes into its surrounding draw
//! batches, so a whole pass of icons costs no extra draw calls, and the
//! baked anti-alias fringe scales with the geometry instead of blurring the
//! way rasterized textures do.

use egui::epaint;
use egui::{Color32, Pos2, Vec2};

use crate::icon_mesh::{IconId, IconMeshLibrary};

/// One icon to draw this pass.
#[derive(Debug, Clone, Copy)]
pub struct IconInstance {
    pub icon: IconId,
    pub center: Pos2,
    /// Half extent in points: the icon spans `2 * size` by `2 * size`.
    pub size: f32,
    /// Unit direction the icon's "up" aligns to; `None` draws it upright.
    pub direction: Option<Vec2>,
    /// Multiplied onto the template's baked colors, like a texture tint:
    /// [Color32::WHITE] keeps the SVG colors, alpha fades the whole icon.
    pub tint: Color32,
}

/// Collects the icon instances of one renderer pass into a single mesh.
///
/// Create it where the pass starts drawing icons and [IconMeshBatch::paint]
/// it at the same point, so layering against non-icon shapes is unchanged.
pub struct IconMeshBatch<'a> {
    library: &'a IconMeshLibrary,
    pixels_per_point: f32,
    mesh: epaint::Mesh,
}

impl<'a> IconMeshBatch<'a> {
    /// `pixels_per_point` (`ui.pixels_per_point()`) picks each instance's
    /// size bucket from its physical on-screen extent.
    pub fn new(library: &'a IconMeshLibrary, pixels_per_point: f32) -> Self {
        Self {
            library,
            pixels_per_point,
            mesh: epaint::Mesh::default(),
        }
    }

    pub fn push(&mut self, instance: IconInstance) {
        let target_px = physical_extent_px(instance.size, self.pixels_per_point);
        let template = self.library.tessellation(instance.icon).mesh_for(target_px);

        debug_assert!(
            u32::try_from(self.mesh.vertices.len() + template.vertices.len()).is_ok(),
            "icon batch exceeds the u32 index range"
        );
        let base = self.mesh.vertices.len() as u32;
        for vertex in &template.vertices {
            let offset = Vec2::new(vertex.pos[0], vertex.pos[1]) * instance.size;
            let rotated = match instance.direction {
                Some(direction) => rotate_up_to(offset, direction),
                None => offset,
            };
            self.mesh.vertices.push(epaint::Vertex {
                pos: instance.center + rotated,
                uv: epaint::WHITE_UV,
                color: tinted_color(vertex.color, instance.tint),
            });
        }
        self.mesh
            .indices
            .extend(template.indices.iter().map(|&index| base + index));
    }

    /// Add the collected mesh to `painter`; a no-op for an empty batch.
    pub fn paint(self, painter: &egui::Painter) {
        if !self.mesh.is_empty() {
            painter.add(egui::Shape::Mesh(self.mesh.into()));
        }
    }
}

/// Rotate `offset` so the template's "up" direction `(0, -1)` aligns with
/// `direction` (a unit vector). Same convention as the rotated texture quads
/// this pipeline replaces.
fn rotate_up_to(offset: Vec2, direction: Vec2) -> Vec2 {
    Vec2::new(
        -offset.x * direction.y - offset.y * direction.x,
        offset.x * direction.x - offset.y * direction.y,
    )
}

/// An icon's full physical on-screen extent, which selects its size bucket.
fn physical_extent_px(half_extent_pt: f32, pixels_per_point: f32) -> f32 {
    half_extent_pt * 2.0 * pixels_per_point
}

/// Multiply a template's straight-alpha sRGB color with a premultiplied tint.
///
/// [Color32]'s `Mul` is a componentwise gamma-space multiply, exactly what
/// egui's shader does when tinting a textured quad, so tint semantics match
/// the texture path.
fn tinted_color(template: [u8; 4], tint: Color32) -> Color32 {
    let [r, g, b, a] = template;
    Color32::from_rgba_unmultiplied(r, g, b, a) * tint
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
    #[case::transparent_template_stays_transparent([200, 100, 50, 0], Color32::WHITE, Color32::TRANSPARENT)]
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
        let mut batch = IconMeshBatch::new(&library, 2.0);
        batch.push(IconInstance {
            icon: IconId::Pin,
            center: Pos2::ZERO,
            size: 10.0,
            direction: None,
            tint: Color32::WHITE,
        });
        // Half extent 10 pt at 2x ppp = 40 physical px.
        let expected = library.tessellation(IconId::Pin).mesh_for(40.0);
        assert_eq!(batch.mesh.vertices.len(), expected.vertices.len());
        assert_eq!(batch.mesh.indices.len(), expected.indices.len());
    }

    #[test]
    fn empty_batch_paints_nothing() {
        let library = crate::icon_mesh::IconMeshLibrary::embedded().unwrap();
        let batch = IconMeshBatch::new(&library, 1.0);
        assert!(batch.mesh.is_empty());
    }
}

#[cfg(test)]
mod snapshot_tests {
    use std::f32::consts::FRAC_1_SQRT_2;

    use strum::IntoEnumIterator as _;

    use super::*;
    use crate::icon_mesh::IconMeshLibrary;
    use crate::test_harness::TestHarness;

    /// Every icon at several sizes plus a rotated, a tinted, and a faded
    /// variant - the mesh-pipeline counterpart of `all_marker_icons`.
    #[test]
    fn icon_mesh_grid_renders_correctly() {
        let icons: Vec<IconId> = IconId::iter().collect();
        let library = IconMeshLibrary::embedded().unwrap();
        let cell = 44.0_f32;
        let margin = 30.0_f32;
        let variants: [(f32, Option<Vec2>, Color32); 6] = [
            (4.0, None, Color32::WHITE),
            (10.0, None, Color32::WHITE),
            (16.0, None, Color32::WHITE),
            (
                10.0,
                Some(Vec2::new(FRAC_1_SQRT_2, -FRAC_1_SQRT_2)),
                Color32::WHITE,
            ),
            (10.0, None, Color32::from_rgb(100, 200, 255)),
            (10.0, None, Color32::WHITE.gamma_multiply(0.4)),
        ];
        let width = margin * 2.0 + variants.len() as f32 * cell;
        let height = margin * 2.0 + icons.len() as f32 * cell;

        let mut harness = TestHarness::builder()
            .size(egui::vec2(width, height))
            .ui(move |ui| {
                ui.painter()
                    .rect_filled(ui.max_rect(), 0.0, Color32::from_rgb(30, 30, 30));

                let mut batch = IconMeshBatch::new(&library, ui.pixels_per_point());
                for (row, &icon) in icons.iter().enumerate() {
                    let y = margin + (row as f32 + 0.5) * cell;
                    for (col, (size, direction, tint)) in variants.into_iter().enumerate() {
                        batch.push(IconInstance {
                            icon,
                            center: egui::pos2(margin + (col as f32 + 0.5) * cell, y),
                            size,
                            direction,
                            tint,
                        });
                    }
                }
                batch.paint(ui.painter());
            });

        harness.run();
        harness.snapshot("icon_mesh_grid");
    }
}

use uom::si::angle::degree;

use super::*;

/// Navigation arrows across the zoom size range (3-12 pt), several
/// headings, an outline fade, and a highlight - the parity grid used to
/// compare the painter and mesh implementations.
#[test]
fn nav_arrow_grid_renders_correctly() {
    let sizes = [3.0_f32, 6.0, 9.0, 12.0];
    let headings = [0.0_f64, 45.0, 120.0, 230.0];
    let cell = 44.0_f32;
    let margin = 26.0_f32;
    // Per row: the headings at full opacity, two faded outlines, one
    // highlighted.
    let cols = headings.len() + 3;
    let width = margin * 2.0 + cols as f32 * cell;
    let height = margin * 2.0 + sizes.len() as f32 * cell;

    let library = crate::icon_mesh::IconMeshLibrary::embedded().ok();
    let mut harness = crate::test_harness::builder()
        .size(egui::vec2(width, height))
        .ui(move |ui| {
            ui.painter()
                .rect_filled(ui.max_rect(), 0.0, Color32::from_rgb(30, 30, 30));
            let mut batch = IconMeshBatch::gpu_when_available(ui, library.as_ref());

            let center = |row: usize, col: usize| {
                egui::pos2(
                    margin + (col as f32 + 0.5) * cell,
                    margin + (row as f32 + 0.5) * cell,
                )
            };
            for (row, &size) in sizes.iter().enumerate() {
                for (col, &heading_deg) in headings.iter().enumerate() {
                    draw_navigation_arrow(
                        ui,
                        &mut batch,
                        center(row, col),
                        Angle::new::<degree>(heading_deg),
                        FIX_STRONG_BLUE,
                        false,
                        1.0,
                        size,
                    );
                }
                for (i, fade) in [0.6, 0.25].into_iter().enumerate() {
                    draw_navigation_arrow(
                        ui,
                        &mut batch,
                        center(row, headings.len() + i),
                        Angle::new::<degree>(315.0),
                        FIX_MARGINAL_YELLOW.gamma_multiply(fade),
                        false,
                        fade,
                        size,
                    );
                }
                draw_navigation_arrow(
                    ui,
                    &mut batch,
                    center(row, headings.len() + 2),
                    Angle::new::<degree>(90.0),
                    FIX_LOST_RED,
                    true,
                    1.0,
                    size,
                );
            }
            batch.paint(ui.painter());
        });

    harness.run();
    // Loose: mesh edges rasterize a few pixels differently between the
    // Linux baseline and the macOS CI runner's Metal backend.
    harness.snapshot_loose("nav_arrow_grid");
}

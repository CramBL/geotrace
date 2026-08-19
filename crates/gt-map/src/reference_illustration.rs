//! Renders the TEC reference material's illustration into the assets of
//! `gt-ionex`, whose document embeds it.
//!
//! The document belongs to `gt-ionex`, which holds the archived maps but
//! cannot draw them: this crate projects them. `just generate-reference-tec-map`
//! runs the ignored test below to write the asset across that boundary.

use std::path::PathBuf;

use chrono::{DateTime, NaiveDate, Utc};
use egui::{Mesh, Rect, Shape};
use gt_ionex::maps::GlobalIonosphereMaps;
use walkers::{MapMemory, Projector};

use crate::tec_renderer::{TecHeatmapSnapshot, visible_cells};
use crate::transform::MercTransform;

/// Where the rendered map lands, resolved from this crate's manifest dir.
const ASSET_PATH: &str = "../gt-ionex/assets/tec_map_2024_05_10_gannon_storm.png";

/// Zoom at which the world spans `256 * 2^2 = 1024` pixels, which is the
/// canvas width below. The canvas height then covers 66 degrees of latitude
/// either side of the equator, the band the published grid carries its
/// structure in.
const WORLD_ZOOM: f64 = 2.0;

const CANVAS_SIZE: egui::Vec2 = egui::vec2(1024.0, 512.0);

/// Drawn without the transparency the app lays the heatmap over its tiles
/// with: the illustration has no tiles under it.
const OPACITY_PERCENT: f32 = 100.0;

/// The archived epoch of 10 May 2024 whose map peaks highest, which is the
/// value the reference material quotes.
fn storm_peak_instant() -> DateTime<Utc> {
    NaiveDate::from_ymd_opt(2024, 5, 10)
        .and_then(|day| day.and_hms_opt(22, 0, 0))
        .map(|naive| naive.and_utc())
        .expect("an epoch of the archived day")
}

/// Paints the archived grid alone: no tiles, no tracks, and none of the map's
/// controls, so the asset shows the field and nothing else.
fn draw_world_heatmap(ui: &egui::Ui, maps: &GlobalIonosphereMaps) {
    let rect = Rect::from_min_size(egui::Pos2::ZERO, CANVAS_SIZE);
    let mut map_memory = MapMemory::default();
    map_memory.set_zoom(WORLD_ZOOM).expect("a valid zoom");
    let projector = Projector::new(rect, &map_memory, walkers::lat_lon(0.0, 0.0));
    let transform = MercTransform::new(&projector, &map_memory, rect.center());
    let snapshot = TecHeatmapSnapshot {
        maps,
        instant: storm_peak_instant(),
    };
    let cells = visible_cells(
        &snapshot,
        &transform,
        rect,
        ui.visuals().dark_mode,
        OPACITY_PERCENT,
    );
    let mut mesh = Mesh::default();
    for cell in &cells {
        mesh.add_colored_rect(cell.rect, cell.fill);
    }
    ui.painter().add(Shape::mesh(mesh));
}

/// Writes the illustration the TEC reference material shows. Ignored so it
/// runs only when the asset is regenerated, which the just recipe does.
#[test]
#[ignore = "writes a committed asset"]
fn generate_tec_reference_illustration() {
    let maps = gt_ionex::captured_maps(gt_ionex::STORM_CAPTURE).expect("the storm capture");
    let mut harness = crate::test_harness::builder()
        .size(CANVAS_SIZE)
        .theme(true)
        .ui(move |ui| draw_world_heatmap(ui, &maps));
    harness.run();
    let rendered = harness.inner.render().expect("the frame renders");
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(ASSET_PATH);
    rendered.save(&path).expect("the asset is written");
    println!("wrote {}", path.display());
}

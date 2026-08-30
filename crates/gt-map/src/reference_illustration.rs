//! Renders the illustrations of the reference material into the assets of the
//! crates whose documents embed them: the TEC map into `gt-ionex`, the
//! aircraft-interference map into `gt-jam`.
//!
//! Each document belongs to the crate holding its data, which cannot draw it:
//! this crate projects it. `just generate-reference-tec-map` and
//! `just generate-reference-interference-map` run the ignored tests below to
//! write the assets across that boundary.

use std::fs;
use std::path::PathBuf;

use chrono::{DateTime, NaiveDate, Utc};
use egui::{Mesh, Rect, Shape, Stroke};
use gt_ionex::maps::GlobalIonosphereMaps;
use gt_jam::dataset::JamDataset;
use gt_jam::wire::{self, ParseWarningReporter};
use walkers::{MapMemory, Projector};

use crate::tec_renderer::{TecHeatmapSnapshot, visible_cells};
use crate::transform::MercTransform;

/// Where the rendered TEC map lands, resolved from this crate's manifest dir.
const TEC_ASSET_PATH: &str = "../gt-ionex/assets/tec_map_2024_05_10_gannon_storm.png";

/// Where the rendered interference map lands, resolved from this crate's
/// manifest dir.
const INTERFERENCE_ASSET_PATH: &str = "../gt-jam/assets/interference_map_2026_07_20.png";

/// Zoom at which the world spans `256 * 2^2 = 1024` pixels, which is the
/// canvas width below. The canvas height then covers 66 degrees of latitude
/// either side of the equator, the band the published grid carries its
/// structure in.
const WORLD_ZOOM: f64 = 2.0;

const CANVAS_SIZE: egui::Vec2 = egui::vec2(1024.0, 512.0);

/// Zoom at which the world spans `256 * 2^3 = 2048` pixels, where one
/// published cell of about 22 km is two pixels across.
const INTERFERENCE_WORLD_ZOOM: f64 = 3.0;

const INTERFERENCE_CANVAS_SIZE: egui::Vec2 = egui::vec2(2048.0, 1024.0);

/// The interference canvas is drawn at twice the width the asset is written
/// at and downsampled: a cell covering half a pixel of the written asset
/// lands as coverage rather than as a dropped or aliased hexagon.
const INTERFERENCE_SUPERSAMPLE: u32 = 2;

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

fn world_memory(zoom: f64) -> MapMemory {
    let mut map_memory = MapMemory::default();
    map_memory.set_zoom(zoom).expect("a valid zoom");
    map_memory
}

fn world_transform(rect: Rect, zoom: f64) -> MercTransform {
    let map_memory = world_memory(zoom);
    let projector = Projector::new(rect, &map_memory, walkers::lat_lon(0.0, 0.0));
    MercTransform::new(&projector, &map_memory, rect.center())
}

/// Paints the archived grid alone: no tiles, no tracks, and none of the map's
/// controls, so the asset shows the field and nothing else.
fn draw_world_heatmap(ui: &egui::Ui, maps: &GlobalIonosphereMaps) {
    let rect = Rect::from_min_size(egui::Pos2::ZERO, CANVAS_SIZE);
    let transform = world_transform(rect, WORLD_ZOOM);
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

/// Paints the day's cells alone, each in the colour its share of low-accuracy
/// reports puts it at. The fill is opaque, as for the heatmap: the
/// illustration has no tiles under it.
fn draw_world_interference(ui: &egui::Ui, dataset: &JamDataset) {
    let rect = Rect::from_min_size(egui::Pos2::ZERO, INTERFERENCE_CANVAS_SIZE);
    let transform = world_transform(rect, INTERFERENCE_WORLD_ZOOM);
    let dark_mode = ui.visuals().dark_mode;
    let cells = crate::jamming_renderer::visible_cells(dataset, &transform, rect, dark_mode);
    for cell in &cells {
        let Some(rate) = cell.observation.rate() else {
            continue;
        };
        let fill = gt_ui_theme::interference_color(rate.bad_fraction).resolve(dark_mode);
        ui.painter().add(Shape::convex_polygon(
            cell.outline.clone(),
            fill,
            Stroke::NONE,
        ));
    }
}

/// The captured world day, read from the fixture `gt-jam` commits.
fn captured_interference_day() -> JamDataset {
    let fixture = gt_jam::FIXTURE_DAYS
        .into_iter()
        .find(gt_jam::FixtureDay::is_served)
        .expect("a served fixture day");
    let day = gt_jam::parse_day(fixture.day).expect("a calendar date");
    let csv = fs::read_to_string(gt_jam::fixtures_dir().join(gt_jam::dataset_file_name(day)))
        .expect("the captured day");
    let observations =
        wire::parse_dataset(&csv, &ParseWarningReporter::default()).expect("the day parses");
    JamDataset::new(day, observations)
}

fn asset_path(relative: &str) -> PathBuf {
    gt_test_utils::cargo_manifest_dir().join(relative)
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
    let path = asset_path(TEC_ASSET_PATH);
    rendered.save(&path).expect("the asset is written");
    println!("wrote {}", path.display());
}

/// Writes the illustration the aircraft-interference reference material shows.
/// Ignored so it runs only when the asset is regenerated, which the just
/// recipe does.
#[test]
#[ignore = "writes a committed asset"]
fn generate_interference_reference_illustration() {
    let dataset = captured_interference_day();
    let mut harness = crate::test_harness::builder()
        .size(INTERFERENCE_CANVAS_SIZE)
        .theme(true)
        .ui(move |ui| draw_world_interference(ui, &dataset));
    harness.run();
    let rendered = harness.inner.render().expect("the frame renders");
    let written = image::imageops::resize(
        &rendered,
        rendered.width() / INTERFERENCE_SUPERSAMPLE,
        rendered.height() / INTERFERENCE_SUPERSAMPLE,
        image::imageops::FilterType::Lanczos3,
    );
    let path = asset_path(INTERFERENCE_ASSET_PATH);
    written.save(&path).expect("the asset is written");
    println!("wrote {}", path.display());
}

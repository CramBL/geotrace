//! Bakes the marker icon SVGs into pre-tessellated meshes embedded in the
//! binary; see the `icon_mesh` module for the runtime side.

use std::error::Error;
use std::path::PathBuf;
use std::{env, fs};

use gt_icon_tessellate::tessellate;
use gt_icon_tessellate::{IconTessellation, StrokeWidthUnit};

/// The one asset baked with [StrokeWidthUnit::PhysicalPixels]: the nav arrow
/// rim keeps the painter path's constant on-screen width across zoom sizes
/// (see the asset's comment). Everything else scales strokes with the glyph.
///
/// build.rs cannot see the crate's `IconId` enum, so the stem is a string
/// here; [main] fails the build if the asset disappears, and the runtime
/// decode rejects unknown stems, so a rename cannot silently change modes.
const PHYSICAL_PIXEL_STROKE_STEMS: [&str; 1] = ["nav_arrow_outline"];

fn main() -> Result<(), Box<dyn Error>> {
    let icons_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/icons");
    // A directory path makes cargo scan it recursively, so edits, additions,
    // and removals all retrigger the bake.
    println!("cargo::rerun-if-changed={}", icons_dir.display());

    let mut entries: Vec<(String, IconTessellation)> = Vec::new();
    for dir_entry in fs::read_dir(&icons_dir)? {
        let path = dir_entry?.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("svg") {
            return Err(format!(
                "unexpected non-SVG file in {}: {path:?}",
                icons_dir.display()
            )
            .into());
        }
        let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
            return Err(format!("icon asset with a non-UTF-8 name: {path:?}").into());
        };
        let svg = fs::read(&path)?;
        let stroke_width_unit = if PHYSICAL_PIXEL_STROKE_STEMS.contains(&stem) {
            StrokeWidthUnit::PhysicalPixels
        } else {
            StrokeWidthUnit::UserUnits
        };
        let tessellation = tessellate::tessellate_icon_with(&svg, stroke_width_unit)
            .map_err(|err| format!("failed to tessellate {path:?}: {err}"))?;
        entries.push((stem.to_owned(), tessellation));
    }
    entries.sort_by(|a, b| a.0.cmp(&b.0));

    // A rename of a physical-pixel asset must fail the bake, not silently
    // fall back to scaled strokes.
    for stem in PHYSICAL_PIXEL_STROKE_STEMS {
        if !entries.iter().any(|(name, _)| name == stem) {
            return Err(format!(
                "physical-pixel stroke asset {stem:?} not found in {}: renamed or removed?",
                icons_dir.display()
            )
            .into());
        }
    }

    let blob = postcard::to_allocvec(&entries)?;
    let out_path = PathBuf::from(env::var("OUT_DIR")?).join("icon_meshes.postcard");
    fs::write(&out_path, blob)?;
    Ok(())
}

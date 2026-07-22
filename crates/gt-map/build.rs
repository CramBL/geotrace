//! Bakes the marker icon SVGs into pre-tessellated meshes embedded in the
//! binary; see the `icon_mesh` module for the runtime side.

use std::error::Error;
use std::path::PathBuf;
use std::{env, fs};

use gt_icon_tessellate::IconTessellation;
use gt_icon_tessellate::tessellate;

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
        let tessellation = tessellate::tessellate_icon(&svg)
            .map_err(|err| format!("failed to tessellate {path:?}: {err}"))?;
        entries.push((stem.to_owned(), tessellation));
    }
    entries.sort_by(|a, b| a.0.cmp(&b.0));

    let blob = postcard::to_allocvec(&entries)?;
    let out_path = PathBuf::from(env::var("OUT_DIR")?).join("icon_meshes.postcard");
    fs::write(&out_path, blob)?;
    Ok(())
}

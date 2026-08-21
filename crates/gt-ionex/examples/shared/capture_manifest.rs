//! The manifest each capture tool keeps beside the files it wrote.
//!
//! One JSON object per captured file, under a `files` array: the fields naming
//! the capture, and the facts its maps parse to. A capture of a subset diffs
//! cleanly: entries are written in the order the caller hands them over.

use std::collections::BTreeMap;
use std::error::Error;
use std::fs;
use std::path::Path;

use serde_json::{Value, json};

use gt_ionex::CAPTURE_MANIFEST;
use gt_ionex::maps::GlobalIonosphereMaps;
use gt_ionex::tec::TotalElectronContent;

/// The entries recorded in `directory`, keyed by `identity_field`, which a
/// capture of a subset keeps the rest of.
pub fn recorded_entries(directory: &Path, identity_field: &str) -> BTreeMap<String, Value> {
    let Ok(recorded) = fs::read_to_string(directory.join(CAPTURE_MANIFEST)) else {
        return BTreeMap::new();
    };
    serde_json::from_str::<Value>(&recorded)
        .ok()
        .as_ref()
        .and_then(|manifest| manifest.get("files"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|entry| {
            Some((
                entry.get(identity_field)?.as_str()?.to_owned(),
                entry.clone(),
            ))
        })
        .collect()
}

/// One entry: the fields naming the capture, and the facts `maps` holds.
pub fn entry(naming_fields: Value, maps: &GlobalIonosphereMaps) -> Value {
    let mut entry = naming_fields;
    if let (Some(fields), Value::Object(facts)) = (entry.as_object_mut(), map_facts(maps)) {
        fields.extend(facts);
    }
    entry
}

pub fn write(directory: &Path, entries: &[Value]) -> Result<(), Box<dyn Error>> {
    fs::create_dir_all(directory)?;
    fs::write(
        directory.join(CAPTURE_MANIFEST),
        format!(
            "{}\n",
            serde_json::to_string_pretty(&json!({ "files": entries }))?
        ),
    )?;
    Ok(())
}

fn map_facts(maps: &GlobalIonosphereMaps) -> Value {
    let grid = maps.grid();
    json!({
        "maps": maps.maps().len(),
        "interval_seconds": maps.interval().num_seconds(),
        "first_epoch": maps.epoch_of_first_map().map(|epoch| epoch.to_rfc3339()),
        "last_epoch": maps.epoch_of_last_map().map(|epoch| epoch.to_rfc3339()),
        "latitude_nodes": grid.latitudes.node_count(),
        "longitude_nodes": grid.longitudes.node_count(),
        "latitude_step_degrees": grid.latitudes.axis().step_degrees(),
        "longitude_step_degrees": grid.longitudes.axis().step_degrees(),
        "shell_height_km": grid.shell_height_km,
        "peak_tecu": maps.peak_total_electron_content().map(TotalElectronContent::tecu),
        "gaps": maps.maps().iter().flat_map(|map| map.values()).filter(Option::is_none).count(),
    })
}

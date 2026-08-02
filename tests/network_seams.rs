//! Keeps network access behind a seam the application controls.
//!
//! Four crates can reach the network, and the application decides for each
//! one at startup: `gt-jam` and `gt-snap` through their `TransportSource`,
//! `gt-map` through [`gt_map::TileAccess`], and the root crate's update
//! check through `StartupOptions::offline`.
//!
//! A fifth crate taking its own HTTP dependency would escape all of that.
//! Depending on a crate is a prerequisite for using it, so the manifests are
//! checked and not the source: reqwest is reached by fully-qualified path,
//! never by `use`, so an import rule would pass while the calls remain.
//!
//! Limits worth knowing. This catches a crate that names a known HTTP client
//! in its manifest, and nothing else; it is not a reachability audit.
//! `walkers` and `axoupdater` are listed because they wrap an HTTP client
//! that the depending crate never names itself, and any other such wrapper
//! is invisible until it is added here. The gate recorded against each crate
//! is there for the failure message and is not itself verified.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use toml::Table;

/// Crates allowed to reach the network, and the gate named in the failure
/// message when a fifth one appears.
const NETWORK_CRATES: [(&str, &str); 4] = [
    ("gt-jam", "TransportSource"),
    ("gt-snap", "TransportSource"),
    ("gt-map", "TileAccess"),
    ("geotrace", "StartupOptions::offline"),
];

/// Crates that reach the network. `walkers` fetches map tiles and
/// `axoupdater` downloads releases, each over its own reqwest, so both count
/// even though the crate depending on them never names reqwest.
const HTTP_CRATES: [&str; 6] = ["reqwest", "ureq", "hyper", "curl", "walkers", "axoupdater"];

/// Manifest sections that make a crate usable from the crate's own code.
///
/// `[workspace.dependencies]` is deliberately absent: it declares versions
/// for the workspace to draw on and grants nothing by itself.
const DEPENDENCY_SECTIONS: [&str; 3] = ["dependencies", "dev-dependencies", "build-dependencies"];

/// Every workspace member's name and parsed manifest.
fn manifests() -> Result<Vec<(String, Table)>, String> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut found = vec![("geotrace".to_owned(), parse(&root.join("Cargo.toml"))?)];
    let crates = root.join("crates");
    let entries = fs::read_dir(&crates).map_err(|err| format!("{}: {err}", crates.display()))?;
    for entry in entries {
        let path = entry
            .map_err(|err| format!("{}: {err}", crates.display()))?
            .path();
        let manifest = path.join("Cargo.toml");
        if !manifest.is_file() {
            continue;
        }
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| format!("{} has no usable directory name", path.display()))?
            .to_owned();
        found.push((name, parse(&manifest)?));
    }
    found.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(found)
}

fn parse(path: &Path) -> Result<Table, String> {
    fs::read_to_string(path)
        .map_err(|err| format!("reading {}: {err}", path.display()))?
        .parse::<Table>()
        .map_err(|err| format!("parsing {}: {err}", path.display()))
}

/// Dependency names the crate's own code can reach, across every dependency
/// section including the target-specific ones.
fn dependencies(manifest: &Table) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    let mut collect = |table: &Table| {
        for section in DEPENDENCY_SECTIONS {
            let Some(Some(entries)) = table.get(section).map(toml::Value::as_table) else {
                continue;
            };
            names.extend(entries.keys().cloned());
        }
    };
    collect(manifest);
    // [target.'cfg(...)'.dependencies] and friends.
    if let Some(Some(targets)) = manifest.get("target").map(toml::Value::as_table) {
        for target in targets.values().filter_map(toml::Value::as_table) {
            collect(target);
        }
    }
    names
}

#[test]
fn only_gated_crates_depend_on_an_http_client() {
    let manifests = manifests().expect("workspace manifests");
    let allowed: BTreeSet<&str> = NETWORK_CRATES.iter().map(|(name, _)| *name).collect();
    let mut offenders = Vec::new();

    for (crate_name, manifest) in &manifests {
        if allowed.contains(crate_name.as_str()) {
            continue;
        }
        let declared = dependencies(manifest);
        for http in HTTP_CRATES {
            if declared.contains(http) {
                offenders.push(format!("{crate_name} depends on {http}"));
            }
        }
    }

    let gated: Vec<String> = NETWORK_CRATES
        .iter()
        .map(|(name, gate)| format!("{name} ({gate})"))
        .collect();
    assert!(
        offenders.is_empty(),
        "network access needs a gate the application controls, as in {}: {}",
        gated.join(", "),
        offenders.join(", ")
    );
}

/// The allowlist names crates that exist and still reach the network, so a
/// rename or a removed dependency cannot quietly widen it to nothing.
#[test]
fn the_allowlisted_crates_exist_and_use_an_http_client() {
    let manifests = manifests().expect("workspace manifests");
    for (expected, _) in NETWORK_CRATES {
        let (_, manifest) = manifests
            .iter()
            .find(|(name, _)| name == expected)
            .unwrap_or_else(|| panic!("{expected} is named in the allowlist but has no manifest"));
        let declared = dependencies(manifest);
        assert!(
            HTTP_CRATES.iter().any(|http| declared.contains(*http)),
            "{expected} is allowed an HTTP client but declares none; drop it from the allowlist"
        );
    }
}

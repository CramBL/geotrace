//! An interference archive created before archives recorded their free space
//! in pages, opened through the store again.

use std::fs;
use std::path::Path;

use chrono::{DateTime, TimeDelta, Utc};
use hdf5::plist::file_create::FileSpaceStrategy;
use hdf5::{File, Group};

use gt_hdf5_archive::attributes;
use gt_jam::wire::{self, HexObservation, ParseWarningReporter};
use gt_jam::{FIXTURE_DAYS, dataset_file_name, fixtures_dir, parse_day};
use gt_jam_store::{FILE_NAME, JamStore};

const HOST: &str = "https://gpsjam.org";

/// What [`gt_hdf5_archive::ArchiveFile::create`] creates an archive with.
const PAGED_FILE_SPACE: FileSpaceStrategy = FileSpaceStrategy::FreeSpaceManager {
    paged: true,
    persist: true,
    threshold: 1,
};

/// The day the archive is filled with, and its observations.
fn captured_day() -> Result<(chrono::NaiveDate, Vec<HexObservation>), String> {
    let fixture = FIXTURE_DAYS
        .iter()
        .find(|fixture| fixture.is_served())
        .ok_or("no served fixture day")?;
    let day = parse_day(fixture.day).map_err(|err| format!("fixture day: {err}"))?;
    let csv = fs::read_to_string(fixtures_dir().join(dataset_file_name(day)))
        .map_err(|err| format!("captured day: {err}"))?;
    let parsed = wire::parse_dataset(&csv, &ParseWarningReporter::default())
        .map_err(|err| format!("parse: {err}"))?;
    Ok((day, parsed))
}

fn file_space_strategy(path: &Path) -> Result<FileSpaceStrategy, String> {
    let file = File::open(path).map_err(|err| format!("open: {err}"))?;
    Ok(file
        .fcpl()
        .map_err(|err| format!("fcpl: {err}"))?
        .file_space_strategy())
}

/// Rewrite the archive at `path` into a file laid out the way libhdf5
/// defaults to, which is what a version before the paged strategy left on
/// disk.
fn rewrite_with_the_default_file_space(path: &Path) -> Result<(), String> {
    let unpaged = path.with_extension("unpaged");
    let source = File::open(path).map_err(|err| format!("open: {err}"))?;
    let target = File::create(&unpaged).map_err(|err| format!("create: {err}"))?;
    copy_members(&source, &target)?;
    copy_attributes(&source, &target)?;
    drop(target);
    drop(source);
    fs::rename(&unpaged, path).map_err(|err| format!("rename: {err}"))
}

fn copy_members(source: &Group, target: &Group) -> Result<(), String> {
    for name in source
        .member_names()
        .map_err(|err| format!("members: {err}"))?
    {
        source
            .group(&name)
            .map_err(|err| format!("{name} group: {err}"))?
            .copy_to(target, &name)
            .map_err(|err| format!("copy {name}: {err}"))?;
    }
    Ok(())
}

fn copy_attributes(source: &Group, target: &Group) -> Result<(), String> {
    for name in source
        .attr_names()
        .map_err(|err| format!("attributes: {err}"))?
    {
        let value = attributes::read_i64(source, &name)
            .ok_or_else(|| format!("attribute {name} is not a whole number"))?;
        attributes::write_i64(target, &name, value)
            .map_err(|err| format!("attribute {name}: {err}"))?;
    }
    Ok(())
}

/// Opening an archive from before the paged strategy rebuilds it, and the day
/// it held reads back through the store as it was stored.
#[test]
fn an_archive_from_before_the_paged_strategy_is_rebuilt_and_reads_back() {
    let (day, parsed) = captured_day().expect("a captured day");
    let fetched_at = DateTime::<Utc>::default();
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join(FILE_NAME);
    {
        let store = JamStore::open_or_create(&path).expect("create");
        store
            .insert_day(day, HOST, fetched_at, &parsed)
            .expect("insert");
    }
    rewrite_with_the_default_file_space(&path).expect("rewrite");
    assert_eq!(
        file_space_strategy(&path).expect("strategy"),
        FileSpaceStrategy::default(),
        "the archive under test is not the one an older version left"
    );

    let store = JamStore::open_or_create(&path).expect("open");

    assert_eq!(
        file_space_strategy(&path).expect("strategy"),
        PAGED_FILE_SPACE
    );
    assert_eq!(store.observations(day).expect("read back"), Some(parsed));
    let stored = store.days().expect("days");
    assert_eq!(stored.len(), 1);
    assert_eq!(
        stored
            .first()
            .map(|entry| (entry.day, entry.host.as_str(), entry.fetched_at)),
        Some((day, HOST, fetched_at))
    );
    assert!(
        !dir.path().join(format!("{FILE_NAME}.rebuilding")).exists(),
        "the rebuild left its file behind"
    );
}

/// A rebuilt archive takes days the way it did before: the day index and the
/// columns it names still agree.
#[test]
fn a_rebuilt_archive_stores_days_again() {
    let (day, parsed) = captured_day().expect("a captured day");
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join(FILE_NAME);
    {
        let store = JamStore::open_or_create(&path).expect("create");
        store
            .insert_day(day, HOST, DateTime::<Utc>::default(), &parsed)
            .expect("insert");
    }
    rewrite_with_the_default_file_space(&path).expect("rewrite");

    let store = JamStore::open_or_create(&path).expect("open");
    let next = day + TimeDelta::days(1);
    store
        .insert_day(next, HOST, DateTime::<Utc>::default(), &parsed)
        .expect("insert after the rebuild");

    assert_eq!(store.observations(next).expect("read back"), Some(parsed));
    assert_eq!(
        store
            .days()
            .expect("days")
            .into_iter()
            .map(|entry| entry.day)
            .collect::<Vec<_>>(),
        [day, next]
    );
}

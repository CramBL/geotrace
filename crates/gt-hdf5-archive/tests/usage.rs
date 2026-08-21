//! What an archive reports about its size and the days it holds.

use chrono::NaiveDate;
use gt_hdf5_archive::{ArchiveUsage, ArchivedDaySpan};

fn day(offset: i64) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 8, 10).unwrap_or_default() + chrono::TimeDelta::days(offset)
}

fn span(oldest: i64, newest: i64) -> Option<ArchivedDaySpan> {
    Some(ArchivedDaySpan {
        oldest: day(oldest),
        newest: day(newest),
    })
}

#[test]
fn an_archive_reports_its_size_and_the_days_it_spans() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("archive.h5");
    std::fs::write(&path, [0_u8; 128]).expect("write");

    let usage = ArchiveUsage::measure(&path, [day(3), day(0), day(1)]);

    assert_eq!(usage.bytes, Some(128));
    assert_eq!(usage.days, 3);
    assert_eq!(usage.span, span(0, 3));
    assert!(!usage.is_empty());
}

/// An archive whose file is not there yet reports no size, which the rows
/// show as an absent value.
#[test]
fn an_archive_without_a_file_reports_no_size() {
    let dir = tempfile::tempdir().expect("temp dir");

    let usage = ArchiveUsage::measure(&dir.path().join("absent.h5"), []);

    assert_eq!(usage.bytes, None);
    assert_eq!(usage.days, 0);
    assert_eq!(usage.span, None);
    assert!(usage.is_empty());
}

#[test]
fn the_total_adds_the_sizes_and_covers_every_span() {
    let total = ArchiveUsage::total([
        ArchiveUsage {
            bytes: Some(400),
            days: 2,
            span: span(0, 1),
        },
        ArchiveUsage {
            bytes: Some(600),
            days: 3,
            span: span(4, 9),
        },
        ArchiveUsage {
            bytes: Some(0),
            days: 0,
            span: None,
        },
    ]);

    assert_eq!(total.bytes, Some(1_000));
    assert_eq!(total.days, 5);
    assert_eq!(total.span, span(0, 9));
}

/// A size that could not be read leaves the total unknown: a total that
/// silently left one archive out would understate what pruning frees.
#[test]
fn a_size_that_could_not_be_read_leaves_the_total_unknown() {
    let total = ArchiveUsage::total([
        ArchiveUsage {
            bytes: Some(400),
            days: 1,
            span: span(0, 0),
        },
        ArchiveUsage {
            bytes: None,
            days: 1,
            span: span(2, 2),
        },
    ]);

    assert_eq!(total.bytes, None);
    assert_eq!(total.days, 2);
    assert_eq!(total.span, span(0, 2));
}

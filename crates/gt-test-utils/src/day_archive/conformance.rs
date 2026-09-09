//! The rules every day archive follows, which each store runs over its own
//! payload. A case reports its failure by panicking with what the archive did
//! instead.
#![expect(
    clippy::expect_used,
    reason = "a conformance case reports its failure by panicking"
)]

use std::fmt::Debug;

use chrono::NaiveDate;
use gt_hdf5_archive::prune::InterruptedDeleteRecovery;
use gt_hdf5_archive::{
    DayArchiveError, ReadOnlyDayArchive as _, SchemaVersions, WritableDayArchive,
};

use crate::day_archive;

/// What one store does with one day of its own payload.
pub struct StoredDayOperations<A, D> {
    /// Writes one day into the archive, returning the payload it wrote.
    pub insert_a_day: fn(&A, NaiveDate) -> Result<D, String>,

    /// What the archive holds for one day, [`None`] where it holds nothing
    /// for that day.
    pub read_a_day: fn(&A, NaiveDate) -> Result<Option<D>, String>,

    /// The days the archive's day index holds, oldest first.
    pub indexed_days: fn(&A) -> Result<Vec<NaiveDate>, String>,
}

impl<A, D> Clone for StoredDayOperations<A, D> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<A, D> Copy for StoredDayOperations<A, D> {}

/// An archive whose file states a schema version this build does not read
/// fails to open, and the failure states that version beside the newest this
/// build reads.
pub fn a_newer_schema_is_rejected<A>()
where
    A: WritableDayArchive,
    A::Error: DayArchiveError,
{
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join(A::ReadOnly::FILE_NAME);
    A::open_or_create(&path).expect("the archive is created");
    let supported = A::ReadOnly::CURRENT_SCHEMA_VERSION;
    {
        let file = hdf5::File::open_rw(&path).expect("the archive reopens for writing");
        file.attr(A::ReadOnly::SCHEMA_VERSION_ATTR)
            .and_then(|attr| attr.write_scalar(&(supported + 1)))
            .expect("the schema version is written past this build's");
    }

    let err = A::open_or_create(&path)
        .err()
        .expect("an archive of a newer schema is rejected");

    assert_eq!(
        err.schema_too_new(),
        Some(SchemaVersions {
            found: supported + 1,
            supported,
        }),
        "{err}"
    );
}

/// A day written into an archive is in it again once the file is closed and
/// opened afresh.
pub fn an_archive_reopens_with_its_days<A, D>(days: &StoredDayOperations<A, D>, day: NaiveDate)
where
    A: WritableDayArchive,
    A::Error: DayArchiveError,
    D: Debug + PartialEq,
{
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join(A::ReadOnly::FILE_NAME);
    let written = {
        let store = A::open_or_create(&path).expect("the archive is created");
        (days.insert_a_day)(&store, day).expect("a day is inserted")
    };

    let reopened = A::open_or_create(&path).expect("the archive reopens");

    assert_eq!(
        (days.read_a_day)(&reopened, day).expect("the day reads back"),
        Some(written)
    );
}

/// A date written into the day index reads back as itself, whatever era it
/// falls in.
pub fn any_date_round_trips_through_the_day_index<A, D>(
    days: &StoredDayOperations<A, D>,
    date: NaiveDate,
) where
    A: WritableDayArchive,
    A::Error: DayArchiveError,
    D: Debug + PartialEq,
{
    let (_dir, store) = day_archive::store_in_a_temp_dir::<A>().expect("an archive in a temp dir");

    (days.insert_a_day)(&store, date).expect("a day is inserted");

    assert_eq!(
        (days.indexed_days)(&store).expect("the indexed days"),
        [date]
    );
}

/// Deleting every day leaves a day index holding nothing, and each day it
/// held reads back as nothing.
pub fn deleting_every_day_empties_the_archive<A, D>(
    days: &StoredDayOperations<A, D>,
    dates: &[NaiveDate],
) where
    A: WritableDayArchive,
    A::Error: DayArchiveError,
    D: Debug + PartialEq,
{
    let (_dir, store) = day_archive::store_in_a_temp_dir::<A>().expect("an archive in a temp dir");
    for &date in dates {
        (days.insert_a_day)(&store, date).expect("a day is inserted");
    }

    let removed = store.delete_all_days(None).expect("every day is deleted");

    assert_eq!(removed, dates.len());
    assert_eq!((days.indexed_days)(&store).expect("the indexed days"), []);
    for &date in dates {
        assert_eq!(
            (days.read_a_day)(&store, date).expect("a deleted day"),
            None,
            "{date} is still archived"
        );
    }
}

/// A settled archive has nothing to recover in three states: before the file
/// exists, once it is created empty, and once it holds a day. An open that
/// declines the recovery then succeeds and finds the day still indexed.
pub fn a_settled_archive_reports_no_interrupted_delete<A, D>(
    days: &StoredDayOperations<A, D>,
    day: NaiveDate,
) where
    A: WritableDayArchive,
    A::Error: DayArchiveError,
    D: Debug + PartialEq,
{
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join(A::ReadOnly::FILE_NAME);
    assert_eq!(
        A::ReadOnly::interrupted_delete_at(&path).expect("before the archive exists"),
        None
    );

    A::open_or_create(&path).expect("the archive is created");

    assert_eq!(
        A::ReadOnly::interrupted_delete_at(&path).expect("an archive holding nothing"),
        None
    );
    {
        let store = A::open_or_create(&path).expect("the archive reopens");
        (days.insert_a_day)(&store, day).expect("a day is inserted");
    }

    assert_eq!(
        A::ReadOnly::interrupted_delete_at(&path).expect("a settled archive"),
        None
    );
    let reopened =
        A::open_or_create_with_recovery_choice(&path, InterruptedDeleteRecovery::Decline)
            .expect("a settled archive opens whatever the recovery choice");

    assert_eq!(
        (days.indexed_days)(&reopened).expect("the indexed days"),
        [day]
    );
}

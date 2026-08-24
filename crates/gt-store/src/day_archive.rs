//! The four day archives a [`Store`] holds: the file name and the shared
//! slot of each, and what its error reports.
//!
//! Each archive has its own error type, and a caller opening all four wants
//! the same two answers from every one of them: whether another process has
//! the file, and whether an open left an interrupted delete unrecovered
//! because it was told to.

use gt_flare_store::{FlareStore, FlareStoreError};
use gt_ionex_store::{IonexStore, IonexStoreError};
use gt_jam_store::{JamStore, JamStoreError};
use gt_solar_store::{SolarStore, SolarStoreError};

use crate::{DeclinedRecovery, InterruptedDelete, SharedArchive, Store, WritableDayArchive};

/// A day archive [`Store`] keeps a file for.
///
/// Implemented for the four archives in this crate: [`Self::shared_in`]
/// returns a private field of [`Store`].
pub trait StoredDayArchive: WritableDayArchive {
    /// Name of this archive's file under [`Store::root`].
    const FILE_NAME: &'static str;

    fn shared_in(store: &Store) -> &SharedArchive<Self, Self::ReadOnly>;
}

/// The failures a caller opening every day archive acts on, whichever archive
/// reported one.
pub trait DayArchiveError: std::error::Error {
    /// Another process has the file open. libhdf5 takes an OS lock for the
    /// duration of an open, readers included, so nothing here can read it
    /// until that process lets go.
    fn is_held_by_another_process(&self) -> bool;

    /// The interrupted delete an open was told not to recover, which left the
    /// file untouched, or [`None`] for any other failure.
    fn interrupted_delete_left_unrecovered(&self) -> Option<InterruptedDelete>;
}

/// Implements [`StoredDayArchive`] for each archive listed, and
/// [`DayArchiveError`] for its error type, which has a `HeldByAnotherProcess`
/// and a `DeclinedRecovery` variant.
macro_rules! stored_day_archives {
    ($($writable:ty {
        error: $error:ty,
        file_name: $file_name:expr,
        shared_from: $slot:ident,
    })+) => {
        $(
            impl StoredDayArchive for $writable {
                const FILE_NAME: &'static str = $file_name;

                fn shared_in(store: &Store) -> &SharedArchive<Self, Self::ReadOnly> {
                    &store.$slot
                }
            }

            impl DayArchiveError for $error {
                fn is_held_by_another_process(&self) -> bool {
                    matches!(self, Self::HeldByAnotherProcess)
                }

                fn interrupted_delete_left_unrecovered(&self) -> Option<InterruptedDelete> {
                    match self {
                        Self::DeclinedRecovery(DeclinedRecovery(interrupted)) => Some(*interrupted),
                        _ => None,
                    }
                }
            }
        )+
    };
}

stored_day_archives! {
    JamStore {
        error: JamStoreError,
        file_name: gt_jam_store::FILE_NAME,
        shared_from: interference,
    }
    SolarStore {
        error: SolarStoreError,
        file_name: gt_solar_store::FILE_NAME,
        shared_from: geomagnetic_indices,
    }
    IonexStore {
        error: IonexStoreError,
        file_name: gt_ionex_store::FILE_NAME,
        shared_from: tec_maps,
    }
    FlareStore {
        error: FlareStoreError,
        file_name: gt_flare_store::FILE_NAME,
        shared_from: solar_flares,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const INTERRUPTED: InterruptedDelete = InterruptedDelete { archived_days: 3 };

    fn answers<E: DayArchiveError>(err: &E) -> (bool, Option<InterruptedDelete>) {
        (
            err.is_held_by_another_process(),
            err.interrupted_delete_left_unrecovered(),
        )
    }

    #[test]
    fn every_archive_answers_both_questions_through_its_own_error() {
        let held = (true, None);
        let declined = (false, Some(INTERRUPTED));

        assert_eq!(answers(&JamStoreError::HeldByAnotherProcess), held);
        assert_eq!(answers(&SolarStoreError::HeldByAnotherProcess), held);
        assert_eq!(answers(&IonexStoreError::HeldByAnotherProcess), held);
        assert_eq!(answers(&FlareStoreError::HeldByAnotherProcess), held);

        assert_eq!(
            answers(&JamStoreError::from(DeclinedRecovery(INTERRUPTED))),
            declined
        );
        assert_eq!(
            answers(&SolarStoreError::from(DeclinedRecovery(INTERRUPTED))),
            declined
        );
        assert_eq!(
            answers(&IonexStoreError::from(DeclinedRecovery(INTERRUPTED))),
            declined
        );
        assert_eq!(
            answers(&FlareStoreError::from(DeclinedRecovery(INTERRUPTED))),
            declined
        );
    }
}

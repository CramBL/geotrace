//! What the four day archives report in common.
//!
//! Each archive has its own error type, and a caller opening all four wants
//! the same two answers from every one of them: whether another process has
//! the file, and whether an open left an interrupted delete unrecovered
//! because it was told to.

use gt_flare_store::FlareStoreError;
use gt_ionex_store::IonexStoreError;
use gt_jam_store::JamStoreError;
use gt_solar_store::SolarStoreError;

use crate::{DeclinedRecovery, InterruptedDelete};

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

/// Implements [`DayArchiveError`] for an error type with a
/// `HeldByAnotherProcess` and a `DeclinedRecovery` variant.
macro_rules! impl_day_archive_error {
    ($($error:ty),+ $(,)?) => {
        $(
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

impl_day_archive_error!(
    JamStoreError,
    SolarStoreError,
    IonexStoreError,
    FlareStoreError,
);

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

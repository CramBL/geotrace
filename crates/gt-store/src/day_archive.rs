//! The four day archives a [`Store`] holds: which they are, where each one is
//! stored, and what its error reports.
//!
//! Each archive has its own error type, and a caller opening all four wants
//! the same two answers from every one of them: whether another process has
//! the file, and whether an open left an interrupted delete unrecovered
//! because it was told to.

use std::path::PathBuf;

use chrono::NaiveDate;
use gt_flare_store::{FlareStore, FlareStoreError};
use gt_ionex_store::{IonexStore, IonexStoreError};
use gt_jam_store::{JamStore, JamStoreError};
use gt_pending_writes::{WriteKind, WriteRegistration};
use gt_solar_store::{SolarStore, SolarStoreError};
use strum::{EnumCount, EnumIter};

use crate::{DeclinedRecovery, InterruptedDelete, SharedArchive, Store, WritableDayArchive};

/// One of the archives, as the settings rows and the delete controls name it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, EnumCount, EnumIter)]
pub enum EnvironmentArchive {
    AircraftInterference,
    GeomagneticIndices,
    IonosphericTec,
    SolarFlares,
}

impl EnvironmentArchive {
    pub const fn label(self) -> &'static str {
        match self {
            Self::AircraftInterference => gt_jam::text::LAYER_LABEL,
            Self::GeomagneticIndices => "Geomagnetic indices",
            Self::IonosphericTec => "Ionospheric TEC",
            Self::SolarFlares => gt_flare::text::LAYER_LABEL,
        }
    }

    /// The label as it reads inside a sentence, where only an acronym keeps
    /// its capitals.
    pub const fn label_in_sentence(self) -> &'static str {
        match self {
            Self::AircraftInterference => "aircraft interference",
            Self::GeomagneticIndices => "geomagnetic indices",
            Self::IonosphericTec => "ionospheric TEC",
            Self::SolarFlares => "solar flares",
        }
    }

    /// Path of this archive's file under `store`.
    pub fn path_in(self, store: &Store) -> PathBuf {
        store.root.join(self.file_name())
    }

    /// What the insert of one downloaded day registers under.
    pub fn day_insert_registration(self, day: NaiveDate) -> WriteRegistration {
        WriteRegistration {
            label: format!("Archiving {} for {day}", self.label_in_sentence()),
            kind: WriteKind::ArchiveDayInsert {
                archive: self.label_in_sentence(),
            },
        }
    }

    /// What the rewrite that deletes this archive's days registers under.
    pub fn day_delete_registration(self) -> WriteRegistration {
        WriteRegistration {
            label: format!("Deleting {} days", self.label_in_sentence()),
            kind: WriteKind::ArchiveCompaction {
                archive: self.label_in_sentence(),
            },
        }
    }
}

/// A day archive [`Store`] keeps a file for.
///
/// Implemented for the four archives in this crate: [`Self::shared_in`]
/// returns a private field of [`Store`].
pub trait StoredDayArchive: WritableDayArchive<Error: DayArchiveError> {
    /// Which of the four archives this type stores.
    const ARCHIVE: EnvironmentArchive;

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

/// Implements [`StoredDayArchive`] for each archive listed, [`DayArchiveError`]
/// for its error type, which has a `HeldByAnotherProcess` and a
/// `DeclinedRecovery` variant, and `EnvironmentArchive::file_name` over the
/// four variants.
macro_rules! stored_day_archives {
    ($($writable:ty {
        archive: $variant:ident,
        error: $error:ty,
        file_name: $file_name:expr,
        shared_from: $slot:ident,
    })+) => {
        impl EnvironmentArchive {
            const fn file_name(self) -> &'static str {
                match self {
                    $(Self::$variant => $file_name,)+
                }
            }
        }

        $(
            impl StoredDayArchive for $writable {
                const ARCHIVE: EnvironmentArchive = EnvironmentArchive::$variant;

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
        archive: AircraftInterference,
        error: JamStoreError,
        file_name: gt_jam_store::FILE_NAME,
        shared_from: interference,
    }
    SolarStore {
        archive: GeomagneticIndices,
        error: SolarStoreError,
        file_name: gt_solar_store::FILE_NAME,
        shared_from: geomagnetic_indices,
    }
    IonexStore {
        archive: IonosphericTec,
        error: IonexStoreError,
        file_name: gt_ionex_store::FILE_NAME,
        shared_from: tec_maps,
    }
    FlareStore {
        archive: SolarFlares,
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

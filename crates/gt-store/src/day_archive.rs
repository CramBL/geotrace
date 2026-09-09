//! The four day archives a [`Store`] holds: which they are, and where each
//! one is stored.
//!
//! A caller opening all four reads the same failures from each. Every archive
//! has an error type of its own, and each of those implements
//! [`DayArchiveError`].

use std::path::PathBuf;

use chrono::NaiveDate;
use gt_flare_store::FlareStore;
use gt_ionex_store::IonexStore;
use gt_jam_store::JamStore;
use gt_pending_writes::{WriteKind, WriteRegistration};
use gt_solar_store::SolarStore;
use strum::{EnumCount, EnumIter};

use crate::{DayArchiveError, SharedArchive, Store, WritableDayArchive};

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

/// Implements [`StoredDayArchive`] for each archive listed, and
/// `EnvironmentArchive::file_name` over the four variants.
macro_rules! stored_day_archives {
    ($($writable:ty {
        archive: $variant:ident,
        shared_from: $slot:ident,
    })+) => {
        impl EnvironmentArchive {
            const fn file_name(self) -> &'static str {
                match self {
                    $(Self::$variant => <<$writable as WritableDayArchive>::ReadOnly
                        as crate::ReadOnlyDayArchive>::FILE_NAME,)+
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
        )+
    };
}

stored_day_archives! {
    JamStore {
        archive: AircraftInterference,
        shared_from: interference,
    }
    SolarStore {
        archive: GeomagneticIndices,
        shared_from: geomagnetic_indices,
    }
    IonexStore {
        archive: IonosphericTec,
        shared_from: tec_maps,
    }
    FlareStore {
        archive: SolarFlares,
        shared_from: solar_flares,
    }
}

#[cfg(test)]
mod tests {
    use gt_flare_store::FlareStoreError;
    use gt_ionex_store::IonexStoreError;
    use gt_jam_store::JamStoreError;
    use gt_solar_store::SolarStoreError;

    use super::*;
    use crate::{DeclinedRecovery, InterruptedDelete, SchemaVersions};

    const INTERRUPTED: InterruptedDelete = InterruptedDelete { archived_days: 3 };

    fn held_and_unrecovered<E: DayArchiveError>(err: &E) -> (bool, Option<InterruptedDelete>) {
        (
            err.is_held_by_another_process(),
            err.interrupted_delete_left_unrecovered(),
        )
    }

    #[test]
    fn every_archive_reports_a_newer_schema_through_its_own_error() {
        let versions = SchemaVersions {
            found: 3,
            supported: 2,
        };
        let too_new = Some(versions);

        let each_archive: [Box<dyn DayArchiveError>; 4] = [
            Box::new(JamStoreError::SchemaTooNew {
                found: versions.found,
                supported: versions.supported,
            }),
            Box::new(SolarStoreError::SchemaTooNew {
                found: versions.found,
                supported: versions.supported,
            }),
            Box::new(IonexStoreError::SchemaTooNew {
                found: versions.found,
                supported: versions.supported,
            }),
            Box::new(FlareStoreError::SchemaTooNew {
                found: versions.found,
                supported: versions.supported,
            }),
        ];

        for err in &each_archive {
            assert_eq!(err.schema_too_new(), too_new);
        }
        assert_eq!(JamStoreError::HeldByAnotherProcess.schema_too_new(), None);
    }

    #[test]
    fn every_archive_reports_both_failures_through_its_own_error() {
        let held = (true, None);
        let declined = (false, Some(INTERRUPTED));

        assert_eq!(
            held_and_unrecovered(&JamStoreError::HeldByAnotherProcess),
            held
        );
        assert_eq!(
            held_and_unrecovered(&SolarStoreError::HeldByAnotherProcess),
            held
        );
        assert_eq!(
            held_and_unrecovered(&IonexStoreError::HeldByAnotherProcess),
            held
        );
        assert_eq!(
            held_and_unrecovered(&FlareStoreError::HeldByAnotherProcess),
            held
        );

        assert_eq!(
            held_and_unrecovered(&JamStoreError::from(DeclinedRecovery(INTERRUPTED))),
            declined
        );
        assert_eq!(
            held_and_unrecovered(&SolarStoreError::from(DeclinedRecovery(INTERRUPTED))),
            declined
        );
        assert_eq!(
            held_and_unrecovered(&IonexStoreError::from(DeclinedRecovery(INTERRUPTED))),
            declined
        );
        assert_eq!(
            held_and_unrecovered(&FlareStoreError::from(DeclinedRecovery(INTERRUPTED))),
            declined
        );
    }
}

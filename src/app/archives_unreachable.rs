//! Why the delete controls and the download controls are both grayed: this
//! session has no archive open to write to.

use super::App;
use super::storage::DatabasesPending;

/// Why this session has no archive open to write to.
///
/// No hover text lives here: the wording for these four states differs
/// between [`super::environment_storage_ui::DeleteBlocker`] and
/// [`super::backfill_ui::BackfillReadiness`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchivesUnreachable {
    /// This session reads the archives beside the instance that owns the data
    /// directory, and changes none of them.
    ReadOnlySession,
    /// This instance does not have the data directory, so it has opened no
    /// archive.
    WaitingForTheDataDirectory,
    /// The open is waiting for the user's choice about an archive a delete
    /// was interrupted in.
    AwaitingAnInterruptedDeleteChoice,
    ArchivesOpening,
}

impl App {
    /// [`None`] once the open has finished in a session that may write.
    pub(super) fn archives_unreachable(&self) -> Option<ArchivesUnreachable> {
        if !self.pending_writes.write_access().allows_writing() {
            return Some(ArchivesUnreachable::ReadOnlySession);
        }
        match self.storage_open.databases_pending()? {
            DatabasesPending::WaitingForTheDataDirectory => {
                Some(ArchivesUnreachable::WaitingForTheDataDirectory)
            }
            DatabasesPending::AwaitingAnInterruptedDeleteChoice => {
                Some(ArchivesUnreachable::AwaitingAnInterruptedDeleteChoice)
            }
            DatabasesPending::Opening => Some(ArchivesUnreachable::ArchivesOpening),
        }
    }
}

use std::collections::BTreeSet;
#[cfg(windows)]
use std::path::{Component, Prefix};
use std::path::{Path, PathBuf};

use gt_session_bridge::{ErrorCode, ManageRecordings, RecordingBatch, RecordingReference, Reply};
use gt_side_panel::NodeKey;
use uuid::Uuid;

use super::App;
use super::loader::GtdLoadMode;
use super::mcp_action_notice::McpActionNotice;
use super::recording_from_disk::ArrivingFileKind;
use super::session_operations::{INITIAL_OPERATION_WAIT, SessionOperationStatus};
use super::session_query::SessionCommand;
use super::storage::QueuedLoad;

impl App {
    pub(super) fn manage_recordings(
        &mut self,
        session: Uuid,
        request: ManageRecordings,
    ) -> Result<SessionCommand, ErrorCode> {
        if self.shutdown_requested() {
            return Err(ErrorCode::ShuttingDown);
        }
        match request {
            ManageRecordings::Open { paths } => self.open_session_files(session, paths),
            ManageRecordings::Unload { recordings } => self
                .unload_session_recordings(session, recordings)
                .map(SessionCommand::Completed),
        }
    }

    fn open_session_files(
        &mut self,
        session: Uuid,
        paths: RecordingBatch<PathBuf>,
    ) -> Result<SessionCommand, ErrorCode> {
        let mut seen = BTreeSet::new();
        let mut requested_paths = Vec::new();
        let mut arriving = Vec::new();
        for path in paths.into_items() {
            self::validate_local_path(&path)?;
            if ArrivingFileKind::of_path(&path).is_none() {
                return Err(ErrorCode::UnsupportedFileExtension);
            }
            if seen.insert(path.as_os_str().to_owned()) {
                requested_paths.push(path.clone());
                arriving.push(QueuedLoad::Path {
                    path,
                    mode: GtdLoadMode::Regular,
                });
            }
        }
        let (submission, wait, files) = self
            .association_batches
            .begin_session_submission(session, requested_paths);
        self.submit_arriving_files(arriving, Some(submission));
        McpActionNotice::QueueFiles.enqueue(&mut self.toasts);
        Ok(SessionCommand::Operation {
            status: SessionOperationStatus::Files { wait, files },
            wait: INITIAL_OPERATION_WAIT,
        })
    }

    fn unload_session_recordings(
        &mut self,
        session: Uuid,
        recordings: RecordingBatch<RecordingReference>,
    ) -> Result<Reply, ErrorCode> {
        let file_indices = {
            let shared = self.shared.borrow();
            let files = shared.loaded_files.view();
            recordings
                .into_items()
                .into_iter()
                .map(|recording| {
                    recording
                        .resolve(files, session)
                        .map(|entry| entry.file_index())
                })
                .collect::<Result<BTreeSet<_>, _>>()?
        };
        let items: Vec<_> = file_indices.into_iter().map(NodeKey::File).collect();
        self.unload_items_from_view(&items);
        McpActionNotice::UnloadRecordings.enqueue(&mut self.toasts);
        Ok(Reply::RecordingsUnloaded {
            unloaded_recordings: items.len(),
        })
    }
}

fn validate_local_path(path: &Path) -> Result<(), ErrorCode> {
    if !path.is_absolute() || path.as_os_str().as_encoded_bytes().contains(&0) {
        return Err(ErrorCode::InvalidLocalFilePath);
    }
    #[cfg(windows)]
    if !matches!(path.components().next(), Some(Component::Prefix(prefix))
        if matches!(prefix.kind(), Prefix::Disk(_) | Prefix::VerbatimDisk(_)))
    {
        return Err(ErrorCode::InvalidLocalFilePath);
    }
    Ok(())
}

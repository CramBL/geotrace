use std::fmt;
use std::path::PathBuf;

use gt_log_view::LogAttachmentRef;
use gt_store::DatabaseRef;
use gt_track_builder::SegmentationConfig;
use strum::Display;

use crate::app::loader::{GtdLoadMode, HistoryOpen, LoadedRecordingPlacement};

#[derive(Clone, Debug)]
pub(in crate::app) enum LoadReadRequest {
    OpenStoredRecording {
        recording: DatabaseRef,
        placement: LoadedRecordingPlacement,
    },
    ReadAttachment(LogAttachmentRef),
    ReadLogFile(PathBuf),
    ReadRecordingFile(Box<RecordingFileRead>),
}

impl fmt::Display for LoadReadRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ReadAttachment(attachment) => write!(
                f,
                "attachment {} in {}",
                attachment.id, attachment.recording
            ),
            Self::ReadLogFile(path) => write!(f, "log file {}", path.display()),
            Self::ReadRecordingFile(request) => {
                write!(f, "recording file {}", request.path.display())
            }
            Self::OpenStoredRecording {
                recording,
                placement,
            } => write!(f, "stored recording {recording} ({placement:?})"),
        }
    }
}

#[derive(Clone, Debug)]
pub(in crate::app) struct RecordingFileRead {
    pub path: PathBuf,
    pub config: SegmentationConfig,
    pub mode: GtdLoadMode,
    pub open: Option<HistoryOpen>,
}

pub(in crate::app) enum LoadRecovery<'a> {
    ChooseFile,
    EnterText(&'a str),
    Retry(&'a LoadReadRequest),
}

impl fmt::Display for LoadRecovery<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ChooseFile => f.write_str("Choose file"),
            Self::EnterText(text) => write!(f, "Enter text ({} bytes retained)", text.len()),
            Self::Retry(request) => write!(f, "Retry {request}"),
        }
    }
}

#[derive(Clone, Copy, Debug, Display, Eq, PartialEq)]
pub(in crate::app) enum LoadFailureKind {
    #[strum(serialize = "Empty input")]
    RejectEmptyInput,
    #[strum(serialize = "Unrecognized format")]
    RejectUnrecognizedFormat,
    #[strum(serialize = "Unsupported attachment")]
    RejectUnsupportedAttachment,
    #[strum(serialize = "Unsupported recording")]
    RejectUnsupportedRecording,
    #[strum(serialize = "Attachment unavailable")]
    ReportUnavailableAttachment,
    #[strum(serialize = "Unreadable attachment")]
    ReportUnreadableAttachment,
    #[strum(serialize = "Unreadable file")]
    ReportUnreadableFile,
    #[strum(serialize = "Unreadable recording")]
    ReportUnreadableRecording,
}

#[cfg(test)]
mod tests {
    use std::io;

    use gt_loader::{LoadError, SdkError};
    use gt_store::{DbError, LogAttachmentError, LogAttachmentId};
    use rstest::rstest;

    use super::super::{LoadFailure, LoadFailureCause, LoadSource};
    use super::*;

    #[test]
    fn a_display_filename_does_not_enable_a_file_read() {
        let source = LoadSource::Filename("/display/name.log".into());
        let failure = LoadFailure::new(
            source.clone(),
            LoadFailureCause::ReadLogFile(io::ErrorKind::NotFound.into()),
        );
        assert!(matches!(failure.recovery(), LoadRecovery::ChooseFile));
        let request = LoadReadRequest::ReadLogFile(PathBuf::from("/original/input.log"));
        let failure = LoadFailure::new(
            source,
            LoadFailureCause::ReadLogFile(io::ErrorKind::NotFound.into()),
        )
        .with_original_read(Some(request));
        let LoadRecovery::Retry(LoadReadRequest::ReadLogFile(path)) = failure.recovery() else {
            panic!("the real read can be repeated");
        };
        assert_eq!(path, &PathBuf::from("/original/input.log"));
        assert!(
            failure
                .copy_text()
                .contains("Original read: log file /original/input.log")
        );
    }

    #[rstest]
    #[case::open_io(LoadError::Io(io::ErrorKind::PermissionDenied.into()), LoadFailureKind::ReportUnreadableRecording, true)]
    #[case::read_io(LoadError::Sdk(SdkError::Io(io::ErrorKind::Interrupted.into())), LoadFailureKind::ReportUnreadableRecording, true)]
    #[case::format(LoadError::Sdk(SdkError::UnsupportedVersion { version: "99".into() }), LoadFailureKind::RejectUnsupportedRecording, false)]
    fn recording_recovery_distinguishes_io_from_format_errors(
        #[case] error: LoadError,
        #[case] kind: LoadFailureKind,
        #[case] retry: bool,
    ) {
        let failure = LoadFailure::new(
            LoadSource::Filename("display.gtd".into()),
            LoadFailureCause::LoadRecording(Box::new(error)),
        )
        .with_original_read(Some(LoadReadRequest::ReadRecordingFile(Box::new(
            RecordingFileRead {
                path: PathBuf::from("/original/input.gtd"),
                config: SegmentationConfig::default(),
                mode: GtdLoadMode::Regular,
                open: None,
            },
        ))));
        assert_eq!(failure.cause.kind(), kind);
        match (failure.recovery(), retry) {
            (LoadRecovery::Retry(LoadReadRequest::ReadRecordingFile(request)), true) => {
                assert_eq!(request.path, PathBuf::from("/original/input.gtd"));
            }
            (LoadRecovery::ChooseFile, false) => {}
            _ => panic!("recovery matches the recording failure"),
        }
    }

    #[rstest]
    #[case::io(LogAttachmentError::Io { path: PathBuf::from("attachment.zst"), source: io::ErrorKind::PermissionDenied.into() }, LoadFailureKind::ReportUnreadableAttachment, true)]
    #[case::missing(LogAttachmentError::UnknownAttachment { id: LogAttachmentId::new_random() }, LoadFailureKind::ReportUnavailableAttachment, false)]
    #[case::unsupported(LogAttachmentError::Database(DbError::SchemaTooNew { found: 99, supported: 1 }), LoadFailureKind::RejectUnsupportedAttachment, false)]
    fn attachment_recovery_depends_on_the_structured_cause(
        #[case] error: LogAttachmentError,
        #[case] kind: LoadFailureKind,
        #[case] retry: bool,
    ) {
        let attachment = LogAttachmentRef {
            recording: DatabaseRef {
                identity: "device".into(),
                group_name: "recording".into(),
            },
            id: LogAttachmentId::new_random(),
        };
        let failure = LoadFailure::new(
            LoadSource::Attachment {
                attachment: attachment.clone(),
                name: "display.log".into(),
            },
            LoadFailureCause::ReadLogAttachment(Box::new(error)),
        )
        .with_original_read(Some(LoadReadRequest::ReadAttachment(attachment.clone())));
        assert_eq!(failure.cause.kind(), kind);
        assert_eq!(matches!(failure.recovery(), LoadRecovery::Retry(_)), retry);
        if let LoadRecovery::Retry(LoadReadRequest::ReadAttachment(original)) = failure.recovery() {
            assert_eq!(original, &attachment);
        }
    }
}

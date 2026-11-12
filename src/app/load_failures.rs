use std::path::Path;
use std::{fmt, io, mem};

use egui::{Button, Label, TextEdit, Window};
use egui_phosphor::regular::COPY as ICON_COPY;
use gt_loader::{LoadError, SdkError};
use gt_log_view::LogAttachmentRef;
use gt_logfile::{LogParseError, LogText};
use gt_store::{DatabaseRef, DbError, LogAttachmentError, LogAttachmentReadError};
use strum::Display;

use crate::app::modals::{self, DialogActionRow, DialogBody};

pub(super) use recovery::{LoadFailureKind, LoadReadRequest, LoadRecovery, RecordingFileRead};
pub(super) use retry::{LoadFailureId, LoadProblemRequest, LoadRetry, LoadRetryAttempt};

mod recovery;
mod retry;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum LoadSource {
    Attachment {
        attachment: LogAttachmentRef,
        name: String,
    },
    Filename(String),
    PastedText,
    StoredRecording(DatabaseRef),
    UnnamedDrop,
}

impl LoadSource {
    pub(super) fn into_filename(self) -> Option<String> {
        match self {
            Self::Attachment { name, .. } | Self::Filename(name) => Some(name),
            Self::PastedText | Self::StoredRecording(_) | Self::UnnamedDrop => None,
        }
    }
}

impl fmt::Display for LoadSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Attachment { attachment, name } => write!(
                f,
                "{name} in recording {} (attachment {})",
                attachment.recording, attachment.id
            ),
            Self::Filename(filename) => f.write_str(filename),
            Self::PastedText => f.write_str("pasted log text"),
            Self::StoredRecording(recording) => write!(f, "recording {recording}"),
            Self::UnnamedDrop => f.write_str("unnamed dropped log"),
        }
    }
}

#[derive(Debug)]
pub(super) struct LoadFailure {
    pub source: LoadSource,
    pub cause: LoadFailureCause,
    input: Box<LoadRecoveryInput>,
}

impl LoadFailure {
    pub(super) fn new(source: LoadSource, cause: LoadFailureCause) -> Self {
        Self {
            source,
            cause,
            input: Box::default(),
        }
    }

    pub(super) fn with_original_read(mut self, request: Option<LoadReadRequest>) -> Self {
        self.input.original_read = request;
        self
    }

    pub(super) fn with_decoded_text(mut self, text: LogText) -> Self {
        self.input.decoded_text = Some(text);
        self
    }

    pub(super) fn original_read(&self) -> Option<&LoadReadRequest> {
        self.input.original_read.as_ref()
    }

    pub(super) fn recovery(&self) -> LoadRecovery<'_> {
        match self.cause.kind() {
            LoadFailureKind::ReportUnreadableAttachment
            | LoadFailureKind::ReportUnreadableFile
            | LoadFailureKind::ReportUnreadableRecording => self
                .original_read()
                .map_or(LoadRecovery::ChooseFile, LoadRecovery::Retry),
            LoadFailureKind::RejectEmptyInput | LoadFailureKind::RejectUnrecognizedFormat => self
                .input
                .decoded_text
                .as_ref()
                .map_or(LoadRecovery::ChooseFile, |text| {
                    LoadRecovery::EnterText(text.text())
                }),
            LoadFailureKind::ReportUnavailableAttachment
            | LoadFailureKind::RejectUnsupportedAttachment
            | LoadFailureKind::RejectUnsupportedRecording => LoadRecovery::ChooseFile,
        }
    }

    fn title(&self) -> String {
        format!("Failed to load {}", self.source)
    }

    fn copy_text(&self) -> String {
        let details = format!("{}\n{}", self.title(), self.cause);
        match (self.original_read(), &self.input.decoded_text) {
            (Some(request), _) => format!(
                "{details}\nOriginal read: {request}\nRecovery: {}",
                self.recovery()
            ),
            (None, Some(_)) => format!("{details}\nRecovery: {}", self.recovery()),
            (None, None) => details,
        }
    }

    fn copy_recovery_details(&self, original: &LoadSource, recovered: Option<&str>) -> String {
        let details = self.copy_text();
        let details = match recovered {
            Some(loaded) => format!("Recovered source: {loaded}\n{details}"),
            None => details,
        };
        if original == &self.source {
            details
        } else {
            format!("Original source: {original}\n{details}")
        }
    }
}

#[derive(Debug, Default)]
struct LoadRecoveryInput {
    original_read: Option<LoadReadRequest>,
    decoded_text: Option<LogText>,
}

#[derive(Debug)]
pub(super) enum LoadFailureCause {
    LoadRecording(Box<LoadError>),
    OpenStoredRecording(Box<DbError>),
    ParseLog(LogParseError),
    ReadLogAttachment(Box<LogAttachmentError>),
    ReadLogFile(io::Error),
}

impl LoadFailureCause {
    pub(super) fn kind(&self) -> LoadFailureKind {
        match self {
            Self::LoadRecording(error) => match error.as_ref() {
                LoadError::Io(_) | LoadError::Sdk(SdkError::Io(_)) => {
                    LoadFailureKind::ReportUnreadableRecording
                }
                _ => LoadFailureKind::RejectUnsupportedRecording,
            },
            Self::ParseLog(LogParseError::Empty) => LoadFailureKind::RejectEmptyInput,
            Self::ParseLog(LogParseError::NoRecognisedFormat { .. }) => {
                LoadFailureKind::RejectUnrecognizedFormat
            }
            Self::ReadLogFile(_) => LoadFailureKind::ReportUnreadableFile,
            Self::OpenStoredRecording(error) => match error.as_ref() {
                DbError::SchemaTooNew { .. } => LoadFailureKind::RejectUnsupportedRecording,
                _ => LoadFailureKind::ReportUnreadableRecording,
            },
            Self::ReadLogAttachment(error) => match error.as_ref() {
                LogAttachmentError::MissingLog { .. }
                | LogAttachmentError::UnknownAttachment { .. } => {
                    LoadFailureKind::ReportUnavailableAttachment
                }
                LogAttachmentError::ContentHashMismatch { .. }
                | LogAttachmentError::NotUtf8 { .. }
                | LogAttachmentError::Database(DbError::SchemaTooNew { .. }) => {
                    LoadFailureKind::RejectUnsupportedAttachment
                }
                LogAttachmentError::Database(DbError::UnreadableLogAttachment {
                    source, ..
                }) if !matches!(source.as_ref(), LogAttachmentReadError::ReadAttribute(_)) => {
                    LoadFailureKind::RejectUnsupportedAttachment
                }
                _ => LoadFailureKind::ReportUnreadableAttachment,
            },
        }
    }
}

impl fmt::Display for LoadFailureCause {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::LoadRecording(error) => write!(f, "{error:#}"),
            Self::ParseLog(error) => write!(f, "{error:#}"),
            Self::ReadLogAttachment(error) => write!(f, "{error:#}"),
            Self::ReadLogFile(error) => write!(f, "Failed to read the file: {error:#}"),
            Self::OpenStoredRecording(error) => write!(f, "{error:#}"),
        }
    }
}

/// The load failures the user has not dismissed yet, oldest first.
#[derive(Default)]
pub(super) struct LoadFailures {
    retained: Vec<RetainedLoadFailure>,
    next_failure_id: u64,
    next_retry_id: u64,
}

impl LoadFailures {
    pub(super) fn push(&mut self, failure: LoadFailure) -> LoadFailureId {
        let failure_id = LoadFailureId(self.next_failure_id);
        self.next_failure_id += 1;
        self.retained.push(RetainedLoadFailure {
            failure_id,
            source: failure.source.clone(),
            failure,
            state: LoadRecoveryState::ReportFailure,
        });
        failure_id
    }

    pub(super) fn begin_retry(&mut self, failure: LoadFailureId) -> Option<LoadRetry> {
        let retained = self
            .retained
            .iter_mut()
            .find(|row| row.failure_id == failure)?;
        if !matches!(retained.state, LoadRecoveryState::ReportFailure) {
            return None;
        }
        let LoadRecovery::Retry(request) = retained.failure.recovery() else {
            return None;
        };
        let request = request.clone();
        let attempt = LoadRetryAttempt::new(failure, self.next_retry_id);
        self.next_retry_id += 1;
        let source = match &request {
            LoadReadRequest::ReadLogFile(path) => LoadSource::Filename(path.display().to_string()),
            LoadReadRequest::ReadRecordingFile(request) => {
                LoadSource::Filename(request.path.display().to_string())
            }
            _ => retained.failure.source.clone(),
        };
        retained.state = LoadRecoveryState::AwaitRetry { attempt, source };
        Some(LoadRetry {
            attempt,
            request,
            source: retained.failure.source.clone(),
        })
    }

    pub(super) fn finish_retry(
        &mut self,
        attempt: LoadRetryAttempt,
        result: Result<(), LoadFailure>,
    ) -> bool {
        let Some(index) = self.retained.iter().position(
            |row| matches!(row.state, LoadRecoveryState::AwaitRetry { attempt: pending, .. } if pending == attempt),
        ) else {
            return false;
        };
        match result {
            Ok(()) => {
                if let Some(retained) = self.retained.get_mut(index)
                    && let LoadRecoveryState::AwaitRetry { source, .. } = &retained.state
                {
                    retained.state = LoadRecoveryState::ReportRecovery(source.clone());
                }
            }
            Err(failure) => {
                if let Some(retained) = self.retained.get_mut(index) {
                    retained.failure = failure;
                    retained.state = LoadRecoveryState::ReportFailure;
                }
            }
        }
        true
    }

    pub(super) fn dismiss(&mut self, failure: LoadFailureId) {
        self.retained.retain(|row| row.failure_id != failure);
    }

    pub(super) fn cancel_retry(&mut self, attempt: LoadRetryAttempt) {
        if let Some(retained) = self.retained.iter_mut().find(
            |row| matches!(row.state, LoadRecoveryState::AwaitRetry { attempt: owner, .. } if owner == attempt),
        ) {
            retained.state = LoadRecoveryState::ReportFailure;
        }
    }

    pub(super) fn cancel_input(&mut self, attempt: LoadRetryAttempt) {
        for retained in &mut self.retained {
            if matches!(retained.state, LoadRecoveryState::AwaitFile(owner) | LoadRecoveryState::EditText { attempt: owner, .. } if owner == attempt)
            {
                retained.state = LoadRecoveryState::ReportFailure;
                break;
            }
        }
    }

    pub(super) fn begin_input(&mut self, failure: LoadFailureId) -> Option<LoadRetryAttempt> {
        let retained = self
            .retained
            .iter_mut()
            .find(|row| row.failure_id == failure)?;
        if !matches!(retained.state, LoadRecoveryState::ReportFailure) {
            return None;
        }
        let attempt = LoadRetryAttempt::new(failure, self.next_retry_id);
        let state = match retained.failure.recovery() {
            LoadRecovery::ChooseFile => LoadRecoveryState::AwaitFile(attempt),
            LoadRecovery::EnterText(text) => LoadRecoveryState::EditText {
                attempt,
                text: text.to_owned(),
                focus: true,
            },
            LoadRecovery::Retry(_) => return None,
        };
        self.next_retry_id += 1;
        retained.state = state;
        Some(attempt)
    }

    pub(super) fn accept_file_input(&mut self, attempt: LoadRetryAttempt, path: &Path) -> bool {
        let Some(retained) = self.retained.iter_mut().find(
            |row| matches!(row.state, LoadRecoveryState::AwaitFile(owner) if owner == attempt),
        ) else {
            return false;
        };
        retained.state = LoadRecoveryState::AwaitRetry {
            attempt,
            source: LoadSource::Filename(path.display().to_string()),
        };
        true
    }

    pub(super) fn submit_text_input(&mut self, attempt: LoadRetryAttempt) -> Option<String> {
        for retained in &mut self.retained {
            if let LoadRecoveryState::EditText {
                attempt: owner,
                text,
                ..
            } = &mut retained.state
                && *owner == attempt
            {
                let text = mem::take(text);
                retained.state = LoadRecoveryState::AwaitRetry {
                    attempt,
                    source: LoadSource::PastedText,
                };
                return Some(text);
            }
        }
        None
    }

    /// Shows one error window per failure, and removes the failures the user
    /// dismissed on this frame.
    pub(super) fn show_windows(&mut self, ctx: &egui::Context) -> Vec<LoadProblemRequest> {
        let failure_count = self.retained.len();
        let mut dismissed = Vec::new();
        let mut retries = Vec::new();
        let mut dismiss_all = false;
        let mut inputs = Vec::new();
        for retained in &mut self.retained {
            match retained.show_window(ctx, failure_count) {
                FailureWindowAction::Dismiss => dismissed.push(retained.failure_id),
                FailureWindowAction::DismissAll => dismiss_all = true,
                FailureWindowAction::Keep => {}
                FailureWindowAction::Retry => {
                    retries.push(LoadProblemRequest::Retry(retained.failure_id))
                }
                action @ (FailureWindowAction::ChooseFile | FailureWindowAction::EnterText) => {
                    inputs.push((retained.failure_id, action))
                }
                FailureWindowAction::CancelInput => {
                    retained.state = LoadRecoveryState::ReportFailure;
                }
                FailureWindowAction::SubmitText => {
                    if let Some(attempt) = retained.state.attempt() {
                        retries.push(LoadProblemRequest::LoadText(attempt));
                    }
                }
            }
        }
        if dismiss_all {
            self.retained.clear();
        } else {
            for failure in dismissed {
                self.dismiss(failure);
            }
        }
        for (failure, action) in inputs {
            if let Some(attempt) = self.begin_input(failure)
                && action == FailureWindowAction::ChooseFile
            {
                retries.push(LoadProblemRequest::ChooseFile(attempt));
            }
        }
        retries
    }

    pub(super) fn file_input_is_pending(&self, attempt: LoadRetryAttempt) -> bool {
        self.retained
            .iter()
            .any(|row| matches!(row.state, LoadRecoveryState::AwaitFile(owner) if owner == attempt))
    }

    pub(super) fn retry_is_pending(&self, attempt: LoadRetryAttempt) -> bool {
        self.retained.iter().any(
            |row| matches!(row.state, LoadRecoveryState::AwaitRetry { attempt: pending, .. } if pending == attempt),
        )
    }

    #[cfg(test)]
    pub(super) fn iter(&self) -> impl Iterator<Item = &LoadFailure> {
        self.retained
            .iter()
            .filter(|row| !matches!(row.state, LoadRecoveryState::ReportRecovery(_)))
            .map(|retained| &retained.failure)
    }
}

struct RetainedLoadFailure {
    /// The window id of the failure: egui derives a window id from the title
    /// otherwise, and two failures of one source have the same title.
    failure_id: LoadFailureId,
    source: LoadSource,
    failure: LoadFailure,
    state: LoadRecoveryState,
}

enum LoadRecoveryState {
    AwaitFile(LoadRetryAttempt),
    AwaitRetry {
        attempt: LoadRetryAttempt,
        source: LoadSource,
    },
    EditText {
        attempt: LoadRetryAttempt,
        text: String,
        focus: bool,
    },
    ReportFailure,
    ReportRecovery(LoadSource),
}

impl LoadRecoveryState {
    fn body_ui(&mut self, ui: &mut egui::Ui, failure: &LoadFailure, original: &LoadSource) {
        match self {
            Self::EditText {
                attempt,
                text,
                focus,
            } => {
                let label = ui.label("Log text");
                let response = ui
                    .add(
                        TextEdit::multiline(text)
                            .id(egui::Id::new(TEXT_INPUT_ID_SALT).with(*attempt))
                            .desired_rows(TEXT_INPUT_ROWS)
                            .desired_width(f32::INFINITY),
                    )
                    .labelled_by(label.id);
                if mem::take(focus) {
                    response.request_focus();
                }
            }
            Self::AwaitFile(_) => {
                ui.label("Choose a replacement file");
            }
            Self::ReportRecovery(source) => {
                ui.label("Recovered");
                ui.add(
                    Label::new(format!("Loaded source {source}"))
                        .selectable(true)
                        .wrap(),
                );
                if source != original {
                    ui.add(
                        Label::new(format!("Original source {original}"))
                            .selectable(true)
                            .wrap(),
                    );
                }
            }
            Self::AwaitRetry { source, .. } => {
                ui.add(
                    Label::new(format!("Loading source {source}"))
                        .selectable(true)
                        .wrap(),
                );
                ui.add(
                    Label::new(failure.cause.to_string())
                        .selectable(true)
                        .wrap(),
                );
            }
            Self::ReportFailure => {
                ui.add(
                    Label::new(failure.cause.to_string())
                        .selectable(true)
                        .wrap(),
                );
            }
        }
    }

    fn attempt(&self) -> Option<LoadRetryAttempt> {
        match self {
            Self::AwaitFile(attempt)
            | Self::AwaitRetry { attempt, .. }
            | Self::EditText { attempt, .. } => Some(*attempt),
            Self::ReportFailure | Self::ReportRecovery(_) => None,
        }
    }
}

impl RetainedLoadFailure {
    fn show_window(&mut self, ctx: &egui::Context, failure_count: usize) -> FailureWindowAction {
        let mut action = FailureWindowAction::Keep;
        let recovery = self.recovery_control();
        let window_id = self.window_id();
        let title = match &self.state {
            LoadRecoveryState::ReportRecovery(source) => format!("Recovered {source}"),
            _ => format!("Failed to load {}", self.source),
        };
        let recovered = if let LoadRecoveryState::ReportRecovery(source) = &self.state {
            Some(source.to_string())
        } else {
            None
        };
        let source = &self.source;
        let failure = &self.failure;
        let state = &mut self.state;
        Window::new(title)
            .id(window_id)
            .collapsible(false)
            .resizable(true)
            .min_width(LOAD_FAILURE_WINDOW_MIN_WIDTH)
            .show(ctx, |ui| {
                modals::dialog_body_above_the_action_row(
                    ui,
                    DialogBody::new(|ui| state.body_ui(ui, failure, source)),
                    DialogActionRow::buttons(|ui| {
                        if let Some((primary, enabled)) = recovery {
                            if ui
                                .add_enabled(enabled, Button::new(primary.to_string()))
                                .on_hover_text(match primary {
                                    FailureWindowAction::CancelInput => "Cancel replacement input",
                                    FailureWindowAction::ChooseFile => "Select a replacement file",
                                    FailureWindowAction::EnterText => "Edit the retained log text",
                                    FailureWindowAction::SubmitText => "Load the edited text",
                                    _ => "Read the original source again",
                                })
                                .on_disabled_hover_text(
                                    if primary == FailureWindowAction::SubmitText {
                                        "Enter log text to load"
                                    } else {
                                        "The source is being loaded"
                                    },
                                )
                                .clicked()
                            {
                                action = primary;
                            }
                            if primary == FailureWindowAction::SubmitText
                                && ui.button("Cancel").clicked()
                            {
                                action = FailureWindowAction::CancelInput;
                            }
                            if !enabled && primary == FailureWindowAction::Retry {
                                ui.spinner();
                                ui.label("Retrying…");
                            }
                        }
                        if ui.button(DISMISS_BUTTON_LABEL).clicked() {
                            action = FailureWindowAction::Dismiss;
                        }
                        if ui
                            .add_enabled(failure_count > 1, Button::new(DISMISS_ALL_BUTTON_LABEL))
                            .on_hover_text(format!("Dismiss all {failure_count} load problems"))
                            .on_disabled_hover_text("No other load failed")
                            .clicked()
                        {
                            action = FailureWindowAction::DismissAll;
                        }
                        if ui
                            .button(format!("{ICON_COPY} {COPY_DETAILS_BUTTON_LABEL}"))
                            .on_hover_text("Copy the source and the error text")
                            .clicked()
                        {
                            ui.ctx().copy_text(
                                failure.copy_recovery_details(source, recovered.as_deref()),
                            );
                        }
                    }),
                );
            });
        action
    }

    fn window_id(&self) -> egui::Id {
        egui::Id::new(LOAD_FAILURE_WINDOW_ID_SALT).with(self.failure_id)
    }

    fn recovery_control(&self) -> Option<(FailureWindowAction, bool)> {
        match &self.state {
            LoadRecoveryState::AwaitFile(_) => Some((FailureWindowAction::CancelInput, true)),
            LoadRecoveryState::AwaitRetry { .. } => Some((FailureWindowAction::Retry, false)),
            LoadRecoveryState::EditText { text, .. } => {
                Some((FailureWindowAction::SubmitText, !text.trim().is_empty()))
            }
            LoadRecoveryState::ReportFailure => Some((
                match self.failure.recovery() {
                    LoadRecovery::ChooseFile => FailureWindowAction::ChooseFile,
                    LoadRecovery::EnterText(_) => FailureWindowAction::EnterText,
                    LoadRecovery::Retry(_) => FailureWindowAction::Retry,
                },
                true,
            )),
            LoadRecoveryState::ReportRecovery(_) => None,
        }
    }
}

#[derive(Clone, Copy, Display, Eq, PartialEq)]
enum FailureWindowAction {
    #[strum(serialize = "Cancel")]
    CancelInput,
    #[strum(serialize = "Choose file…")]
    ChooseFile,
    Dismiss,
    DismissAll,
    #[strum(serialize = "Enter text…")]
    EnterText,
    Keep,
    Retry,
    #[strum(serialize = "Load text")]
    SubmitText,
}

const TEXT_INPUT_ID_SALT: &str = "load_recovery_text";
const TEXT_INPUT_ROWS: usize = 4;

const DISMISS_BUTTON_LABEL: &str = "Dismiss";

const DISMISS_ALL_BUTTON_LABEL: &str = "Dismiss all";

const COPY_DETAILS_BUTTON_LABEL: &str = "Copy details";

const LOAD_FAILURE_WINDOW_ID_SALT: &str = "load_failure_window";

const LOAD_FAILURE_WINDOW_MIN_WIDTH: f32 = 420.0;

#[cfg(test)]
mod tests {
    use std::iter;

    use egui::accesskit::Role;
    use egui_kittest::kittest::{NodeT as _, Queryable as _};
    use gt_test_utils::window_fit::{CRAMPED_VIEWPORT, NARROW_VIEWPORT, SHORT_VIEWPORT};
    use gt_test_utils::{
        AuditedWindow, ControlLabel, HarnessInteraction as _, TestHarness, WindowFitAssertions as _,
    };
    use rstest::rstest;

    use super::*;

    const PARSE_ERROR: &str =
        "Not a recognised log: no line has a timestamp in a known format (first line: \"boot\")";

    fn unrecognised_log(first_line: &str) -> LoadFailureCause {
        LoadFailureCause::ParseLog(LogParseError::NoRecognisedFormat {
            first_line: first_line.to_owned(),
        })
    }

    fn failure_of(source: LoadSource) -> LoadFailure {
        LoadFailure::new(source, unrecognised_log("boot"))
    }

    fn harness_showing(failures: Vec<LoadFailure>) -> TestHarness<'static, LoadFailures> {
        harness_showing_at(failures, egui::vec2(900.0, 600.0))
    }

    fn harness_showing_at(
        failures: Vec<LoadFailure>,
        viewport: egui::Vec2,
    ) -> TestHarness<'static, LoadFailures> {
        let mut retained = LoadFailures::default();
        for failure in failures {
            retained.push(failure);
        }
        let mut harness = TestHarness::builder().size(viewport).ui_state(
            |ui: &mut egui::Ui, failures: &mut LoadFailures| {
                let _retries = failures.show_windows(ui.ctx());
            },
            retained,
        );
        harness.run();
        harness
    }

    fn click_copy_details(harness: &mut TestHarness<'static, LoadFailures>) {
        harness
            .inner
            .get_by_label_contains(COPY_DETAILS_BUTTON_LABEL)
            .click();
        harness.step();
    }

    #[rstest]
    #[case::file(LoadSource::Filename("navsyncd.log".to_owned()), "Failed to load navsyncd.log")]
    #[case::pasted_text(LoadSource::PastedText, "Failed to load pasted log text")]
    #[case::unnamed_drop(LoadSource::UnnamedDrop, "Failed to load unnamed dropped log")]
    fn the_window_title_states_the_source_of_the_failure(
        #[case] source: LoadSource,
        #[case] title: &str,
    ) {
        let harness = harness_showing(vec![failure_of(source)]);

        let window = harness.inner.get_by_role_and_label(Role::Window, title);
        assert!(window.query_by_label(PARSE_ERROR).is_some());
    }

    #[test]
    fn copying_a_recording_failure_writes_the_title_above_the_error_text() {
        let mut harness = harness_showing(vec![LoadFailure::new(
            LoadSource::Filename("drive.gtd".to_owned()),
            LoadFailureCause::LoadRecording(Box::new(LoadError::Io(
                io::ErrorKind::UnexpectedEof.into(),
            ))),
        )]);
        let error_text = "I/O error: unexpected end of file";
        assert!(harness.inner.query_by_label(error_text).is_some());

        click_copy_details(&mut harness);

        assert_eq!(
            harness.inner.copied_text().as_deref(),
            Some(format!("Failed to load drive.gtd\n{error_text}").as_str())
        );
    }

    #[test]
    fn dragging_across_the_details_selects_them_for_copying() {
        let mut harness = harness_showing(vec![failure_of(LoadSource::Filename(
            "navsyncd.log".to_owned(),
        ))]);
        let details = harness.inner.get_by_label(PARSE_ERROR).rect();

        harness.inner.press_drag_release(
            details.left_top() + egui::vec2(1.0, 2.0),
            details.size() - egui::vec2(2.0, 2.0),
            4,
        );
        harness.inner.input_mut().events.push(egui::Event::Copy);
        harness.step();

        assert_eq!(harness.inner.copied_text().as_deref(), Some(PARSE_ERROR));
    }

    #[test]
    fn dismissing_a_failure_removes_only_that_failure_of_the_same_source() {
        let first = LoadFailure::new(LoadSource::PastedText, unrecognised_log("first entry"));
        let second = LoadFailure::new(LoadSource::PastedText, unrecognised_log("second entry"));
        let second_copy_text = second.copy_text();
        let mut harness = harness_showing(vec![first, second]);

        harness
            .inner
            .query_all_by_role_and_label(Role::Window, "Failed to load pasted log text")
            .find(|window| window.query_by_label_contains("first entry").is_some())
            .expect("the window of the first failure")
            .get_by_label(DISMISS_BUTTON_LABEL)
            .click();
        harness.run();

        assert_eq!(
            harness
                .state()
                .iter()
                .map(LoadFailure::copy_text)
                .collect::<Vec<_>>(),
            [second_copy_text]
        );
        assert_eq!(
            harness
                .inner
                .query_all_by_role_and_label(Role::Window, "Failed to load pasted log text")
                .count(),
            1
        );
    }

    #[rstest]
    #[case::one_failure(1, false)]
    #[case::two_failures(2, true)]
    fn dismiss_all_is_enabled_with_more_than_one_failure(
        #[case] failure_count: usize,
        #[case] enabled: bool,
    ) {
        let harness = harness_showing(
            iter::repeat_with(|| failure_of(LoadSource::PastedText))
                .take(failure_count)
                .collect(),
        );

        let buttons: Vec<bool> = harness
            .inner
            .query_all_by_label(DISMISS_ALL_BUTTON_LABEL)
            .map(|button| !button.accesskit_node().is_disabled())
            .collect();
        assert_eq!(buttons, vec![enabled; failure_count]);
    }

    #[test]
    fn dismissing_all_from_one_window_removes_every_failure() {
        let mut harness = harness_showing(vec![
            failure_of(LoadSource::PastedText),
            failure_of(LoadSource::Filename("navsyncd.log".to_owned())),
        ]);

        harness
            .inner
            .get_by_role_and_label(Role::Window, "Failed to load navsyncd.log")
            .get_by_label(DISMISS_ALL_BUTTON_LABEL)
            .click();
        harness.run();

        assert_eq!(harness.state().iter().count(), 0);
        assert!(
            harness
                .inner
                .query_by_label_contains("Failed to load")
                .is_none()
        );
    }

    #[rstest]
    fn the_window_keeps_its_actions_reachable_with_oversized_text(
        #[values(CRAMPED_VIEWPORT, NARROW_VIEWPORT, SHORT_VIEWPORT)] viewport: egui::Vec2,
        #[values(false, true)] retrying: bool,
    ) {
        let failure = if retrying {
            LoadFailure::new(
                LoadSource::Filename(gt_test_utils::oversized_text('f')),
                LoadFailureCause::ReadLogFile(io::Error::other(gt_test_utils::oversized_text('d'))),
            )
            .with_original_read(Some(LoadReadRequest::ReadLogFile("original.log".into())))
        } else {
            LoadFailure::new(
                LoadSource::Filename(gt_test_utils::oversized_text('f')),
                unrecognised_log(&gt_test_utils::oversized_text('d')),
            )
        };
        let title = failure.title();
        let mut harness = harness_showing_at(vec![failure], viewport);
        if retrying {
            let id = harness.state().retained.first().unwrap().failure_id;
            assert!(harness.state_mut().begin_retry(id).is_some());
        }
        harness.inner.run_steps(8);
        let window_id = harness
            .state()
            .retained
            .first()
            .map(RetainedLoadFailure::window_id)
            .expect("the failure is retained");

        let window = AuditedWindow::identified(&title, window_id);
        harness.inner.assert_window_fits_the_viewport(window);
        if retrying {
            harness
                .inner
                .assert_control_is_reachable(window, ControlLabel("Retry"));
            assert!(
                harness
                    .inner
                    .get_by_label("Retry")
                    .accesskit_node()
                    .is_disabled()
            );
        }
        let copy_label = format!("{ICON_COPY} {COPY_DETAILS_BUTTON_LABEL}");
        for control in [
            DISMISS_BUTTON_LABEL,
            DISMISS_ALL_BUTTON_LABEL,
            copy_label.as_str(),
        ] {
            harness
                .inner
                .assert_control_is_reachable(window, ControlLabel(control));
        }
    }

    #[test]
    fn snapshot_load_failure_window() {
        let mut harness = harness_showing_at(
            vec![failure_of(LoadSource::Filename("navsyncd.log".to_owned()))],
            egui::vec2(480.0, 160.0),
        );
        harness.snapshot_with_color_tolerance("load_failure_window");
    }
}

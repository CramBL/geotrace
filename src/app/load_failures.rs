use std::path::Path;
use std::{fmt, io, mem};

use egui::{
    Button, Label, RichText, ScrollArea, TextEdit, TextStyle, WidgetInfo, WidgetType, Window,
};
use egui_phosphor::regular::{
    CHECK as ICON_CHECK, CLOCK_COUNTER_CLOCKWISE as ICON_CLOCK, COPY as ICON_COPY,
    WARNING as ICON_WARNING, X as ICON_DISMISS,
};
use gt_loader::{LoadError, SdkError};
use gt_log_view::LogAttachmentRef;
use gt_logfile::{LogParseError, LogText};
use gt_store::{DatabaseRef, DbError, LogAttachmentError, LogAttachmentReadError};
use strum::Display;

use crate::app::keyboard_scope::{KeyboardAccess, WindowKeyboardScope};
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

/// The load problems the user has not dismissed yet, oldest first.
#[derive(Default)]
pub(super) struct LoadFailures {
    retained: Vec<RetainedLoadFailure>,
    next_failure_id: u64,
    next_retry_id: u64,
    selected: Option<LoadFailureId>,
    details_open: bool,
    scroll_to_selected: bool,
    keyboard_scope: WindowKeyboardScope,
}

impl LoadFailures {
    fn footer_ui(ui: &mut egui::Ui, count: usize, details_open: bool) -> FailureWindowAction {
        let mut action = FailureWindowAction::Keep;
        let dismiss = ui
            .button(ICON_DISMISS)
            .on_hover_text("Dismiss the selected input");
        dismiss.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, DISMISS_BUTTON_LABEL));
        if dismiss.clicked() {
            action = FailureWindowAction::Dismiss;
        }
        if ui
            .add_enabled(
                count > 1,
                Button::new(format!("{DISMISS_ALL_BUTTON_LABEL} ({count})")),
            )
            .on_hover_text(format!("Dismiss all {count} load problems"))
            .on_disabled_hover_text("No other load problem")
            .clicked()
        {
            action = FailureWindowAction::DismissAll;
        }
        let copy = ui
            .button(ICON_COPY)
            .on_hover_text("Copy the selected source and full diagnostic");
        copy.widget_info(|| {
            WidgetInfo::labeled(WidgetType::Button, true, COPY_DETAILS_BUTTON_LABEL)
        });
        if copy.clicked() {
            action = FailureWindowAction::CopyDetails;
        }
        if ui
            .selectable_label(details_open, "Details")
            .on_hover_text("Show the selected diagnostic and format help")
            .clicked()
        {
            action = FailureWindowAction::ToggleDetails;
        }
        action
    }

    pub(super) fn push(&mut self, failure: LoadFailure) -> LoadFailureId {
        let failure_id = LoadFailureId(self.next_failure_id);
        self.next_failure_id += 1;
        if self.selected.is_none() {
            self.selected = Some(failure_id);
            self.keyboard_scope = WindowKeyboardScope::default();
        }
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
        let Some(index) = self
            .retained
            .iter()
            .position(|row| row.failure_id == failure)
        else {
            return;
        };
        self.retained.remove(index);
        if self.selected == Some(failure) {
            self.selected = self
                .retained
                .get(index)
                .or_else(|| self.retained.last())
                .map(|row| row.failure_id);
            self.scroll_to_selected = true;
        }
        if self.retained.is_empty() {
            self.details_open = false;
            self.keyboard_scope = WindowKeyboardScope::default();
        }
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

    pub(super) fn handle_keyboard(&mut self, ctx: &egui::Context) {
        let layer = egui::LayerId::new(
            egui::Order::Middle,
            egui::Id::new(LOAD_FAILURE_WINDOW_ID_SALT),
        );
        let focused = ctx.memory(|memory| memory.focused());
        if self.retained.is_empty()
            || !self.keyboard_scope.owns_keyboard(ctx, layer)
            || !KeyboardAccess::Permit.available(ctx)
        {
            return;
        }
        if ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
            if let Some(selected) = self.selected {
                let input = self
                    .retained
                    .iter()
                    .find(|row| row.failure_id == selected)
                    .and_then(|row| match row.state {
                        LoadRecoveryState::AwaitFile(attempt)
                        | LoadRecoveryState::EditText { attempt, .. } => Some(attempt),
                        _ => None,
                    });
                if let Some(attempt) = input {
                    self.cancel_input(attempt);
                } else {
                    self.dismiss(selected);
                }
            }
            if let Some(focused) = focused {
                ctx.memory_mut(|memory| memory.surrender_focus(focused));
            }
        }
    }

    pub(super) fn show_window(&mut self, ctx: &egui::Context) -> Vec<LoadProblemRequest> {
        if self.retained.is_empty() {
            return Vec::new();
        }
        let count = self.retained.len();
        let details_open = self.details_open;
        let mut action = None;
        let viewport = ctx.content_rect();
        let window = Window::new(LOAD_PROBLEMS_TITLE)
            .id(egui::Id::new(LOAD_FAILURE_WINDOW_ID_SALT))
            .collapsible(false)
            .resizable(true)
            .default_width(LOAD_PROBLEMS_DEFAULT_WIDTH)
            .min_width(LOAD_PROBLEMS_MIN_WIDTH.min(viewport.width()))
            .max_width(viewport.width())
            .max_height(viewport.height())
            .constrain_to(viewport)
            .show(ctx, |ui| {
                ui.label(format!(
                    "{count} {}",
                    gt_fmt::pluralize(count, "input", "inputs")
                ));
                let footer = modals::dialog_body_above_the_action_row(
                    ui,
                    DialogBody::new(|ui| action = self.content_ui(ui)),
                    DialogActionRow::buttons(|ui| Self::footer_ui(ui, count, details_open)),
                );
                if footer != FailureWindowAction::Keep {
                    action = self.selected.map(|id| (id, footer));
                }
            });
        if let Some(window) = window {
            self.keyboard_scope
                .remember_focus(ctx, window.response.layer_id);
        }
        self.apply_window_action(action, ctx).into_iter().collect()
    }

    pub(super) fn is_empty(&self) -> bool {
        self.retained.is_empty()
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

    fn content_ui(&mut self, ui: &mut egui::Ui) -> Option<(LoadFailureId, FailureWindowAction)> {
        let mut action = None;
        ui.set_max_width(ui.available_width().min(ui.clip_rect().width()));
        let multiple = self.retained.len() > 1;
        let row_height = ui.spacing().interact_size.y
            + ui.text_style_height(&TextStyle::Small)
            + ui.spacing().item_spacing.y;
        if multiple {
            let mut list = ScrollArea::vertical()
                .id_salt(PROBLEM_LIST_ID_SALT)
                .max_height(PROBLEM_LIST_MAX_HEIGHT)
                .min_scrolled_height(0.0)
                .auto_shrink([false, true]);
            if mem::take(&mut self.scroll_to_selected)
                && let Some(index) = self
                    .retained
                    .iter()
                    .position(|row| Some(row.failure_id) == self.selected)
            {
                list = list.vertical_scroll_offset(
                    index as f32 * (row_height + ui.spacing().item_spacing.y),
                );
            }
            list.show_rows(ui, row_height, self.retained.len(), |ui, range| {
                for row in self.retained.get(range).unwrap_or_default() {
                    let row_action = row.row_ui(ui, Some(Some(row.failure_id) == self.selected));
                    if row_action != FailureWindowAction::Keep {
                        action = Some((row.failure_id, row_action));
                    }
                }
            });
        } else if let Some(row) = self.retained.first() {
            let row_action = row.row_ui(ui, None);
            if row_action != FailureWindowAction::Keep {
                action = Some((row.failure_id, row_action));
            }
        }
        if let Some(selected) = self.selected
            && let Some(row) = self
                .retained
                .iter_mut()
                .find(|row| row.failure_id == selected)
        {
            let editing = matches!(row.state, LoadRecoveryState::EditText { .. });
            if editing || self.details_open {
                ui.separator();
                ScrollArea::both()
                    .id_salt((PROBLEM_DETAILS_ID_SALT, selected))
                    .max_height(PROBLEM_DETAILS_MAX_HEIGHT)
                    .min_scrolled_height(0.0)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        if editing {
                            row.state.text_input_ui(ui);
                        }
                        if self.details_open {
                            row.details_ui(ui);
                        }
                    });
            }
        }
        action
    }

    fn apply_window_action(
        &mut self,
        action: Option<(LoadFailureId, FailureWindowAction)>,
        ctx: &egui::Context,
    ) -> Option<LoadProblemRequest> {
        let (id, action) = action?;
        ctx.request_repaint();
        if action == FailureWindowAction::DismissAll {
            self.retained.clear();
            self.selected = None;
            self.details_open = false;
            self.keyboard_scope = WindowKeyboardScope::default();
            return None;
        }
        if action == FailureWindowAction::Dismiss {
            self.dismiss(id);
            return None;
        }
        if action == FailureWindowAction::ToggleDetails {
            self.details_open = !self.details_open;
            return None;
        }
        if action == FailureWindowAction::CopyDetails {
            if let Some(row) = self.retained.iter().find(|row| row.failure_id == id) {
                ctx.copy_text(row.copy_text());
            }
            return None;
        }
        self.selected = Some(id);
        self.scroll_to_selected = true;
        match action {
            FailureWindowAction::Select => {
                self.details_open = true;
            }
            FailureWindowAction::Retry => return Some(LoadProblemRequest::Retry(id)),
            FailureWindowAction::ChooseFile | FailureWindowAction::EnterText => {
                let attempt = self.begin_input(id)?;
                if action == FailureWindowAction::ChooseFile {
                    return Some(LoadProblemRequest::ChooseFile(attempt));
                }
            }
            FailureWindowAction::CancelInput | FailureWindowAction::SubmitText => {
                let attempt = self
                    .retained
                    .iter()
                    .find(|row| row.failure_id == id)?
                    .state
                    .attempt()?;
                if action == FailureWindowAction::CancelInput {
                    self.cancel_input(attempt);
                } else {
                    return Some(LoadProblemRequest::LoadText(attempt));
                }
            }
            _ => {}
        }
        None
    }
}

struct RetainedLoadFailure {
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
    fn text_input_ui(&mut self, ui: &mut egui::Ui) {
        if let Self::EditText {
            attempt,
            text,
            focus,
        } = self
        {
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
    fn row_ui(&self, ui: &mut egui::Ui, selected: Option<bool>) -> FailureWindowAction {
        let mut action = FailureWindowAction::Keep;
        let recovery = self.recovery_control();
        let action_width = recovery.map_or(0.0, |(primary, _)| {
            primary.button_width(ui)
                + if primary == FailureWindowAction::SubmitText {
                    FailureWindowAction::CancelInput.button_width(ui) + ui.spacing().item_spacing.x
                } else {
                    0.0
                }
        });
        let identity_width =
            (ui.available_width() - action_width - ui.spacing().item_spacing.x).max(0.0);
        ui.push_id(self.failure_id, |ui| {
            ui.horizontal_top(|ui| {
                ui.vertical(|ui| {
                    ui.set_width(identity_width);
                    let identity = self.source.to_string();
                    let origin = self.failure.original_read().map_or_else(
                        || identity.clone(),
                        |request| format!("{identity}\n{request}"),
                    );
                    if let Some(selected) = selected {
                        if ui
                            .add_sized(
                                [identity_width, ui.spacing().interact_size.y],
                                Button::new(&identity).selected(selected).truncate(),
                            )
                            .on_hover_text(&origin)
                            .clicked()
                        {
                            action = FailureWindowAction::Select;
                        }
                    } else {
                        ui.add(Label::new(&identity).selectable(true).truncate())
                            .on_hover_text(&origin);
                    }
                    self.status_ui(ui);
                });
                if let Some((primary, enabled)) = recovery {
                    let response = ui
                        .add_enabled(enabled, Button::new(primary.to_string()))
                        .on_hover_text(primary.hover_text())
                        .on_disabled_hover_text(if primary == FailureWindowAction::SubmitText {
                            "Enter log text to load"
                        } else {
                            "The source is being loaded"
                        });
                    if response.clicked() {
                        action = primary;
                    }
                    if primary == FailureWindowAction::SubmitText
                        && ui.small_button("Cancel").clicked()
                    {
                        action = FailureWindowAction::CancelInput;
                    }
                }
            })
        });
        action
    }

    fn status_ui(&self, ui: &mut egui::Ui) {
        let (icon, color, summary) = match &self.state {
            LoadRecoveryState::ReportFailure => (
                ICON_WARNING,
                gt_ui_theme::warning_amber(ui.visuals().dark_mode),
                self.failure.cause.kind().to_string(),
            ),
            LoadRecoveryState::ReportRecovery(source) => (
                ICON_CHECK,
                gt_ui_theme::SUCCESS_GREEN,
                format!("Recovered {source}"),
            ),
            LoadRecoveryState::AwaitFile(_) => (
                ICON_CLOCK,
                ui.visuals().text_color(),
                "Choosing file".into(),
            ),
            LoadRecoveryState::EditText { .. } => {
                (ICON_CLOCK, ui.visuals().text_color(), "Editing text".into())
            }
            LoadRecoveryState::AwaitRetry { source, .. } => (
                ICON_CLOCK,
                ui.visuals().text_color(),
                format!("Loading {source}"),
            ),
        };
        ui.add(
            Label::new(
                RichText::new(format!("{icon} {summary}"))
                    .small()
                    .color(color),
            )
            .truncate(),
        )
        .on_hover_text(summary);
    }

    fn details_ui(&self, ui: &mut egui::Ui) {
        ui.add(Label::new(self.copy_text()).selectable(true).wrap());
        match self.failure.cause.kind() {
            LoadFailureKind::RejectEmptyInput | LoadFailureKind::RejectUnrecognizedFormat => {
                ui.separator();
                ui.label(SUPPORTED_LOG_FORMAT_HELP);
            }
            LoadFailureKind::RejectUnsupportedRecording => {
                ui.separator();
                ui.label("Choose a GeoTrace .gtd recording or a timestamped log file");
            }
            _ => {}
        }
    }

    fn copy_text(&self) -> String {
        let recovered = if let LoadRecoveryState::ReportRecovery(source) = &self.state {
            Some(source.to_string())
        } else {
            None
        };
        self.failure
            .copy_recovery_details(&self.source, recovered.as_deref())
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
    CopyDetails,
    Dismiss,
    DismissAll,
    #[strum(serialize = "Enter text…")]
    EnterText,
    Keep,
    Retry,
    Select,
    #[strum(serialize = "Load text")]
    SubmitText,
    ToggleDetails,
}

impl FailureWindowAction {
    fn button_width(&self, ui: &egui::Ui) -> f32 {
        ui.fonts_mut(|fonts| {
            fonts
                .layout_no_wrap(
                    self.to_string(),
                    TextStyle::Button.resolve(ui.style()),
                    ui.visuals().text_color(),
                )
                .size()
                .x
        }) + ui.spacing().button_padding.x * 2.0
    }

    fn hover_text(self) -> &'static str {
        match self {
            Self::CancelInput => "Cancel replacement input",
            Self::ChooseFile => "Select a replacement file",
            Self::EnterText => "Edit the retained log text",
            Self::SubmitText => "Load the edited text",
            _ => "Read the original source again",
        }
    }
}

const LOAD_PROBLEMS_TITLE: &str = "Load problems";
const LOAD_PROBLEMS_DEFAULT_WIDTH: f32 = 520.0;
const LOAD_PROBLEMS_MIN_WIDTH: f32 = 240.0;
const PROBLEM_LIST_ID_SALT: &str = "load_problem_list";
const PROBLEM_DETAILS_ID_SALT: &str = "load_problem_details";
const PROBLEM_LIST_MAX_HEIGHT: f32 = 180.0;
const PROBLEM_DETAILS_MAX_HEIGHT: f32 = 220.0;
const SUPPORTED_LOG_FORMAT_HELP: &str = "Log entries start with an ISO 8601 date and time or a syslog month, day and time. Fractional seconds are supported.";

const TEXT_INPUT_ID_SALT: &str = "load_recovery_text";
const TEXT_INPUT_ROWS: usize = 4;

const DISMISS_BUTTON_LABEL: &str = "Dismiss";

const DISMISS_ALL_BUTTON_LABEL: &str = "Dismiss all";

const COPY_DETAILS_BUTTON_LABEL: &str = "Copy details";

const LOAD_FAILURE_WINDOW_ID_SALT: &str = "load_failure_window";

#[cfg(test)]
mod tests {
    use std::iter;

    use egui::accesskit::Role;
    use egui_kittest::kittest::{NodeT as _, Queryable as _};
    use gt_test_utils::window_fit::{CRAMPED_VIEWPORT, NARROW_VIEWPORT, SHORT_VIEWPORT};
    use gt_test_utils::{
        AuditedWindow, By, ControlLabel, HarnessInteraction as _, TestHarness,
        WindowFitAssertions as _,
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
                failures.handle_keyboard(ui.ctx());
                for request in failures.show_window(ui.ctx()) {
                    if let LoadProblemRequest::Retry(id) = request {
                        let _retry = failures.begin_retry(id);
                    }
                }
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
    #[case::file(LoadSource::Filename("navsyncd.log".to_owned()), "navsyncd.log")]
    #[case::pasted_text(LoadSource::PastedText, "pasted log text")]
    #[case::unnamed_drop(LoadSource::UnnamedDrop, "unnamed dropped log")]
    fn the_single_problem_shows_its_source_and_discloses_the_diagnostic(
        #[case] source: LoadSource,
        #[case] source_label: &str,
    ) {
        let mut harness = harness_showing(vec![failure_of(source)]);
        harness
            .inner
            .get_by_role_and_label(Role::Window, LOAD_PROBLEMS_TITLE);
        assert!(
            harness
                .inner
                .query(By::new().role(Role::Label).value(source_label))
                .is_some()
        );
        assert!(
            harness
                .inner
                .query_by_label_contains("Not a recognised log")
                .is_none()
        );
        harness.inner.get_by_label("Details").click();
        harness.run();
        assert!(
            harness
                .inner
                .query(By::new().role(Role::Label).predicate(|node| {
                    node.value()
                        .is_some_and(|value| value.contains(PARSE_ERROR))
                }))
                .is_some()
        );
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
        assert!(harness.inner.query_by_label(error_text).is_none());

        click_copy_details(&mut harness);

        assert_eq!(
            harness.inner.copied_text().as_deref(),
            Some(format!("Failed to load drive.gtd\n{error_text}").as_str())
        );
    }

    #[test]
    fn dragging_across_the_details_selects_them_for_copying() {
        let failure = failure_of(LoadSource::Filename("navsyncd.log".to_owned()));
        let expected = failure.copy_text();
        let mut harness = harness_showing(vec![failure]);
        harness.inner.get_by_label("Details").click();
        harness.run();
        let details = harness
            .inner
            .get(By::new().role(Role::Label).value(&expected))
            .rect();

        harness.inner.press_drag_release(
            details.left_top() + egui::vec2(1.0, 2.0),
            details.size() - egui::vec2(2.0, 2.0),
            4,
        );
        harness.inner.input_mut().events.push(egui::Event::Copy);
        harness.step();

        assert_eq!(
            harness.inner.copied_text().as_deref(),
            Some(expected.as_str())
        );
    }

    #[test]
    fn dismissing_a_failure_removes_only_that_failure_of_the_same_source() {
        let first = LoadFailure::new(LoadSource::PastedText, unrecognised_log("first entry"));
        let second = LoadFailure::new(LoadSource::PastedText, unrecognised_log("second entry"));
        let second_copy_text = second.copy_text();
        let mut harness = harness_showing(vec![first, second]);

        harness.inner.get_by_label(DISMISS_BUTTON_LABEL).click();
        harness.run();

        assert_eq!(
            harness.state().selected,
            harness.state().retained.first().map(|row| row.failure_id)
        );
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
                .query_all_by_role_and_label(Role::Window, LOAD_PROBLEMS_TITLE)
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
            .query_all_by_label(format!("{DISMISS_ALL_BUTTON_LABEL} ({failure_count})").as_str())
            .map(|button| !button.accesskit_node().is_disabled())
            .collect();
        assert_eq!(buttons, vec![enabled]);
    }

    #[test]
    fn dismissing_all_from_one_window_removes_every_failure() {
        let mut harness = harness_showing(vec![
            failure_of(LoadSource::PastedText),
            failure_of(LoadSource::Filename("navsyncd.log".to_owned())),
        ]);

        harness
            .inner
            .get_by_role_and_label(Role::Window, LOAD_PROBLEMS_TITLE)
            .get_by_label("Dismiss all (2)")
            .click();
        harness.run();

        assert_eq!(harness.state().iter().count(), 0);
        assert!(harness.inner.query_by_label(LOAD_PROBLEMS_TITLE).is_none());
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
        let title = LOAD_PROBLEMS_TITLE;
        let mut harness = harness_showing_at(vec![failure], viewport);
        if retrying {
            let id = harness.state().retained.first().unwrap().failure_id;
            assert!(harness.state_mut().begin_retry(id).is_some());
        }
        harness.inner.get_by_label("Details").click();
        harness.inner.run_steps(8);
        let window = AuditedWindow::identified(title, egui::Id::new(LOAD_FAILURE_WINDOW_ID_SALT));
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
        let copy_label = COPY_DETAILS_BUTTON_LABEL;
        for control in [DISMISS_BUTTON_LABEL, "Dismiss all (1)", copy_label] {
            harness
                .inner
                .assert_control_is_reachable(window, ControlLabel(control));
        }
    }

    #[test]
    fn selection_survives_new_problems_and_recovery_before_scoped_dismissal() {
        let failures = ["first.log", "second.log", "third.log"]
            .into_iter()
            .map(|name| {
                LoadFailure::new(
                    LoadSource::Filename(name.into()),
                    LoadFailureCause::ReadLogFile(io::ErrorKind::PermissionDenied.into()),
                )
                .with_original_read(Some(LoadReadRequest::ReadLogFile("actual.log".into())))
            })
            .collect();
        let mut harness = harness_showing(failures);
        harness.inner.get_by_label("second.log").click();
        harness.run();
        let selected = harness.state().selected;
        harness
            .inner
            .query_all_by_label("Retry")
            .nth(1)
            .unwrap()
            .click();
        harness.run();
        let attempt = harness
            .state()
            .retained
            .get(1)
            .expect("the second problem is retained")
            .state
            .attempt()
            .unwrap();
        harness.state_mut().push(failure_of(LoadSource::PastedText));
        assert_eq!(harness.state().selected, selected);
        assert!(harness.state_mut().finish_retry(attempt, Ok(())));
        harness.run();
        assert_eq!(harness.state().selected, selected);
        harness
            .inner
            .get_by_label(format!("{ICON_CHECK} Recovered actual.log").as_str());
        click_copy_details(&mut harness);
        let copied = harness.inner.copied_text().unwrap();
        assert!(copied.contains("Failed to load second.log"));
        assert!(copied.contains("Recovered source: actual.log"));
        harness.inner.get_by_label(DISMISS_BUTTON_LABEL).click();
        harness.run();
        assert_eq!(
            harness.state().selected,
            harness.state().retained.get(1).map(|row| row.failure_id)
        );
        harness.inner.get_by_label("Dismiss all (3)").click();
        harness.run();
        assert_eq!(harness.state().selected, None);
        assert!(harness.inner.query_by_label(LOAD_PROBLEMS_TITLE).is_none());
    }

    #[test]
    fn escape_cancels_owned_text_input_before_dismissing_the_selected_problem() {
        let failure =
            failure_of(LoadSource::PastedText).with_decoded_text("seed".to_owned().into());
        let mut harness = harness_showing(vec![failure]);
        harness.inner.get_by_label("Enter text…").click();
        harness.run();
        harness.inner.get_by_role(Role::MultilineTextInput);
        harness.inner.key_press(egui::Key::Escape);
        harness.run();
        assert_eq!(harness.state().retained.len(), 1);
        harness.inner.get_by_label("Enter text…");
        harness.inner.key_press(egui::Key::Escape);
        harness.run();
        assert!(harness.state().retained.is_empty());
    }

    #[rstest]
    fn snapshot_load_problem_batch(
        #[values(CRAMPED_VIEWPORT, NARROW_VIEWPORT, SHORT_VIEWPORT)] viewport: egui::Vec2,
    ) {
        let pending = LoadFailure::new(
            LoadSource::Filename("sensor.log".into()),
            LoadFailureCause::ReadLogFile(io::ErrorKind::PermissionDenied.into()),
        )
        .with_original_read(Some(LoadReadRequest::ReadLogFile(
            "/logs/sensor.log".into(),
        )));
        let mut harness = harness_showing_at(
            vec![
                failure_of(LoadSource::Filename("unrecognized.log".into())),
                pending,
                failure_of(LoadSource::PastedText),
            ],
            viewport,
        );
        let id = harness
            .state()
            .retained
            .get(1)
            .expect("the second problem is retained")
            .failure_id;
        assert!(harness.state_mut().begin_retry(id).is_some());
        harness.inner.get_by_label("unrecognized.log").click();
        harness.run();
        let window = AuditedWindow::identified(
            LOAD_PROBLEMS_TITLE,
            egui::Id::new(LOAD_FAILURE_WINDOW_ID_SALT),
        );
        harness.inner.assert_window_fits_the_viewport(window);
        for control in [
            DISMISS_BUTTON_LABEL,
            COPY_DETAILS_BUTTON_LABEL,
            "Dismiss all (3)",
        ] {
            harness
                .inner
                .assert_control_is_reachable(window, ControlLabel(control));
        }
        harness.snapshot_with_color_tolerance(&format!(
            "load_problem_batch_{}x{}",
            viewport.x, viewport.y
        ));
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

use std::cmp::Ordering;
use std::time::Instant;

use chrono::NaiveDate;
use egui::{Button, DragValue, Label, RichText, ScrollArea, TextEdit, Window};
use egui_phosphor::regular::CARET_DOWN as ICON_CARET_DOWN;
use egui_phosphor::regular::CARET_UP as ICON_CARET_UP;
use egui_phosphor::regular::X as ICON_X;
use gt_pending_writes::WriteAccess;
use gt_store::{DatabaseRef, NavPointTimeRange, PruneMode, RecordingEntry, RecordingMeta};
use gt_types::TravelMode;
use gt_ui_theme::labels::LabelWithHover;
use gt_ui_theme::warning_amber;
use strum::{EnumCount, EnumIter};

use crate::app::history::delete_hidden_prompt::DeleteHiddenTracksPrompt;
use crate::app::history_db::{DeleteReason, HistoryWorker};
use crate::app::modals::{self, DialogActionRow, DialogBody};
use crate::app::read_only_session::READ_ONLY_RECORDING_HISTORY_HOVER;
use crate::app::storage_controls;
use crate::settings::StorageSettings;

/// Shown in place of the list while the startup open runs: the database is
/// not unavailable, it is not open yet.
pub(in crate::app) const OPENING_RECORDINGS_DATABASE: &str = "Opening the recordings database";

mod delete_hidden_prompt;
mod table;

/// Which pruning mode is selected in the Prune dialog.
#[derive(Clone, Copy, PartialEq, Eq)]
enum PruneKind {
    Age,
    TotalSize,
    Count,
}

const PRUNE_WINDOW_TITLE: &str = "Prune History…";

/// What a permanent delete of stored recordings costs, and what it leaves
/// alone. Shared by the prune dialog, the auto-prune prompt and the
/// delete-hidden confirmation.
pub(super) const DESTRUCTIVE_DELETE_HOVER: &str =
    "This cannot be undone. The original source files are unaffected.";

/// The height the window opens at, whatever the length of the listing it
/// opens on.
const DEFAULT_WINDOW_HEIGHT_PX: f32 = 480.0;

/// Floor on the listing's height, so a very short screen still shows part of
/// the list.
const MIN_LISTING_HEIGHT: f32 = 100.0;

/// Lines of body text the listing keeps for the footer before the footer has
/// been drawn once, which is more than its rule, its stats line and the
/// database path take. A first frame that kept too little would put the
/// content past the window's bottom edge, and egui grows the window by the
/// difference and never back.
const FOOTER_LINES_BEFORE_IT_IS_DRAWN: f32 = 4.0;

struct PruneDialog {
    open: bool,
    mode: PruneKind,
    /// By-age input: number of days.
    age_days: u32,
    /// By-total-size input: limit in MB.
    size_limit_mb: u32,
    /// By-count input: recordings to keep per identity.
    keep_count: u32,
    /// Preview of which refs would be pruned.
    preview: Option<Vec<DatabaseRef>>,
    /// Whether a preview has been requested and is still being computed.
    preview_pending: bool,
}

impl PruneDialog {
    fn new() -> Self {
        Self {
            open: false,
            mode: PruneKind::Age,
            age_days: 90,
            size_limit_mb: 500,
            keep_count: 10,
            preview: None,
            preview_pending: false,
        }
    }

    fn reset(&mut self) {
        self.preview = None;
        self.preview_pending = false;
    }

    /// Apply a preview result that arrived from the worker.
    fn set_preview(&mut self, refs: Vec<DatabaseRef>) {
        self.preview = Some(refs);
        self.preview_pending = false;
    }

    fn to_prune_mode(&self) -> PruneMode {
        match self.mode {
            PruneKind::Age => PruneMode::ByAge {
                max_age_secs: self.age_days as u64 * 86_400,
            },
            PruneKind::TotalSize => PruneMode::ByTotalSize {
                max_bytes: self.size_limit_mb as u64 * 1_024 * 1_024,
            },
            PruneKind::Count => PruneMode::ByCount {
                keep: self.keep_count as usize,
            },
        }
    }

    /// The mode selector with the parameters it takes, and the recordings the
    /// last preview request found.
    fn body_ui(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            ui.label("Mode");
            let old = self.mode;
            ui.selectable_value(&mut self.mode, PruneKind::Age, "By age");
            ui.selectable_value(&mut self.mode, PruneKind::TotalSize, "By total size");
            ui.selectable_value(&mut self.mode, PruneKind::Count, "By count");
            if self.mode != old {
                self.reset();
            }
        });

        ui.add_space(4.0);

        let params_changed = match self.mode {
            PruneKind::Age => {
                let prev = self.age_days;
                ui.horizontal(|ui| {
                    ui.label("Remove recordings older than");
                    ui.add(DragValue::new(&mut self.age_days).range(1..=3650));
                    ui.label("days");
                });
                self.age_days != prev
            }
            PruneKind::TotalSize => {
                let prev = self.size_limit_mb;
                ui.horizontal(|ui| {
                    ui.label("Keep total size under");
                    ui.add(DragValue::new(&mut self.size_limit_mb).range(1..=100_000));
                    ui.label("MB");
                });
                self.size_limit_mb != prev
            }
            PruneKind::Count => {
                let prev = self.keep_count;
                ui.horizontal(|ui| {
                    ui.label("Keep at most");
                    ui.add(DragValue::new(&mut self.keep_count).range(1..=10_000));
                    ui.label("recordings per identity");
                });
                self.keep_count != prev
            }
        };

        if params_changed {
            // A preview for the old parameters is now stale. Drop any
            // in-flight request so its result is ignored.
            self.preview = None;
            self.preview_pending = false;
        }

        ui.add_space(4.0);
        ui.separator();

        match &self.preview {
            Some(refs) if refs.is_empty() => {
                ui.label("Nothing to prune");
            }
            Some(refs) => {
                let n = refs.len();
                let rec_label = gt_fmt::pluralize(n, "recording", "recordings");
                ui.label(format!("{n} {rec_label} will be deleted"));
                for r in refs {
                    // The recordings about to be deleted - selectable
                    // so one can be copied out before confirming. A
                    // truncated label shows its full text on hover by
                    // itself.
                    let label = format!("{}/{}", r.identity, r.group_name);
                    ui.add(Label::new(label.as_str()).truncate().selectable(true));
                }
            }
            None if self.preview_pending => {
                ui.spinner();
            }
            None => {}
        }
    }

    /// Show the Prune dialog. Sends preview/delete requests to `worker`. The
    /// results arrive asynchronously via [`HistoryWindow::set_prune_preview`].
    fn show(&mut self, ctx: &egui::Context, worker: &HistoryWorker) {
        if !self.open {
            return;
        }

        let mut open = self.open;
        let mut do_prune = false;
        let mut do_preview = false;
        let mut do_cancel_preview = false;

        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
            open = false;
        }

        Window::new(PRUNE_WINDOW_TITLE)
            .open(&mut open)
            .resizable(true)
            .collapsible(false)
            .default_width(420.0)
            .show(ctx, |ui| {
                // What the preview found, read before the closures below take
                // their own borrows of the dialog.
                let previewed = self.preview.as_ref().map(Vec::len);
                let preview_pending = self.preview_pending;
                modals::dialog_body_above_the_action_row(
                    ui,
                    DialogBody::new(|ui| self.body_ui(ui)),
                    DialogActionRow::buttons(|ui| match previewed {
                        // A preview that found nothing, and one still being
                        // computed, leave nothing to act on.
                        Some(0) => {}
                        Some(_) => {
                            if ui
                                .button(
                                    RichText::new("Delete these recordings")
                                        .color(warning_amber(ui.visuals().dark_mode)),
                                )
                                .on_hover_text(DESTRUCTIVE_DELETE_HOVER)
                                .clicked()
                            {
                                do_prune = true;
                            }
                            if ui.button("Cancel").clicked() {
                                do_cancel_preview = true;
                            }
                        }
                        None if preview_pending => {}
                        None => {
                            if ui.button("Preview").clicked() {
                                do_preview = true;
                            }
                        }
                    }),
                );
            });

        self.open = open;

        if do_preview {
            worker.prune_preview(self.to_prune_mode());
            self.preview_pending = true;
        }
        if do_cancel_preview {
            self.reset();
        }
        if do_prune {
            let refs = self.preview.take().unwrap_or_default();
            self.open = false;
            self.reset();
            if !refs.is_empty() {
                worker.delete_recordings(refs, DeleteReason::Prune);
            }
        }
    }
}

/// A column of the History table, and the order it can impose on the list.
///
/// The variants are in table order: [`table::history_table`] renders one header per
/// variant, so adding a column here is what adds it to the table.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, EnumCount, EnumIter)]
enum SortColumn {
    Identity,
    Date,
    Duration,
    Points,
    Size,
    Logs,
}

impl SortColumn {
    /// The column's header text.
    fn title(self) -> &'static str {
        match self {
            Self::Identity => "Identity",
            Self::Date => "Date",
            Self::Duration => "Duration",
            Self::Points => "Points",
            Self::Size => "Size",
            Self::Logs => "Logs",
        }
    }

    /// The direction a first click on this column sorts in: names read best
    /// from A, while dates and magnitudes are most useful biggest-first.
    fn initial_direction(self) -> SortDirection {
        match self {
            Self::Identity => SortDirection::Ascending,
            Self::Date | Self::Duration | Self::Points | Self::Size | Self::Logs => {
                SortDirection::Descending
            }
        }
    }

    /// The header's hover hint for this column in `direction`, e.g. "newest
    /// first".
    fn order_hint(self, direction: SortDirection) -> &'static str {
        match (self, direction) {
            (Self::Identity, SortDirection::Ascending) => "A to Z",
            (Self::Identity, SortDirection::Descending) => "Z to A",
            (Self::Date, SortDirection::Ascending) => "oldest first",
            (Self::Date, SortDirection::Descending) => "newest first",
            (Self::Duration | Self::Points | Self::Size | Self::Logs, SortDirection::Ascending) => {
                "smallest first"
            }
            (
                Self::Duration | Self::Points | Self::Size | Self::Logs,
                SortDirection::Descending,
            ) => "largest first",
        }
    }

    /// Order two entries by this column's value, ascending. Identity compares
    /// on the displayed name (case-insensitively), so the order matches what
    /// the column shows.
    fn compare(self, a: &RecordingEntry, b: &RecordingEntry) -> Ordering {
        match self {
            Self::Identity => compare_identities(&a.db_ref.identity, &b.db_ref.identity),
            Self::Date => a
                .meta
                .time_range
                .map(NavPointTimeRange::start_us)
                .cmp(&b.meta.time_range.map(NavPointTimeRange::start_us)),
            Self::Duration => a
                .meta
                .time_range
                .map(NavPointTimeRange::duration_us)
                .cmp(&b.meta.time_range.map(NavPointTimeRange::duration_us)),
            Self::Points => a.meta.nav_point_count.cmp(&b.meta.nav_point_count),
            Self::Size => a.meta.gtd_size_bytes.cmp(&b.meta.gtd_size_bytes),
            Self::Logs => a.log_attachments.len().cmp(&b.log_attachments.len()),
        }
    }
}

/// Which way a [`SortColumn`]'s order runs.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum SortDirection {
    Ascending,
    Descending,
}

impl SortDirection {
    fn reversed(self) -> Self {
        match self {
            Self::Ascending => Self::Descending,
            Self::Descending => Self::Ascending,
        }
    }

    /// The caret drawn beside the active column's header, pointing the way the
    /// values grow down the list.
    fn caret(self) -> &'static str {
        match self {
            Self::Ascending => ICON_CARET_UP,
            Self::Descending => ICON_CARET_DOWN,
        }
    }
}

/// How the History list is ordered.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct HistorySort {
    column: SortColumn,
    direction: SortDirection,
}

impl Default for HistorySort {
    /// Newest first, the database's own listing order.
    fn default() -> Self {
        Self {
            column: SortColumn::Date,
            direction: SortDirection::Descending,
        }
    }
}

impl HistorySort {
    /// Apply a header click: clicking the active column reverses it, clicking
    /// another switches to it in the direction that reads most naturally there.
    fn clicked(&mut self, column: SortColumn) {
        *self = if self.column == column {
            Self {
                column,
                direction: self.direction.reversed(),
            }
        } else {
            Self {
                column,
                direction: column.initial_direction(),
            }
        };
    }

    /// Order `entries` in place.
    ///
    /// Ties break on the recording's database reference, which is unique, so
    /// equal keys keep one stable order. The tie-break is independent of the
    /// chosen direction.
    fn apply(self, entries: &mut [&RecordingEntry]) {
        entries.sort_by(|a, b| {
            let by_column = match self.direction {
                SortDirection::Ascending => self.column.compare(a, b),
                SortDirection::Descending => self.column.compare(a, b).reverse(),
            };
            by_column
                .then_with(|| a.db_ref.identity.cmp(&b.db_ref.identity))
                .then_with(|| a.db_ref.group_name.cmp(&b.db_ref.group_name))
        });
    }
}

/// Compare two stored identities the way the Identity column shows them:
/// by display name, case-insensitively, without allocating a lowercased copy
/// per comparison.
fn compare_identities(a: &str, b: &str) -> Ordering {
    let a = identity_display_parts(a).0;
    let b = identity_display_parts(b).0;
    a.chars()
        .flat_map(char::to_lowercase)
        .cmp(b.chars().flat_map(char::to_lowercase))
}

pub struct HistoryWindow {
    pub open: bool,
    /// Cached recording list - `None` until the window is first shown.
    entries: Option<Vec<RecordingEntry>>,
    /// Identity substring filter (case-insensitive).
    filter_text: String,
    /// Minimum nav-point count filter (empty = no filter).
    filter_min_points: String,
    /// Maximum nav-point count filter (empty = no filter).
    filter_max_points: String,
    /// Start-date lower bound in `YYYY-MM-DD` (empty = no filter).
    filter_date_from: String,
    /// Start-date upper bound in `YYYY-MM-DD`, inclusive (empty = no filter).
    filter_date_to: String,
    /// Error from the last operation, if any.
    error: Option<String>,
    prune: PruneDialog,
    delete_hidden_prompt: DeleteHiddenTracksPrompt,
    /// Whether a recording-list request is in flight (drives the spinner and
    /// prevents re-requesting every frame while waiting).
    list_pending: bool,
    /// In-progress inline identity rename, if any.
    rename: Option<RenameEdit>,
    /// Which column the list is ordered by, and which way.
    sort: HistorySort,
    /// Height the separator and the stats footer took below the listing on the
    /// previous frame, reserved out of the room the listing may claim. The
    /// stats line's wrap point and the presence of the database-path line are
    /// known only once they are drawn. `None` before the first of those
    /// frames.
    footer_height_last_frame: Option<f32>,
}

/// What the app hands the History window on the frame it draws.
pub struct HistoryWindowFrame<'a> {
    /// The frame's instant, which the delete-hidden confirmation counts its
    /// own close down against.
    pub now: Instant,
    /// Every database operation goes here, and its result arrives
    /// asynchronously through [`HistoryWindow::set_entries`] and friends.
    pub worker: &'a HistoryWorker,
    /// The content fingerprints of the files loaded in the app, which is what
    /// disables re-opening a recording that is already open.
    pub loaded_metas: &'a [RecordingMeta],
    pub storage: &'a mut StorageSettings,
    /// Whether the startup open of the databases is still running, which is
    /// what the window shows its "opening" notice for.
    pub databases_opening: bool,
    pub write_access: WriteAccess,
}

/// State for the inline identity-rename editor on one History row.
struct RenameEdit {
    /// The current (old) identity of the row being edited - identifies the row.
    identity: String,
    /// The editable buffer, seeded with the identity's display form.
    buffer: String,
}

impl HistoryWindow {
    pub fn new() -> Self {
        Self {
            open: false,
            entries: None,
            filter_text: String::new(),
            filter_min_points: String::new(),
            filter_max_points: String::new(),
            filter_date_from: String::new(),
            filter_date_to: String::new(),
            error: None,
            prune: PruneDialog::new(),
            delete_hidden_prompt: DeleteHiddenTracksPrompt::default(),
            list_pending: false,
            rename: None,
            sort: HistorySort::default(),
            footer_height_last_frame: None,
        }
    }

    fn any_filter_active(&self) -> bool {
        !self.filter_text.is_empty()
            || !self.filter_min_points.is_empty()
            || !self.filter_max_points.is_empty()
            || !self.filter_date_from.is_empty()
            || !self.filter_date_to.is_empty()
    }

    /// Request the recording list from `worker` unless it is already cached or
    /// a request is in flight. The reply arrives via
    /// [`HistoryWindow::set_entries`].
    pub fn request_recording_list_if_missing(&mut self, worker: &HistoryWorker) {
        if self.entries.is_none() && !self.list_pending && worker.available() {
            worker.list();
            self.list_pending = true;
        }
    }

    /// The most recently started recording of the cached list, `None` until the
    /// list has arrived. Equal start times break on the database reference, so
    /// the result does not depend on the order the backend enumerated the
    /// groups in.
    pub fn latest_listed_recording(&self) -> Option<&RecordingEntry> {
        self.entries.as_ref()?.iter().max_by(|a, b| {
            a.meta
                .time_range
                .map(NavPointTimeRange::start_us)
                .cmp(&b.meta.time_range.map(NavPointTimeRange::start_us))
                .then_with(|| a.db_ref.identity.cmp(&b.db_ref.identity))
                .then_with(|| a.db_ref.group_name.cmp(&b.db_ref.group_name))
        })
    }

    /// Call after a mutation to force a list refresh next time the window shows.
    pub fn invalidate(&mut self) {
        self.entries = None;
        self.list_pending = false;
    }

    /// Apply a recording list that arrived from the worker.
    pub fn set_entries(&mut self, entries: Vec<RecordingEntry>) {
        self.entries = Some(entries);
        self.list_pending = false;
        self.error = None;
    }

    /// Record an error from a failed list request.
    pub fn set_error(&mut self, message: String) {
        self.error = Some(message);
        self.list_pending = false;
    }

    /// Apply a prune-preview result that arrived from the worker.
    pub fn set_prune_preview(&mut self, refs: Vec<DatabaseRef>) {
        self.prune.set_preview(refs);
    }

    pub fn show(
        &mut self,
        ctx: &egui::Context,
        HistoryWindowFrame {
            now,
            worker,
            loaded_metas,
            storage,
            databases_opening,
            write_access,
        }: HistoryWindowFrame<'_>,
    ) {
        if !self.open {
            return;
        }

        // A spinner shows until the list arrives.
        self.request_recording_list_if_missing(worker);

        // Show Prune dialog (a separate window).
        self.prune.show(ctx, worker);

        // Hidden tracks live inside otherwise-visible recordings (there is no
        // recording-level hide). Count them across all recordings so the toolbar
        // can offer a "Delete hidden data" action that permanently drops them.
        // The count is `None` until the recording list arrives, and `Some(0)`
        // once the list reports no hidden track.
        let hidden_track_count: Option<usize> = self
            .entries
            .as_ref()
            .map(|entries| entries.iter().map(|e| e.hidden_tracks).sum());

        let mut open = self.open;

        // Escape closes the whole window only when nothing more local claims
        // it. The confirmation dialog and an open inline rename both take it
        // first, so the key is left unconsumed here via short-circuit.
        if self.rename.is_none()
            && !self.delete_hidden_prompt.is_up()
            && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
        {
            open = false;
        }

        // Take the inline-rename state out so `render_row` can mutate it while
        // `self.entries` is borrowed immutably for the list. Restored after.
        let mut rename = std::mem::take(&mut self.rename);
        let mut footer_height = self.footer_height_last_frame;

        Window::new("History")
            .open(&mut open)
            .resizable(true)
            .default_width(640.0)
            // The height the window opens at, and the height the empty listing
            // centres its notice in.
            .default_height(DEFAULT_WINDOW_HEIGHT_PX)
            // egui keeps a resizable window's height in memory and grows it
            // towards the window's content, never back. Declaring the cap every
            // frame holds that memory to the screen the app is on now.
            .max_height(ctx.content_rect().height())
            .show(ctx, |ui| {
                if databases_opening {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(OPENING_RECORDINGS_DATABASE);
                    });
                    return;
                }
                if !worker.available() {
                    ui.label(
                        RichText::new("History database is unavailable.")
                            .color(warning_amber(ui.visuals().dark_mode)),
                    );
                    return;
                }

                if let Some(err) = &self.error {
                    ui.label(RichText::new(err).color(warning_amber(ui.visuals().dark_mode)));
                    ui.add_space(4.0);
                }

                // The count arrives with the recording list: the spinner
                // stands until both are there.
                let Some(hidden_track_count) = hidden_track_count else {
                    ui.spinner();
                    return;
                };

                // Snapshot filter active state before the closures that mutably
                // borrow individual filter fields - avoids whole-self method calls
                // inside closures where `entries` also holds an immutable borrow.
                let filter_active = self.any_filter_active();

                self.toolbar_ui(ui, storage, write_access, hidden_track_count, filter_active);

                let Some(entries) = &self.entries else {
                    return;
                };

                let filter_identity = self.filter_text.to_lowercase();
                let filter_min_points: Option<u64> = self.filter_min_points.parse().ok();
                let filter_max_points: Option<u64> = self.filter_max_points.parse().ok();
                let filter_from_us = date_to_start_us(&self.filter_date_from);
                let filter_to_us = date_to_end_us(&self.filter_date_to);

                let mut visible: Vec<&RecordingEntry> = entries
                    .iter()
                    .filter(|e| {
                        if !filter_identity.is_empty()
                            && !e
                                .db_ref
                                .identity
                                .to_lowercase()
                                .contains(filter_identity.as_str())
                        {
                            return false;
                        }
                        if filter_min_points.is_some_and(|min| e.meta.nav_point_count < min) {
                            return false;
                        }
                        if filter_max_points.is_some_and(|max| e.meta.nav_point_count > max) {
                            return false;
                        }
                        let started_at = e.meta.time_range.map(NavPointTimeRange::start_us);
                        if filter_from_us
                            .is_some_and(|from| started_at.is_none_or(|start| start < from))
                        {
                            return false;
                        }
                        if filter_to_us.is_some_and(|to| started_at.is_none_or(|start| start > to))
                        {
                            return false;
                        }
                        true
                    })
                    .collect();
                self.sort.apply(&mut visible);

                if entries.is_empty() {
                    ui.centered_and_justified(|ui| {
                        ui.label("No recordings in history yet");
                    });
                    return;
                }

                // The listing takes the room the window leaves below the
                // toolbar, minus the footer that sits under it, and scrolls its
                // rows inside that. The window is as tall as the user left it,
                // however long the listing is.
                let footer_room = footer_height.unwrap_or_else(|| {
                    FOOTER_LINES_BEFORE_IT_IS_DRAWN * ui.text_style_height(&egui::TextStyle::Body)
                });
                let max_listing_height =
                    (ui.available_height() - footer_room).max(MIN_LISTING_HEIGHT);

                // The listing scrolls sideways once its metadata columns alone
                // need more width than the window has, identity having clamped
                // to its minimum by then.
                ScrollArea::horizontal()
                    .id_salt("history_listing")
                    .show(ui, |ui| {
                        table::history_table(
                            ui,
                            table::HistoryTable {
                                max_listing_height,
                                visible: &visible,
                                loaded_metas,
                                worker,
                                rename: &mut rename,
                                sort: &mut self.sort,
                                write_access,
                            },
                        );
                    });
                let listing_bottom = ui.min_rect().bottom();

                ui.separator();
                // Footer stats cover every stored recording. Hidden tracks are
                // reported separately since they are pending permanent
                // deletion.
                let stored_count = entries.len();
                let total_size: u64 = entries.iter().map(|e| e.meta.gtd_size_bytes).sum();
                ui.horizontal_wrapped(|ui| {
                    let rec_label = gt_fmt::pluralize(stored_count, "recording", "recordings");
                    ui.label(format!(
                        "{stored_count} {rec_label} - {}",
                        gt_fmt::format_bytes(total_size)
                    ));
                    if filter_active && visible.len() != stored_count {
                        ui.weak(format!("({} shown)", visible.len()));
                    }
                    if hidden_track_count > 0 {
                        let track_label = gt_fmt::pluralize(hidden_track_count, "track", "tracks");
                        ui.weak(format!("- {hidden_track_count} hidden {track_label}"));
                    }
                });
                if let Some(path) = worker.path() {
                    // Kept selectable for copying.
                    ui.add(
                        Label::new(RichText::new(path.display().to_string()).weak())
                            .truncate()
                            .selectable(true),
                    );
                }

                footer_height = Some(ui.min_rect().bottom() - listing_bottom);
            });

        self.rename = rename;
        self.footer_height_last_frame = footer_height;

        if self
            .delete_hidden_prompt
            .show(ctx, now, hidden_track_count)
            .is_some()
        {
            worker.delete_hidden_tracks();
        }

        self.open = open;
    }

    /// The toolbar above the listing: the identity filter and the actions on
    /// the stored recordings, the point and date filters, and the auto-prune
    /// settings.
    ///
    /// Scrolls sideways when the window is narrower than its controls.
    fn toolbar_ui(
        &mut self,
        ui: &mut egui::Ui,
        storage: &mut StorageSettings,
        write_access: WriteAccess,
        hidden_track_count: usize,
        filter_active: bool,
    ) {
        ScrollArea::horizontal()
            .id_salt("history_toolbar")
            .show(ui, |ui| {
                // Toolbar row: identity filter on the left, actions on the
                // right. The right-side controls are laid out right-to-left so
                // they claim their width first. The filter field then fills only
                // the space between the label and them. Adding the field in the
                // outer left-to-right layout instead lets it grow into the
                // right-side controls and overlap them once the window narrows.
                ui.horizontal(|ui| {
                    LabelWithHover::underlined_term(RichText::new("Identity"))
                        .explanation_ui(ui, crate::terms::IDENTITY);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let delete_hidden_label = if hidden_track_count > 0 {
                            format!("Delete hidden data ({hidden_track_count})…")
                        } else {
                            "Delete hidden data…".to_owned()
                        };
                        let writes_recordings = write_access.allows_writing();
                        let delete_hidden = ui
                        .add_enabled(
                            hidden_track_count > 0 && writes_recordings,
                            Button::new(delete_hidden_label),
                        )
                        .on_hover_text(
                            "Permanently delete every hidden track from the original recordings",
                        )
                        .on_disabled_hover_text(if writes_recordings {
                            "No hidden tracks to delete"
                        } else {
                            READ_ONLY_RECORDING_HISTORY_HOVER
                        });
                        if delete_hidden.clicked() {
                            self.delete_hidden_prompt.open(hidden_track_count);
                        }
                        if ui
                            .add_enabled(writes_recordings, Button::new("Prune…"))
                            .on_hover_text("Delete stored recordings by age, total size or count")
                            .on_disabled_hover_text(READ_ONLY_RECORDING_HISTORY_HOVER)
                            .clicked()
                        {
                            self.prune.open = true;
                            self.prune.reset();
                        }
                        storage_controls::show_auto_store_checkbox(ui, storage, write_access);
                        ui.add(
                            TextEdit::singleline(&mut self.filter_text)
                                .desired_width(ui.available_width()),
                        );
                    });
                });

                // Advanced filter row: points + date range. Wraps onto a
                // second line, keeping the window's width.
                ui.horizontal_wrapped(|ui| {
                    ui.label("Points ≥");
                    ui.add(TextEdit::singleline(&mut self.filter_min_points).desired_width(60.0));
                    ui.label("≤");
                    ui.add(TextEdit::singleline(&mut self.filter_max_points).desired_width(60.0));
                    ui.separator();
                    ui.label("Date");
                    ui.add(
                        TextEdit::singleline(&mut self.filter_date_from)
                            .desired_width(90.0)
                            .hint_text("YYYY-MM-DD"),
                    );
                    ui.label("–");
                    ui.add(
                        TextEdit::singleline(&mut self.filter_date_to)
                            .desired_width(90.0)
                            .hint_text("YYYY-MM-DD"),
                    );
                    if filter_active && ui.small_button(ICON_X).clicked() {
                        self.filter_text.clear();
                        self.filter_min_points.clear();
                        self.filter_max_points.clear();
                        self.filter_date_from.clear();
                        self.filter_date_to.clear();
                    }
                });

                // Auto-prune settings - separated because this is a persistent
                // setting, not a filter or list entry.
                ui.separator();
                ui.horizontal_wrapped(|ui| {
                    storage_controls::show_auto_prune_limit(ui, storage, write_access);
                    ui.separator();
                    storage_controls::show_auto_prune_confirm_checkbox(ui, storage, write_access);
                });
                ui.add_space(4.0);
            });
    }
}

fn identity_display_parts(identity: &str) -> (&str, bool) {
    gt_loaded_files::display_identity(identity)
}

/// Display form of a History entry's raw travel-mode wire value (the DB stores
/// the `meta_travel_mode` attribute verbatim): known modes get their human
/// spelling, unknown wire values pass through verbatim.
fn travel_mode_display(wire: &str) -> String {
    TravelMode::from_wire(wire).display_name().to_owned()
}

fn format_duration(dur: chrono::Duration) -> String {
    let total_secs = dur.num_seconds().max(0);
    let d = total_secs / 86400;
    let h = (total_secs % 86400) / 3600;
    let m = (total_secs % 3600) / 60;
    let s = total_secs % 60;
    if d > 0 {
        format!("{d}d {h}h")
    } else if h > 0 {
        format!("{h}h {m:02}m")
    } else if m > 0 {
        format!("{m}m {s:02}s")
    } else {
        format!("{s}s")
    }
}

/// Parse a `YYYY-MM-DD` string into microseconds-since-epoch at the start of that day (UTC).
/// Returns `None` if the string is empty or not a valid date.
fn date_to_start_us(s: &str) -> Option<i64> {
    if s.is_empty() {
        return None;
    }
    let date = NaiveDate::parse_from_str(s, "%Y-%m-%d").ok()?;
    let dt = date.and_hms_opt(0, 0, 0)?.and_utc();
    Some(dt.timestamp_micros())
}

/// Parse a `YYYY-MM-DD` string into microseconds-since-epoch at the end of that day (UTC),
/// so the "to" bound is inclusive.
/// Returns `None` if the string is empty or not a valid date.
fn date_to_end_us(s: &str) -> Option<i64> {
    if s.is_empty() {
        return None;
    }
    let date = NaiveDate::parse_from_str(s, "%Y-%m-%d").ok()?;
    let dt = date.and_hms_opt(23, 59, 59)?.and_utc();
    Some(dt.timestamp_micros())
}

#[cfg(test)]
mod tests;

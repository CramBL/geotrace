use std::cmp::Ordering;

use chrono::NaiveDate;
use egui::{Button, Checkbox, DragValue, Label, RichText, ScrollArea, TextEdit, Window};
use egui_phosphor::regular::CARET_DOWN as ICON_CARET_DOWN;
use egui_phosphor::regular::CARET_UP as ICON_CARET_UP;
use egui_phosphor::regular::X as ICON_X;
use gt_store::{DatabaseRef, PruneMode, RecordingEntry, RecordingMeta};
use gt_types::TravelMode;
use gt_ui_theme::warning_amber;
use strum::{EnumCount, EnumIter};

use crate::app::history_db::{DeleteReason, HistoryWorker};

mod table;

/// Turn off label text-selection for a History window's contents.
///
/// egui labels default to selectable. Anything worth copying opts back in
/// with [`Label::selectable`].
fn use_plain_labels(ui: &mut egui::Ui) {
    ui.style_mut().interaction.selectable_labels = false;
}

/// Which pruning mode is selected in the Prune dialog.
#[derive(Clone, Copy, PartialEq, Eq)]
enum PruneKind {
    Age,
    TotalSize,
    Count,
}

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

        Window::new("Prune History…")
            .open(&mut open)
            .resizable(true)
            .collapsible(false)
            .default_width(420.0)
            .show(ctx, |ui| {
                use_plain_labels(ui);
                ui.horizontal(|ui| {
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
                            ui.add(
                                DragValue::new(&mut self.size_limit_mb).range(1..=100_000),
                            );
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

                // Preview button / spinner / computed preview
                if let Some(refs) = &self.preview {
                    if refs.is_empty() {
                        ui.label("Nothing to prune");
                    } else {
                        let n = refs.len();
                        let rec_label = gt_fmt::pluralize(n, "recording", "recordings");
                        ui.label(format!("{n} {rec_label} will be deleted"));
                        ScrollArea::vertical()
                            .max_height(200.0)
                            .show(ui, |ui| {
                                for r in refs {
                                    // The recordings about to be deleted -
                                    // selectable so one can be copied out
                                    // before confirming. A truncated label
                                    // shows its full text on hover by itself.
                                    let label = format!("{}/{}", r.identity, r.group_name);
                                    ui.add(
                                        Label::new(label.as_str()).truncate().selectable(true),
                                    );
                                }
                            });

                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            let confirm_btn = ui
                                .button(
                                    RichText::new("Delete these recordings")
                                        .color(warning_amber(ui.visuals().dark_mode)),
                                )
                                .on_hover_text(
                                    "This cannot be undone. The original source files are unaffected.",
                                );
                            if confirm_btn.clicked() {
                                do_prune = true;
                            }
                            if ui.button("Cancel").clicked() {
                                do_cancel_preview = true;
                            }
                        });
                    }
                } else if self.preview_pending {
                    ui.spinner();
                } else if ui.button("Preview").clicked() {
                    do_preview = true;
                }
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
        }
    }

    /// The direction a first click on this column sorts in: names read best
    /// from A, while dates and magnitudes are most useful biggest-first.
    fn initial_direction(self) -> SortDirection {
        match self {
            Self::Identity => SortDirection::Ascending,
            Self::Date | Self::Duration | Self::Points | Self::Size => SortDirection::Descending,
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
            (Self::Duration | Self::Points | Self::Size, SortDirection::Ascending) => {
                "smallest first"
            }
            (Self::Duration | Self::Points | Self::Size, SortDirection::Descending) => {
                "largest first"
            }
        }
    }

    /// Order two entries by this column's value, ascending. Identity compares
    /// on the displayed name (case-insensitively), so the order matches what
    /// the column shows.
    fn compare(self, a: &RecordingEntry, b: &RecordingEntry) -> Ordering {
        match self {
            Self::Identity => compare_identities(&a.db_ref.identity, &b.db_ref.identity),
            Self::Date => a.meta.start_us.cmp(&b.meta.start_us),
            Self::Duration => duration_us(&a.meta).cmp(&duration_us(&b.meta)),
            Self::Points => a.meta.nav_point_count.cmp(&b.meta.nav_point_count),
            Self::Size => a.meta.gtd_size_bytes.cmp(&b.meta.gtd_size_bytes),
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

/// A recording's span in microseconds, clamped at zero for a recording whose
/// stored end precedes its start.
fn duration_us(meta: &RecordingMeta) -> i64 {
    meta.end_us.saturating_sub(meta.start_us).max(0)
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
    delete_hidden_confirm_open: bool,
    /// Whether a recording-list request is in flight (drives the spinner and
    /// prevents re-requesting every frame while waiting).
    list_pending: bool,
    /// In-progress inline identity rename, if any.
    rename: Option<RenameEdit>,
    /// Which column the list is ordered by, and which way.
    sort: HistorySort,
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
            delete_hidden_confirm_open: false,
            list_pending: false,
            rename: None,
            sort: HistorySort::default(),
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
                .start_us
                .cmp(&b.meta.start_us)
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

    /// Show the History window. All database work is sent to `worker`. Results
    /// arrive asynchronously and are applied via [`HistoryWindow::set_entries`]
    /// and friends.
    ///
    /// `loaded_metas` are the content fingerprints of the files currently loaded
    /// in the app, used to disable re-opening a recording that is already open.
    #[expect(
        clippy::too_many_arguments,
        reason = "the window drives several independent pieces of persisted app state plus the loaded-file set; bundling them would obscure rather than clarify"
    )]
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        worker: &HistoryWorker,
        loaded_metas: &[RecordingMeta],
        storage_enabled: &mut bool,
        auto_prune_enabled: &mut bool,
        auto_prune_max_bytes: &mut u64,
        auto_prune_confirm: &mut bool,
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
        let hidden_count: usize = self
            .entries
            .as_ref()
            .map(|entries| entries.iter().map(|e| e.hidden_tracks).sum())
            .unwrap_or_default();

        let mut open = self.open;

        // Escape closes the whole window only when nothing more local claims
        // it. The confirmation dialog and an open inline rename both take it
        // first, so the key is left unconsumed here via short-circuit.
        if self.rename.is_none()
            && !self.delete_hidden_confirm_open
            && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
        {
            open = false;
        }

        // Take the inline-rename state out so `render_row` can mutate it while
        // `self.entries` is borrowed immutably for the list. Restored after.
        let mut rename = std::mem::take(&mut self.rename);

        Window::new("History")
            .open(&mut open)
            .resizable(true)
            .default_width(640.0)
            .default_height(480.0)
            .show(ctx, |ui| {
                use_plain_labels(ui);
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

                if self.entries.is_none() {
                    ui.spinner();
                    return;
                }

                // Snapshot filter active state before the closures that mutably
                // borrow individual filter fields - avoids whole-self method calls
                // inside closures where `entries` also holds an immutable borrow.
                let filter_active = self.any_filter_active();

                // Toolbar row: identity filter on the left, actions on the
                // right. The right-side controls are laid out right-to-left so
                // they claim their width first. The filter field then fills only
                // the space between the label and them. Adding the field in the
                // outer left-to-right layout instead lets it grow into the
                // right-side controls and overlap them once the window narrows.
                ui.horizontal(|ui| {
                    crate::terms::term_label(
                        ui,
                        RichText::new("Identity"),
                        crate::terms::IDENTITY,
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let delete_hidden_label = if hidden_count > 0 {
                            format!("Delete hidden data ({hidden_count})…")
                        } else {
                            "Delete hidden data…".to_owned()
                        };
                        let delete_hidden = ui
                            .add_enabled(hidden_count > 0, Button::new(delete_hidden_label))
                            .on_hover_text(if hidden_count > 0 {
                                "Permanently delete every hidden track from the original recordings"
                            } else {
                                "No hidden tracks to delete"
                            });
                        if delete_hidden.clicked() {
                            self.delete_hidden_confirm_open = true;
                        }
                        if ui.button("Prune…").clicked() {
                            self.prune.open = true;
                            self.prune.reset();
                        }
                        ui.checkbox(storage_enabled, "Auto-store recordings");
                        ui.add(
                            TextEdit::singleline(&mut self.filter_text)
                                .desired_width(ui.available_width()),
                        );
                    });
                });

                // Advanced filter row: points + date range
                ui.horizontal(|ui| {
                    ui.label("Points ≥");
                    ui.add(
                        TextEdit::singleline(&mut self.filter_min_points).desired_width(60.0),
                    );
                    ui.label("≤");
                    ui.add(
                        TextEdit::singleline(&mut self.filter_max_points).desired_width(60.0),
                    );
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
                // setting, not a filter or list entry.  Always rendered so the
                // layout stays stable, controls are grayed out when inactive,
                // with hover text explaining what to enable first.
                ui.separator();
                ui.horizontal(|ui| {
                    let storage_on = *storage_enabled;
                    let prune_on = *auto_prune_enabled && storage_on;

                    ui.add_enabled(
                        storage_on,
                        Checkbox::new(auto_prune_enabled, "Auto-prune when over"),
                    )
                    .on_hover_text(if storage_on {
                        "Automatically delete the oldest recordings when storage exceeds the threshold"
                    } else {
                        "Enable 'Auto-store recordings' to use auto-pruning"
                    });

                    let mut max_gb = *auto_prune_max_bytes as f64 / gt_fmt::BYTES_PER_GB as f64;
                    ui.add_enabled(
                        prune_on,
                        DragValue::new(&mut max_gb)
                            .range(0.1..=1_000.0)
                            .speed(0.1),
                    )
                    .on_hover_text(if prune_on {
                        "Storage limit - oldest recordings are pruned when this is exceeded"
                    } else if storage_on {
                        "Tick 'Auto-prune when over' to set a threshold"
                    } else {
                        "Enable 'Auto-store recordings' to use auto-pruning"
                    });

                    if prune_on {
                        #[expect(
                            clippy::cast_sign_loss,
                            reason = "DragValue range is 0.1..=1000 so value is always positive"
                        )]
                        let bytes = (max_gb * gt_fmt::BYTES_PER_GB as f64).round() as u64;
                        *auto_prune_max_bytes = bytes;
                    }

                    ui.label("GB");

                    ui.separator();

                    ui.add_enabled(
                        prune_on,
                        Checkbox::new(auto_prune_confirm, "Confirm before pruning"),
                    )
                    .on_hover_text(if prune_on {
                        "Show a confirmation dialog before auto-pruning deletes recordings"
                    } else if storage_on {
                        "Tick 'Auto-prune when over' to configure this"
                    } else {
                        "Enable 'Auto-store recordings' to use auto-pruning"
                    });
                });
                ui.add_space(4.0);

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
                        if filter_from_us.is_some_and(|from| e.meta.start_us < from) {
                            return false;
                        }
                        if filter_to_us.is_some_and(|to| e.meta.start_us > to) {
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

                // Reserve space for stats footer
                let footer_height = ui.text_style_height(&egui::TextStyle::Body) + 8.0;
                let available = ui.available_size();
                let list_height = (available.y - footer_height - 8.0).max(100.0);

                table::history_table(
                    ui,
                    list_height,
                    &visible,
                    loaded_metas,
                    worker,
                    &mut rename,
                    &mut self.sort,
                );

                ui.separator();
                // Footer stats cover every stored recording. Hidden tracks are
                // reported separately since they are pending permanent deletion.
                let stored_count = entries.len();
                let total_size: u64 = entries.iter().map(|e| e.meta.gtd_size_bytes).sum();
                ui.horizontal(|ui| {
                    let rec_label = gt_fmt::pluralize(stored_count, "recording", "recordings");
                    ui.label(format!(
                        "{stored_count} {rec_label} - {}",
                        gt_fmt::format_bytes(total_size)
                    ));
                    if filter_active && visible.len() != stored_count {
                        ui.weak(format!("({} shown)", visible.len()));
                    }
                    if hidden_count > 0 {
                        let track_label = gt_fmt::pluralize(hidden_count, "track", "tracks");
                        ui.weak(format!("- {hidden_count} hidden {track_label}"));
                    }
                });
                if let Some(path) = worker.path() {
                    // Kept selectable for copying.
                    ui.add(
                        Label::new(RichText::new(path.display().to_string()).weak())
                            .selectable(true),
                    );
                }
            });

        self.rename = rename;

        // Confirmation for the destructive "delete hidden data" action, mirroring
        // the prune/auto-prune confirm flow (no permanent delete without a prompt).
        if self.delete_hidden_confirm_open {
            if hidden_count == 0 {
                self.delete_hidden_confirm_open = false;
            } else {
                let mut do_delete = false;
                let mut cancel =
                    ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
                Window::new("Delete hidden data?")
                    .collapsible(false)
                    .resizable(false)
                    .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                    .show(ctx, |ui| {
                        use_plain_labels(ui);
                        let track_label = gt_fmt::pluralize(hidden_count, "track", "tracks");
                        ui.label(format!(
                            "{hidden_count} hidden {track_label} will be permanently removed from their recordings."
                        ));
                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            if ui
                                .button(
                                    RichText::new("Delete hidden tracks")
                                        .color(warning_amber(ui.visuals().dark_mode)),
                                )
                                .on_hover_text(
                                    "This cannot be undone. The original source files are unaffected.",
                                )
                                .clicked()
                            {
                                do_delete = true;
                            }
                            if ui.button("Cancel").clicked() {
                                cancel = true;
                            }
                        });
                    });
                if do_delete {
                    worker.delete_hidden_tracks();
                    self.delete_hidden_confirm_open = false;
                } else if cancel {
                    self.delete_hidden_confirm_open = false;
                }
            }
        }

        self.open = open;
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

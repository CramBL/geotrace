use std::cell::Cell;
use std::cmp::Ordering;

use chrono::{DateTime, NaiveDate, Utc};
use egui::{Button, Checkbox, DragValue, Grid, Label, RichText, ScrollArea, TextEdit, Window};
use egui_extras::{Column, TableBuilder, TableRow};
use egui_phosphor::regular::CARET_DOWN as ICON_CARET_DOWN;
use egui_phosphor::regular::CARET_UP as ICON_CARET_UP;
use egui_phosphor::regular::NOTE as ICON_NOTE;
use egui_phosphor::regular::X as ICON_X;
use gt_side_panel::widgets::{MetadataView, has_metadata_details, metadata_detail_rows};
use gt_store::{ChannelSummary, DatabaseRef, PruneMode, RecordingEntry, RecordingMeta};
use gt_types::TravelMode;
use gt_ui_theme::{EM_DASH, warning_amber};
use strum::{EnumCount, EnumIter, IntoEnumIterator as _};

use crate::app::history_db::{DeleteReason, HistoryWorker};

/// Turn off label text-selection for a History window's contents.
///
/// egui makes every label selectable by default, which puts a text-editing
/// I-beam under the pointer over each one. That reads as an invitation to type
/// where there is nothing to type: these windows are captions, values, and
/// controls rather than prose, and a column header that sorts on click was
/// showing the same I-beam as a text field.
///
/// Anything genuinely worth copying opts back in with [`Label::selectable`],
/// and keeps the I-beam as the signal that it can be.
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
/// The variants are in table order: [`history_table`] renders one header per
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

    /// How the given order reads for this column, for the header's hover hint -
    /// "newest first" says more about a date column than "descending" does.
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
    /// the column actually shows rather than the stored `auto:`-prefixed form.
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
    /// Newest first, matching the order the database itself lists recordings
    /// in - so opening the window shows what it always did until sorted.
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
    /// Ties break on the recording's database reference, which is unique - so
    /// equal keys (two same-size recordings, say) keep one stable order instead
    /// of shuffling between frames, and the tie-break stays independent of the
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
    /// Whether the "delete hidden data" confirmation dialog is open.
    confirm_delete_hidden: bool,
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
            confirm_delete_hidden: false,
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

    /// Ask `worker` for the recording list unless it is already cached or a
    /// request is in flight. The reply arrives via [`HistoryWindow::set_entries`].
    pub fn request_recording_list_if_missing(&mut self, worker: &HistoryWorker) {
        if self.entries.is_none() && !self.list_pending && worker.available() {
            worker.list();
            self.list_pending = true;
        }
    }

    /// The most recently started recording of the cached list, `None` until the
    /// list has arrived. Equal start times break on the database reference, so
    /// the answer does not depend on the order the backend enumerated the
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

        // Escape closes the whole window only when nothing more local wants it:
        // while the confirmation dialog is up it dismisses that, and while an
        // inline rename is open it must reach the editor to cancel it (so the
        // key is left unconsumed here in that case, via short-circuit).
        if self.rename.is_none()
            && !self.confirm_delete_hidden
            && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
        {
            open = false;
        }

        // Take the inline-rename state out so `render_row` can mutate it while
        // `self.entries` is borrowed immutably for the list; restored after.
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
                // they claim their width first; the filter field then fills only
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
                            self.confirm_delete_hidden = true;
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

                history_table(
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
                    // Worth copying out of the app, so this one keeps text
                    // selection - and the I-beam that advertises it.
                    ui.add(
                        Label::new(RichText::new(path.display().to_string()).weak())
                            .selectable(true),
                    );
                }
            });

        self.rename = rename;

        // Confirmation for the destructive "delete hidden data" action, mirroring
        // the prune/auto-prune confirm flow (no permanent delete without a prompt).
        if self.confirm_delete_hidden {
            if hidden_count == 0 {
                self.confirm_delete_hidden = false;
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
                    self.confirm_delete_hidden = false;
                } else if cancel {
                    self.confirm_delete_hidden = false;
                }
            }
        }

        self.open = open;
    }
}

/// The scrolling recordings table, laid out like a file manager's list: the
/// metadata columns (date, duration, points, size, actions) size to their
/// fixed-format content, and the identity column fills whatever width is left,
/// clipping long names. Its width is a function of the window's width, so the
/// table is always exactly as wide as the window - resize the window to give
/// identity more or less room.
///
/// Identity is sized as a [`Column::exact`] recomputed each frame rather than a
/// [`Column::remainder`] on purpose. A remainder that is not the *last* column
/// ratchets in egui_extras: it feeds its clipped width back into its own minimum
/// every frame, so it can never shrink again, which stops the window from being
/// made narrower and lets it creep wider. Computing the width ourselves sidesteps
/// that: the table always fits the window, so the window stays freely resizable.
fn history_table(
    ui: &mut egui::Ui,
    list_height: f32,
    visible: &[&RecordingEntry],
    loaded_metas: &[gt_store::RecordingMeta],
    worker: &HistoryWorker,
    rename: &mut Option<RenameEdit>,
    sort: &mut HistorySort,
) {
    let row_height = ui.text_style_height(&egui::TextStyle::Body) + 6.0;

    // Identity fills the width the metadata columns leave over. We size it as an
    // exact column ourselves (window width minus last frame's metadata width)
    // rather than a `Column::remainder`, whose ratcheting breaks window shrink -
    // see this function's doc comment for the full rationale.
    let available_width = ui.available_width();
    let metadata_width_id = ui.id().with("history_metadata_width");
    let identity_width = ui
        .data(|d| d.get_temp::<f32>(metadata_width_id))
        .map_or(IDENTITY_DEFAULT_WIDTH, |metadata| {
            (available_width - metadata).max(IDENTITY_MIN_WIDTH)
        });

    // Right edges of the identity and last (action) columns, captured from the
    // header this frame to measure the metadata width for the next one. The
    // measurement is only trusted outside the table's sizing pass: during it the
    // auto columns have not yet grown to their content, so the reserve reads too
    // small and identity briefly blows up (window sticks wide).
    let identity_right = Cell::new(0.0_f32);
    let last_column_right = Cell::new(0.0_f32);
    let measured_while_sizing = Cell::new(false);

    TableBuilder::new(ui)
        .id_salt("history_list")
        .striped(true)
        // Cells lay out in a row (no vertical wrapping): dates stay on one
        // line and the action buttons sit side by side.
        .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
        // Don't shrink to content: the table fills the window's width, which the
        // computed identity column already accounts for.
        .auto_shrink([false, true])
        .max_scroll_height(list_height)
        // Identity fills the leftover width (see above) and clips long names
        // rather than growing to fit them.
        .column(Column::exact(identity_width).clip(true))
        // Metadata columns size to their fixed-format content - every sortable
        // column except identity, which was added above. They are not
        // resizable: there is nothing to gain from resizing a date or a byte
        // count, and it keeps the table's width fully determined by the window.
        .columns(Column::auto().resizable(false), SortColumn::COUNT - 1)
        .column(Column::auto().resizable(false))
        .header(row_height, |mut header| {
            // Driven off the enum rather than a parallel list of titles, so a
            // new sortable column cannot be added without a header appearing.
            for column in SortColumn::iter() {
                header.col(|ui| {
                    // Identity is the measured, term-explained column; every
                    // other one is a plain sortable header.
                    let term = (column == SortColumn::Identity).then(|| {
                        identity_right.set(ui.max_rect().right());
                        measured_while_sizing.set(ui.is_sizing_pass());
                        crate::terms::IDENTITY
                    });
                    sort_header(ui, column, sort, term);
                });
            }
            header.col(|ui| {
                last_column_right.set(ui.max_rect().right());
            });
        })
        .body(|body| {
            body.rows(row_height, visible.len(), |mut row| {
                // In-range by construction: `rows` hands out indices below
                // `visible.len()`; skip defensively rather than unwrap.
                let Some(entry) = visible.get(row.index()) else {
                    return;
                };
                let already_loaded = loaded_metas.iter().any(|m| m.same_recording(&entry.meta));
                render_row(&mut row, entry, already_loaded, worker, rename);
            });
        });

    // Record the metadata columns' total width (everything right of identity)
    // for next frame's fill calculation. The header always renders - unlike the
    // virtualized body rows - so these edges are always fresh.
    let metadata_width = last_column_right.get() - identity_right.get();
    if metadata_width > 0.0 && !measured_while_sizing.get() {
        ui.data_mut(|d| d.insert_temp(metadata_width_id, metadata_width));
    }
}

/// A clickable table header that orders the list by `column`.
///
/// The active column carries a caret pointing the way its values run; clicking
/// it reverses that, clicking any other column switches to it. `term`, when
/// given, is the column's glossary explanation - it underlines the title and
/// leads the hover, matching [`crate::terms::term_label`].
fn sort_header(ui: &mut egui::Ui, column: SortColumn, sort: &mut HistorySort, term: Option<&str>) {
    let active = sort.column == column;
    let mut title = RichText::new(column.title()).strong();
    if term.is_some() {
        title = title.underline();
    }

    let clicked = ui
        .horizontal(|ui| {
            let title = ui.add(
                Label::new(title)
                    .selectable(false)
                    .sense(egui::Sense::click()),
            );
            if active {
                ui.label(RichText::new(sort.direction.caret()).small().weak());
            }
            title
        })
        .inner
        // A header sorts on click, so the pointer says "clickable" - not the
        // text I-beam a selectable label would otherwise put here.
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_ui(|ui| {
            if let Some(term) = term {
                ui.label(term);
            }
            // Name the order the click produces, not the one already applied,
            // so the hint reads as the action it is.
            let next = if active {
                sort.direction.reversed()
            } else {
                column.initial_direction()
            };
            ui.label(
                RichText::new(format!("Click to sort {}", column.order_hint(next)))
                    .small()
                    .color(ui.visuals().weak_text_color()),
            );
        })
        .clicked();

    if clicked {
        sort.clicked(column);
    }
}

/// Identity never collapses below a readable width, even in a narrow window.
const IDENTITY_MIN_WIDTH: f32 = 160.0;

/// Identity's width on the very first frame, before the metadata columns have
/// been measured (see [`history_table`]); from then on it fills the leftover
/// width. Kept above [`IDENTITY_MIN_WIDTH`] so this bootstrap value is already a
/// readable width without needing the same clamp the measured path applies.
const IDENTITY_DEFAULT_WIDTH: f32 = 280.0;

fn render_row(
    row: &mut TableRow<'_, '_>,
    entry: &RecordingEntry,
    already_loaded: bool,
    worker: &HistoryWorker,
    rename: &mut Option<RenameEdit>,
) {
    // Identity column: the inline editor when this row is being renamed,
    // otherwise the normal cell.
    row.col(|ui| {
        if rename
            .as_ref()
            .is_some_and(|r| r.identity == entry.db_ref.identity)
        {
            render_rename_editor(ui, rename, worker);
        } else {
            identity_cell(ui, entry, worker, rename);
        }
    });

    breakdown_cell(row, entry, SortColumn::Date, |ui| {
        let ts = DateTime::<Utc>::from_timestamp_micros(entry.meta.start_us)
            .unwrap_or_default()
            .format("%Y-%m-%d %H:%M")
            .to_string();
        ui.label(ts);
    });

    breakdown_cell(row, entry, SortColumn::Duration, |ui| {
        let dur = chrono::Duration::microseconds(duration_us(&entry.meta));
        ui.label(format_duration(dur));
    });

    breakdown_cell(row, entry, SortColumn::Points, |ui| {
        ui.label(gt_store::format_count_suffix(entry.meta.nav_point_count));
        if entry.hidden_tracks > 0 {
            ui.weak(format!(
                "({}/{} hidden)",
                entry.hidden_tracks, entry.total_tracks
            ));
        }
    });

    breakdown_cell(row, entry, SortColumn::Size, |ui| {
        ui.label(gt_fmt::format_bytes(entry.meta.gtd_size_bytes));
    });

    row.col(|ui| {
        let open = ui.add_enabled(!already_loaded, Button::new("Open").small());
        if already_loaded {
            open.on_hover_text("Already loaded");
        } else if open.clicked() {
            worker.open(entry.db_ref.clone());
        }
        if ui.small_button("Delete").clicked() {
            worker.delete_recordings(vec![entry.db_ref.clone()], DeleteReason::Manual);
        }
    });
}

/// Render one of a row's value cells and give the whole cell - the text and the
/// blank space beside it - the recording's data breakdown as hover text.
///
/// Nothing inside a value cell senses hover or click of its own, so covering
/// the cell's whole rect is what makes the breakdown reachable by pointing
/// anywhere along the row rather than only at the label.
fn breakdown_cell(
    row: &mut TableRow<'_, '_>,
    entry: &RecordingEntry,
    column: SortColumn,
    content: impl FnOnce(&mut egui::Ui),
) {
    row.col(|ui| {
        let cell = ui.max_rect();
        content(ui);
        ui.interact(cell, breakdown_cell_id(entry, column), egui::Sense::hover())
            .on_hover_ui(|ui| data_breakdown_ui(ui, entry));
    });
}

/// The widget id of a row's breakdown cell.
///
/// The recording's database reference identifies the row and `column` separates
/// the cells within it, so no two breakdown cells in the table share an id and
/// none collides with a neighbour's interaction state.
fn breakdown_cell_id(entry: &RecordingEntry, column: SortColumn) -> egui::Id {
    egui::Id::new((
        "history_row_breakdown",
        entry.db_ref.identity.as_str(),
        entry.db_ref.group_name.as_str(),
        column,
    ))
}

/// What the recording holds, as hover detail for a History row: its exact span,
/// its shape on disk, and a count per kind of data - including the ad-hoc sensor
/// channels, which no table column reveals.
fn data_breakdown_ui(ui: &mut egui::Ui, entry: &RecordingEntry) {
    let meta = &entry.meta;
    let start = DateTime::<Utc>::from_timestamp_micros(meta.start_us).unwrap_or_default();
    let end = DateTime::<Utc>::from_timestamp_micros(meta.end_us).unwrap_or_default();
    ui.label(gt_fmt::format_time_range(start, end));

    Grid::new("history_breakdown_counts")
        .num_columns(2)
        .spacing([12.0, 4.0])
        .show(ui, |ui| {
            let mut row = |label: &str, value: String| {
                ui.label(RichText::new(label).weak());
                ui.label(value);
                ui.end_row();
            };
            row(
                "Duration",
                format_duration(chrono::Duration::microseconds(duration_us(meta))),
            );
            row("Size", gt_fmt::format_bytes(meta.gtd_size_bytes));
            row("Tracks", track_count_text(entry));
            row("Nav points", format_stored_count(meta.nav_point_count));
            row(
                "Satellite reports",
                format_stored_count(meta.sat_report_count),
            );
            row("Markers", format_stored_count(meta.marker_count));
            row(
                "Event markers",
                format_stored_count(meta.event_marker_count),
            );
        });

    if entry.hidden_tracks > 0 {
        ui.label(
            RichText::new(
                "Hidden tracks came from 'remove filtered data'. \
                 Use 'Delete hidden data' to drop them permanently.",
            )
            .small()
            .color(ui.visuals().weak_text_color()),
        );
    }

    channels_breakdown_ui(ui, &entry.channels);
}

/// The recording's ad-hoc sensor channels, one row each: name (with a vector
/// channel's component labels), unit, and sample count. Long channel lists are
/// truncated so the hover cannot outgrow the screen.
///
/// A recording with no channels says so rather than rendering nothing - the
/// absence is the answer to "what custom data is in here".
fn channels_breakdown_ui(ui: &mut egui::Ui, channels: &[ChannelSummary]) {
    ui.add_space(4.0);
    if channels.is_empty() {
        ui.label(RichText::new("No custom channels").color(ui.visuals().weak_text_color()));
        return;
    }

    let count = channels.len();
    ui.label(
        RichText::new(format!(
            "{count} custom {}",
            gt_fmt::pluralize(count, "channel", "channels")
        ))
        .strong(),
    );
    Grid::new("history_breakdown_channels")
        .num_columns(3)
        .spacing([12.0, 4.0])
        .show(ui, |ui| {
            for channel in channels.iter().take(MAX_HOVER_CHANNELS) {
                ui.vertical(|ui| {
                    // A vertical inside a grid cell has no width of its own to
                    // wrap against, so let the labels size the column instead.
                    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
                    ui.label(channel_title(channel));
                    // The producer's own words for what the channel measures,
                    // tucked under the name so the columns stay aligned.
                    if let Some(description) = &channel.description {
                        ui.label(
                            RichText::new(description)
                                .small()
                                .color(ui.visuals().weak_text_color()),
                        );
                    }
                });
                ui.label(
                    RichText::new(channel.unit.as_deref().unwrap_or(EM_DASH))
                        .color(ui.visuals().weak_text_color()),
                );
                ui.label(
                    RichText::new(format!(
                        "{} {}",
                        format_stored_count(channel.sample_count),
                        gt_fmt::pluralize(
                            usize::try_from(channel.sample_count).unwrap_or(usize::MAX),
                            "sample",
                            "samples",
                        )
                    ))
                    .color(ui.visuals().weak_text_color()),
                );
                ui.end_row();
            }
        });
    if let Some(hidden) = count.checked_sub(MAX_HOVER_CHANNELS).filter(|n| *n > 0) {
        ui.label(RichText::new(format!("and {hidden} more")).color(ui.visuals().weak_text_color()));
    }
}

/// How many channels the hover lists before summarizing the rest, so a
/// recording carrying dozens of them still produces a readable tooltip.
const MAX_HOVER_CHANNELS: usize = 8;

/// A channel's name, with a vector channel's component labels appended:
/// `accel (x, y, z)`. A scalar channel is just its name.
fn channel_title(channel: &ChannelSummary) -> String {
    if channel.components.is_empty() {
        return channel.name.clone();
    }
    format!("{} ({})", channel.name, channel.components.join(", "))
}

/// The recording's track count, noting how many of them are hidden.
fn track_count_text(entry: &RecordingEntry) -> String {
    if entry.hidden_tracks > 0 {
        format!("{} ({} hidden)", entry.total_tracks, entry.hidden_tracks)
    } else {
        entry.total_tracks.to_string()
    }
}

/// Thousands-separated form of one of the database's `u64` counters.
fn format_stored_count(n: u64) -> String {
    gt_fmt::format_count(usize::try_from(n).unwrap_or(usize::MAX))
}

/// Render the inline identity-rename editor in the identity column. Commits on
/// Enter, cancels on focus loss (click-away or Escape); either way the editor
/// closes. A no-op commit (empty, or unchanged from the displayed name) does not
/// send a rename. `rename` is guaranteed `Some` by the caller.
fn render_rename_editor(
    ui: &mut egui::Ui,
    rename: &mut Option<RenameEdit>,
    worker: &HistoryWorker,
) {
    let Some(edit) = rename.as_mut() else {
        return;
    };
    let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
    let resp = ui.add(
        TextEdit::singleline(&mut edit.buffer)
            .desired_width(f32::INFINITY)
            .hint_text("Identity"),
    );
    if resp.lost_focus() {
        let old = std::mem::take(&mut edit.identity);
        let new = edit.buffer.trim().to_owned();
        let unchanged = new == identity_display_parts(&old).0;
        *rename = None;
        if enter && !new.is_empty() && !unchanged {
            worker.rename_identity(old, new);
        }
    } else {
        // Keep focus in the freshly-opened editor until the user commits or
        // clicks away.
        resp.request_focus();
    }
}

/// Open the inline rename editor for a recording's identity.
fn begin_rename(rename: &mut Option<RenameEdit>, entry: &RecordingEntry) {
    let identity = entry.db_ref.identity.clone();
    let buffer = identity_display_parts(&identity).0.to_owned();
    *rename = Some(RenameEdit { identity, buffer });
}

/// The identity column of a History row. Double-clicking the cell opens the
/// inline rename editor; right-clicking offers Rename and Delete.
fn identity_cell(
    ui: &mut egui::Ui,
    entry: &RecordingEntry,
    worker: &HistoryWorker,
    rename: &mut Option<RenameEdit>,
) {
    let identity = entry.db_ref.identity.as_str();
    let (display_name, is_auto) = identity_display_parts(identity);
    // The full identity is the hover's first line, so leave it out of the view:
    // the note icon and rows are for the SDK's title/device/notes only. Every
    // recording has an identity, so including it would badge every row.
    let travel_mode = entry.travel_mode.as_deref().map(travel_mode_display);
    let meta = MetadataView {
        title: entry.title.as_deref(),
        device: entry.device.as_deref(),
        travel_mode: travel_mode.as_deref(),
        identity: None,
        notes: entry.notes.as_deref(),
    };
    let has_metadata = has_metadata_details(&meta);
    let label = ui
        .horizontal(|ui| {
            if is_auto {
                ui.label(
                    RichText::new("auto")
                        .small()
                        .color(ui.visuals().weak_text_color()),
                );
            }
            if has_metadata {
                ui.label(RichText::new(ICON_NOTE).weak());
            }
            // The label itself senses clicks: it is the rename target. Text
            // selection stays off so a double click opens the editor rather
            // than selecting a word under the pointer. The cell's hover leads
            // with the full identity, so egui's elided-text tooltip would open
            // a second tooltip saying the same thing.
            ui.add(
                Label::new(display_name)
                    .truncate()
                    .selectable(false)
                    .show_tooltip_when_elided(false)
                    .sense(egui::Sense::click()),
            )
        })
        .inner
        // Double-click renames and right-click offers the menu, so the cell
        // reads as interactive rather than as editable text.
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_ui(|ui| {
            ui.label(identity);
            metadata_detail_rows(ui, &meta);
            // The same breakdown the value cells show, so the hover is the
            // whole row's story wherever the pointer lands.
            ui.separator();
            data_breakdown_ui(ui, entry);
            ui.separator();
            ui.label(
                RichText::new("Double-click to rename")
                    .small()
                    .color(ui.visuals().weak_text_color()),
            );
        });
    if label.double_clicked() {
        begin_rename(rename, entry);
    }
    label.context_menu(|ui| {
        if ui.button("Rename").clicked() {
            begin_rename(rename, entry);
            ui.close();
        }
        if ui.button("Delete").clicked() {
            worker.delete_recordings(vec![entry.db_ref.clone()], DeleteReason::Manual);
            ui.close();
        }
    });
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
mod tests {
    use egui_kittest::kittest::Queryable as _;
    use gt_store::HistoryDatabase as _;
    use gt_test_utils::TestHarness;

    use crate::app::history_db::Response;

    use super::{
        ChannelSummary, DatabaseRef, HistorySort, HistoryWindow, HistoryWorker, ICON_CARET_DOWN,
        ICON_CARET_UP, ICON_NOTE, MAX_HOVER_CHANNELS, RecordingEntry, RecordingMeta, SortColumn,
        SortDirection, breakdown_cell_id, channel_title, data_breakdown_ui, identity_display_parts,
        track_count_text, travel_mode_display,
    };
    use strum::{EnumCount as _, IntoEnumIterator as _};

    /// Harness state for driving the History window: the window, a live (empty)
    /// worker so the list branch renders, and the settings toggles `show` needs.
    struct HistoryHarness {
        window: HistoryWindow,
        worker: HistoryWorker,
        storage_enabled: bool,
        auto_prune_enabled: bool,
        auto_prune_max_bytes: u64,
        auto_prune_confirm: bool,
        _dir: tempfile::TempDir,
    }

    fn history_harness(entries: Vec<RecordingEntry>) -> HistoryHarness {
        let dir = tempfile::tempdir().expect("temp dir");
        let db =
            gt_store::Recordings::open_or_create(&dir.path().join("history.h5")).expect("open db");
        let worker = HistoryWorker::spawn(db, egui::Context::default());
        let mut window = HistoryWindow::new();
        window.open = true;
        // Populate directly so the list renders without a worker round-trip.
        window.set_entries(entries);
        HistoryHarness {
            window,
            worker,
            storage_enabled: true,
            auto_prune_enabled: false,
            auto_prune_max_bytes: 0,
            auto_prune_confirm: true,
            _dir: dir,
        }
    }

    fn show_history(ui: &mut egui::Ui, s: &mut HistoryHarness) {
        s.window.show(
            ui.ctx(),
            &s.worker,
            &[],
            &mut s.storage_enabled,
            &mut s.auto_prune_enabled,
            &mut s.auto_prune_max_bytes,
            &mut s.auto_prune_confirm,
        );
    }

    /// A harness backed by a real database holding one recording, with no
    /// pre-seeded entries - the list arrives from the worker (see [`pump_history`]).
    fn history_harness_with_recording(identity: &str) -> HistoryHarness {
        use gt_store::{StoredSegmentation, TrackRange};

        let dir = tempfile::tempdir().expect("temp dir");
        let mut db =
            gt_store::Recordings::open_or_create(&dir.path().join("history.h5")).expect("open db");
        let bytes = gt_test_utils::GOLD_BYTES;
        let meta = gt_store::extract_meta(bytes).expect("meta");
        let tracks = [TrackRange {
            start: 0,
            end: meta.nav_point_count,
            hidden: false,
        }];
        let settings = StoredSegmentation {
            track_split_gap_us: 300_000_000,
            detect_clock_discontinuities: true,
            clock_discontinuity_sigmas: 5.0,
        };
        db.insert(identity, &meta, &tracks, settings, bytes)
            .expect("insert recording");
        let worker = HistoryWorker::spawn(db, egui::Context::default());
        let mut window = HistoryWindow::new();
        window.open = true;
        HistoryHarness {
            window,
            worker,
            storage_enabled: true,
            auto_prune_enabled: false,
            auto_prune_max_bytes: 0,
            auto_prune_confirm: true,
            _dir: dir,
        }
    }

    /// Drive one frame like the app does: drain the worker's responses into the
    /// window (list refresh, mutation acknowledgements) and then render it.
    fn pump_history(ui: &mut egui::Ui, s: &mut HistoryHarness) {
        for resp in s.worker.poll() {
            match resp {
                Response::Listed(Ok(entries)) => s.window.set_entries(entries),
                Response::Mutated { result: Ok(()), .. } => s.window.invalidate(),
                _ => {}
            }
        }
        show_history(ui, s);
    }

    /// Run frames (yielding to the worker thread) until `pred` holds or the
    /// budget is exhausted; returns whether it held.
    fn run_until(
        h: &mut TestHarness<HistoryHarness>,
        pred: impl Fn(&mut TestHarness<HistoryHarness>) -> bool,
    ) -> bool {
        for _ in 0..100 {
            // Single-frame `step` (not `run`): the History window paints a spinner
            // while a list request is in flight, so it never reaches a quiescent
            // state for `run` to converge on.
            h.step();
            if pred(h) {
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(3));
        }
        false
    }

    #[test]
    fn rename_workflow_updates_the_listed_identity_end_to_end() {
        // Full workflow against a real worker + database: the row lists, the user
        // edits the identity inline, and after the async rename the list shows the
        // new name.
        let harness = history_harness_with_recording("auto:ride.gtd");
        let mut h = TestHarness::builder()
            .size(egui::vec2(900.0, 500.0))
            .ui_state(pump_history, harness);

        // The recording lists under its stripped identity.
        assert!(
            run_until(&mut h, |h| h
                .inner
                .query_by_label_contains("ride.gtd")
                .is_some()),
            "recording should appear in the History list"
        );

        // Open the inline editor through the identity's context menu.
        // `request_focus` applies the frame after the editor first renders,
        // so settle a couple of frames before typing.
        h.inner.get_by_label_contains("ride.gtd").click_secondary();
        h.step();
        h.inner.get_by_label("Rename").click_accesskit();
        h.step();
        h.step();
        assert!(
            h.inner.query_all_by_value("ride.gtd").next().is_some(),
            "probe: editor not open after Rename click"
        );

        // Append to the seeded name and commit with Enter.
        h.inner.event(egui::Event::Text(" v2".to_owned()));
        h.step();
        h.inner.key_press(egui::Key::Enter);
        h.step();

        // After the worker renames and the window re-lists, the new identity shows.
        assert!(
            run_until(&mut h, |h| h
                .inner
                .query_by_label_contains("ride.gtd v2")
                .is_some()),
            "the renamed identity should appear in the refreshed list"
        );
    }

    /// The recordings table: identity takes the remaining width (long names
    /// get the room), the value columns stay compact, headers carry the
    /// resize handles.
    #[test]
    fn snapshot_history_window_table() {
        let mut harness = history_harness(vec![
            entry_with_identity("auto:ride.gtd"),
            entry_with_identity("a much longer recording identity that needs the room"),
            entry_with_identity("survey_flight_2026_07_15.gtd"),
        ]);
        // The temporary database path differs every run; keep it out of the
        // image.
        harness.worker.hide_path();
        let mut h = TestHarness::builder()
            .size(egui::vec2(900.0, 500.0))
            .ui_state(show_history, harness);
        // Auto columns measure their content over the first frames; settle
        // before snapshotting.
        for _ in 0..4 {
            h.run();
        }
        h.snapshot("history_window_table");
    }

    #[test]
    fn double_clicking_identity_opens_inline_editor() {
        let harness = history_harness(vec![entry_with_identity("auto:ride.gtd")]);
        // Frames at 60 fps: kittest's default 0.25 s/frame clock (one frame
        // per queued event) spaces the two clicks beyond egui's 0.3 s
        // double-click window.
        let mut h = TestHarness::builder()
            .size(egui::vec2(900.0, 500.0))
            .step_dt(1.0 / 60.0)
            .ui_state(show_history, harness);
        h.run();
        // Two quick clicks on the identity label register as a double click
        // and swap the cell for the inline text editor (seeded with the
        // `auto:`-stripped name).
        h.inner.get_by_label_contains("ride.gtd").click();
        h.inner.get_by_label_contains("ride.gtd").click();
        h.run();
        assert!(
            h.inner.query_all_by_value("ride.gtd").next().is_some(),
            "inline editor should show the stripped identity as its value"
        );
        // The editor holds keyboard focus: typing extends its buffer.
        h.step();
        h.inner.event(egui::Event::Text(" v2".to_owned()));
        h.step();
        h.step();
        assert!(
            h.inner.query_all_by_value("ride.gtd v2").next().is_some(),
            "typed text should reach the freshly opened editor"
        );
    }

    /// A listing entry for `identity` with no tracks and no SDK metadata, for the
    /// identity-cell layout tests.
    fn entry_with_identity(identity: &str) -> RecordingEntry {
        RecordingEntry {
            db_ref: DatabaseRef {
                identity: identity.to_owned(),
                group_name: "rec0".to_owned(),
            },
            meta: RecordingMeta {
                start_us: 0,
                end_us: 0,
                nav_point_count: 0,
                sat_report_count: 0,
                marker_count: 0,
                event_marker_count: 0,
                gtd_size_bytes: 0,
            },
            total_tracks: 0,
            hidden_tracks: 0,
            title: None,
            device: None,
            notes: None,
            travel_mode: None,
            channels: Vec::new(),
        }
    }

    /// A listing entry with the four sortable value columns set, for the
    /// ordering tests. `duration_us` is added to `start_us` to give the entry
    /// its span.
    fn sortable_entry(
        identity: &str,
        start_us: i64,
        duration_us: i64,
        nav_point_count: u64,
        gtd_size_bytes: u64,
    ) -> RecordingEntry {
        let mut entry = entry_with_identity(identity);
        entry.meta.start_us = start_us;
        entry.meta.end_us = start_us + duration_us;
        entry.meta.nav_point_count = nav_point_count;
        entry.meta.gtd_size_bytes = gtd_size_bytes;
        entry
    }

    /// Three entries whose columns disagree about the order, so sorting by any
    /// one of them produces a different sequence: `beta` is the oldest but the
    /// longest and biggest, `alpha` the newest but the shortest.
    fn sortable_entries() -> Vec<RecordingEntry> {
        vec![
            sortable_entry("Alpha", 3_000, 10, 5, 50),
            sortable_entry("beta", 1_000, 300, 100, 5_000),
            sortable_entry("Gamma", 2_000, 60, 40, 400),
        ]
    }

    /// The identities the sort produces, in list order.
    fn sorted_identities(sort: HistorySort, entries: &[RecordingEntry]) -> Vec<&str> {
        let mut visible: Vec<&RecordingEntry> = entries.iter().collect();
        sort.apply(&mut visible);
        visible.iter().map(|e| e.db_ref.identity.as_str()).collect()
    }

    /// Every column orders the list by its own value, in both directions.
    /// Identity compares case-insensitively on the displayed name, so `beta`
    /// sorts between `Alpha` and `Gamma` rather than after both.
    #[rstest::rstest]
    #[case(SortColumn::Identity, SortDirection::Ascending, ["Alpha", "beta", "Gamma"])]
    #[case(SortColumn::Identity, SortDirection::Descending, ["Gamma", "beta", "Alpha"])]
    #[case(SortColumn::Date, SortDirection::Ascending, ["beta", "Gamma", "Alpha"])]
    #[case(SortColumn::Date, SortDirection::Descending, ["Alpha", "Gamma", "beta"])]
    #[case(SortColumn::Duration, SortDirection::Ascending, ["Alpha", "Gamma", "beta"])]
    #[case(SortColumn::Duration, SortDirection::Descending, ["beta", "Gamma", "Alpha"])]
    #[case(SortColumn::Points, SortDirection::Ascending, ["Alpha", "Gamma", "beta"])]
    #[case(SortColumn::Points, SortDirection::Descending, ["beta", "Gamma", "Alpha"])]
    #[case(SortColumn::Size, SortDirection::Ascending, ["Alpha", "Gamma", "beta"])]
    #[case(SortColumn::Size, SortDirection::Descending, ["beta", "Gamma", "Alpha"])]
    fn sorting_orders_by_the_chosen_column(
        #[case] column: SortColumn,
        #[case] direction: SortDirection,
        #[case] expected: [&str; 3],
    ) {
        let entries = sortable_entries();
        let sort = HistorySort { column, direction };

        assert_eq!(sorted_identities(sort, &entries), expected.to_vec());
    }

    /// Entries that tie on the sorted column keep one stable order regardless of
    /// direction, so equal rows do not shuffle when the sort is reversed.
    #[test]
    fn ties_break_stably_and_independently_of_direction() {
        // Same size, different identities - only the tie-break can separate them.
        let entries = vec![
            sortable_entry("charlie", 3_000, 10, 5, 100),
            sortable_entry("alpha", 1_000, 20, 9, 100),
            sortable_entry("bravo", 2_000, 30, 7, 100),
        ];
        let by_size = |direction| {
            sorted_identities(
                HistorySort {
                    column: SortColumn::Size,
                    direction,
                },
                &entries,
            )
        };

        assert_eq!(
            by_size(SortDirection::Ascending),
            ["alpha", "bravo", "charlie"]
        );
        assert_eq!(
            by_size(SortDirection::Descending),
            ["alpha", "bravo", "charlie"],
            "reversing the direction must not reshuffle rows that tie on the column",
        );
    }

    /// Clicking the active column reverses it; clicking another switches to it
    /// in that column's own natural direction rather than inheriting the
    /// previous one.
    #[test]
    fn header_clicks_reverse_then_switch_columns() {
        let mut sort = HistorySort::default();
        assert_eq!(sort.column, SortColumn::Date);
        assert_eq!(sort.direction, SortDirection::Descending);

        sort.clicked(SortColumn::Date);
        assert_eq!(
            sort.direction,
            SortDirection::Ascending,
            "re-click reverses"
        );

        sort.clicked(SortColumn::Identity);
        assert_eq!(
            (sort.column, sort.direction),
            (SortColumn::Identity, SortDirection::Ascending),
            "identity starts A to Z",
        );

        sort.clicked(SortColumn::Size);
        assert_eq!(
            (sort.column, sort.direction),
            (SortColumn::Size, SortDirection::Descending),
            "size starts largest first, not carrying identity's ascending order",
        );
    }

    /// Every sortable column carries its own header title and a distinct hint
    /// per direction, so no variant can be added without describing itself.
    #[test]
    fn every_sort_column_describes_itself() {
        let columns: Vec<SortColumn> = SortColumn::iter().collect();
        assert_eq!(
            columns.len(),
            SortColumn::COUNT,
            "the iterator must cover every variant",
        );

        let mut titles: Vec<&str> = columns.iter().map(|c| c.title()).collect();
        titles.sort_unstable();
        titles.dedup();
        assert_eq!(
            titles.len(),
            SortColumn::COUNT,
            "column titles must be unique"
        );

        for column in columns {
            assert_ne!(
                column.order_hint(SortDirection::Ascending),
                column.order_hint(SortDirection::Descending),
                "{column:?} must read differently in each direction",
            );
        }
    }

    /// The DB hands the listing the raw `meta_travel_mode` wire value; the
    /// hover must show the human spelling for known modes and the preserved
    /// wire value verbatim for unknown ones.
    #[rstest::rstest]
    #[case("bicycle", "Bicycle")]
    #[case("hovercraft", "hovercraft")]
    fn travel_mode_display_humanizes_the_wire_value(#[case] wire: &str, #[case] expected: &str) {
        assert_eq!(travel_mode_display(wire), expected);
    }

    /// A travel mode alone must badge the row with the note icon, proving
    /// `identity_cell` feeds the field into the shared metadata presence check.
    #[test]
    fn travel_mode_alone_shows_the_metadata_note_icon() {
        let mut entry = entry_with_identity("auto:ride.gtd");
        entry.travel_mode = Some("bicycle".to_owned());
        let harness = history_harness(vec![entry]);
        let mut h = TestHarness::builder()
            .size(egui::vec2(900.0, 500.0))
            .ui_state(show_history, harness);
        h.run();
        assert!(
            h.inner.query_by_label(ICON_NOTE).is_some(),
            "the note icon should appear for an entry whose only metadata is a travel mode"
        );
    }

    /// Settled width of the History window, through the real rendering path
    /// ([`HistoryWindow::show`]). A resizable window runs a sizing pass over
    /// its content, the path where an un-clipped column would report its
    /// full text width and stretch the window.
    fn history_window_width(identity: &str) -> f32 {
        let harness = history_harness(vec![entry_with_identity(identity)]);
        let mut h = TestHarness::builder()
            .size(egui::vec2(1600.0, 500.0))
            .ui_state(show_history, harness);
        for _ in 0..6 {
            h.run();
        }
        let window = h
            .inner
            .get_by_role_and_label(egui::accesskit::Role::Window, "History");
        window.rect().width()
    }

    /// A long recording identity truncates in the History window rather than
    /// stretching it: a short, a long, and a much longer identity all settle the
    /// resizable window at the same width. Without the truncation the identity
    /// column would size to its full text and the window would grow with it.
    #[test]
    fn long_identity_does_not_widen_history_window() {
        let short = history_window_width("auto:ride.gtd");
        let long = history_window_width(&"a/very/long/recording/identity/".repeat(4));
        let longer = history_window_width(&"a/very/long/recording/identity/".repeat(12));
        assert!(
            (long - short).abs() < 1.0 && (longer - short).abs() < 1.0,
            "identity length changed the history window width: \
             short={short}px long={long}px longer={longer}px",
        );
    }

    /// The metadata-width measurement is ignored during the table's sizing pass:
    /// on the first frame the auto columns have not grown to their content, so
    /// the reserve reads far too small and, if cached, would inflate identity and
    /// stick the window permanently wide. A freshly opened window must therefore
    /// settle to its content width, not a bloated one.
    #[test]
    fn fresh_window_settles_to_content_width_not_a_bloated_one() {
        // Room to bloat into: the screen is 1600px, the content needs well under
        // half that. A leaked sizing-pass measurement pushed this past 900px.
        let width = history_window_width("auto:ride.gtd");
        assert!(
            width < 750.0,
            "the History window settled far wider than its content ({width:.0}px); \
             the sizing-pass metadata measurement likely leaked into the identity fill",
        );
    }

    /// The identity filter field fills the toolbar space to the left of the
    /// action controls and must yield as the window narrows, never growing into
    /// them. Previously the field kept a fixed width and the "Auto-store
    /// recordings" checkbox slid left underneath it, overlapping.
    #[test]
    fn filter_field_does_not_overlap_the_toolbar_controls() {
        let harness = history_harness(vec![entry_with_identity("auto:ride.gtd")]);
        let mut h = TestHarness::builder()
            .size(egui::vec2(1200.0, 500.0))
            .ui_state(show_history, harness);
        for _ in 0..8 {
            h.step();
        }
        // Shrink toward the window's minimum, where the overlap used to appear.
        let w = window_rect(&h);
        drag(
            &mut h,
            egui::pos2(w.right() - 1.0, w.bottom() - 1.0),
            egui::vec2(-500.0, 0.0),
            10,
        );
        for _ in 0..3 {
            h.step();
        }

        let checkbox_left = h.inner.get_by_label("Auto-store recordings").rect().left();
        // The first text input in the window is the identity filter field.
        let filter_right = h
            .inner
            .get_all_by_role(egui::accesskit::Role::TextInput)
            .map(|n| n.rect())
            .next()
            .expect("identity filter field")
            .right();
        assert!(
            filter_right <= checkbox_left + 1.0,
            "the identity filter field (right edge {filter_right:.0}px) overlaps the \
             Auto-store checkbox (left edge {checkbox_left:.0}px)",
        );
    }

    /// A History window sized to a wide screen, populated with long identities
    /// (they clip in the identity column), settled so the auto columns have
    /// measured their content.
    fn resize_harness() -> TestHarness<'static, HistoryHarness> {
        let long = "a/very/long/recording/identity/that/needs/lots/of/room/".repeat(2);
        let harness = history_harness(vec![
            entry_with_identity(&long),
            entry_with_identity(&format!("{long}/2")),
            entry_with_identity(&format!("{long}/3")),
        ]);
        let mut h = TestHarness::builder()
            .size(egui::vec2(1400.0, 600.0))
            .ui_state(show_history, harness);
        // Settle the sizing pass and let the window finish auto-positioning.
        for _ in 0..10 {
            h.step();
        }
        h
    }

    /// The rightmost content (the Delete button) relative to the window's right
    /// edge. Identity fills the leftover width, so this "gap" is only the
    /// window's frame padding - at every window size.
    fn content_gap_to_window_edge(h: &TestHarness<HistoryHarness>) -> f32 {
        let win = window_rect(h);
        let delete = h
            .inner
            .get_all_by_label("Delete")
            .last()
            .expect("delete button")
            .rect();
        win.right() - delete.right()
    }

    /// Identity fills the window at every size: the metadata columns keep their
    /// content width and identity takes the rest. Growing or shrinking the
    /// window leaves no gap on the right and traps no content off-screen - the
    /// table is always exactly as wide as the window.
    #[test]
    fn identity_fills_the_window_at_every_size() {
        let mut h = resize_harness();
        let settled_gap = content_gap_to_window_edge(&h);

        // Grow the window from its bottom-right corner.
        let before = window_rect(&h);
        drag(
            &mut h,
            egui::pos2(before.right() - 1.0, before.bottom() - 1.0),
            egui::vec2(300.0, 0.0),
            8,
        );
        for _ in 0..3 {
            h.step();
        }
        assert!(
            window_rect(&h).width() > before.width() + 200.0,
            "the window did not grow: {:.0}px -> {:.0}px",
            before.width(),
            window_rect(&h).width(),
        );
        assert!(
            (content_gap_to_window_edge(&h) - settled_gap).abs() < 4.0,
            "growing the window left a gap on the right - identity did not fill it",
        );

        // Shrink it back down. egui clamps the drag at the content's minimum
        // width (measured by a sizing pass when the drag starts), so stay well
        // above that floor: the identity-fill invariant is what matters here.
        let grown = window_rect(&h);
        drag(
            &mut h,
            egui::pos2(grown.right() - 1.0, grown.bottom() - 1.0),
            egui::vec2(-80.0, 0.0),
            8,
        );
        for _ in 0..3 {
            h.step();
        }
        assert!(
            window_rect(&h).width() < grown.width() - 40.0,
            "the window did not shrink: {:.0}px -> {:.0}px",
            grown.width(),
            window_rect(&h).width(),
        );
        assert!(
            (content_gap_to_window_edge(&h) - settled_gap).abs() < 4.0,
            "shrinking the window left a gap on the right - identity did not fill it",
        );
    }

    fn window_rect(h: &TestHarness<HistoryHarness>) -> egui::Rect {
        h.inner
            .get_by_role_and_label(egui::accesskit::Role::Window, "History")
            .rect()
    }

    /// Press-drag-release the pointer from `from` by `delta` over `steps` frames.
    fn drag(h: &mut TestHarness<HistoryHarness>, from: egui::Pos2, delta: egui::Vec2, steps: u32) {
        h.inner.event(egui::Event::PointerMoved(from));
        h.step();
        h.inner.event(egui::Event::PointerButton {
            pos: from,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        });
        h.step();
        for i in 1..=steps {
            h.inner.event(egui::Event::PointerMoved(
                from + delta * (i as f32 / steps as f32),
            ));
            h.step();
        }
        h.inner.event(egui::Event::PointerButton {
            pos: from + delta,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        });
        h.step();
    }

    /// The window can be dragged narrower than its settled width. Identity
    /// yields as the window shrinks, so the table follows the window down
    /// instead of pinning it at a content minimum that snaps it back to full
    /// width (the old "can't shrink the window" bug).
    #[test]
    fn the_window_can_be_shrunk_narrower() {
        let mut h = resize_harness();
        let before = window_rect(&h);
        // Drag the bottom-right resize corner inward.
        let corner = egui::pos2(before.right() - 1.0, before.bottom() - 1.0);
        drag(&mut h, corner, egui::vec2(-200.0, 0.0), 8);
        for _ in 0..3 {
            h.step();
        }
        let after = window_rect(&h);
        assert!(
            after.width() < before.width() - 50.0,
            "the window did not shrink: {:.1}px -> {:.1}px",
            before.width(),
            after.width(),
        );
    }

    /// The identities the table currently lists, top to bottom, read off the
    /// rendered row positions.
    fn listed_order(h: &TestHarness<HistoryHarness>, identities: &[&str]) -> Vec<String> {
        let mut rows: Vec<(f32, String)> = identities
            .iter()
            .map(|identity| {
                let top = h.inner.get_by_label_contains(identity).rect().top();
                (top, (*identity).to_owned())
            })
            .collect();
        rows.sort_by(|a, b| a.0.total_cmp(&b.0));
        rows.into_iter().map(|(_, identity)| identity).collect()
    }

    /// Click the table's header for `title`. The toolbar carries an "Identity"
    /// label of its own, so match on the lowest node on screen - the header row
    /// sits below the toolbar.
    fn click_header(h: &TestHarness<HistoryHarness>, title: &str) {
        header_node(h, title).click();
    }

    /// The table header labelled exactly `title`.
    ///
    /// Takes the lowest matching node: the toolbar and filter row carry labels
    /// with the same words ("Identity", "Points") and sit above the table.
    fn header_node<'t>(
        h: &'t TestHarness<HistoryHarness>,
        title: &'t str,
    ) -> egui_kittest::Node<'t> {
        h.inner
            .get_all_by_label(title)
            .max_by(|a, b| a.rect().top().total_cmp(&b.rect().top()))
            .expect("column header")
    }

    /// Clicking a column header reorders the rendered table, and clicking the
    /// same header again reverses it - the sort reaching the actual list, not
    /// just the state struct.
    #[test]
    fn clicking_a_header_reorders_the_rendered_rows() {
        let harness = history_harness(sortable_entries());
        let mut h = TestHarness::builder()
            .size(egui::vec2(900.0, 500.0))
            .ui_state(show_history, harness);
        for _ in 0..4 {
            h.run();
        }

        let identities = ["Alpha", "beta", "Gamma"];
        assert_eq!(
            listed_order(&h, &identities),
            ["Alpha", "Gamma", "beta"],
            "the default order is newest first",
        );

        // Sort by identity: a first click on a new column sorts it A to Z.
        click_header(&h, "Identity");
        h.run();
        assert_eq!(listed_order(&h, &identities), ["Alpha", "beta", "Gamma"]);

        // Clicking the active column reverses it.
        click_header(&h, "Identity");
        h.run();
        assert_eq!(listed_order(&h, &identities), ["Gamma", "beta", "Alpha"]);

        // Switching to Points sorts largest first.
        click_header(&h, "Points");
        h.run();
        assert_eq!(listed_order(&h, &identities), ["beta", "Gamma", "Alpha"]);
    }

    /// The active column is the only one showing a caret, and the caret follows
    /// the direction - so the header always says how the list is ordered.
    #[test]
    fn only_the_active_column_shows_a_direction_caret() {
        let harness = history_harness(sortable_entries());
        let mut h = TestHarness::builder()
            .size(egui::vec2(900.0, 500.0))
            .ui_state(show_history, harness);
        for _ in 0..4 {
            h.run();
        }

        // Default sort is Date descending: exactly one caret, pointing down.
        assert_eq!(h.inner.query_all_by_label(ICON_CARET_DOWN).count(), 1);
        assert_eq!(h.inner.query_all_by_label(ICON_CARET_UP).count(), 0);

        // Reversing it flips the caret without adding a second one.
        click_header(&h, "Date");
        h.run();
        assert_eq!(h.inner.query_all_by_label(ICON_CARET_DOWN).count(), 0);
        assert_eq!(h.inner.query_all_by_label(ICON_CARET_UP).count(), 1);
    }

    /// A recording carrying ad-hoc sensor channels: two of them, one vector and
    /// one scalar, plus counts for every data kind the breakdown reports.
    fn entry_with_channels() -> RecordingEntry {
        let mut entry = sortable_entry(
            "auto:sensors.gtd",
            1_700_000_000_000_000,
            3_600_000_000,
            8_940,
            4_096,
        );
        entry.meta.sat_report_count = 1_234;
        entry.meta.marker_count = 12;
        entry.meta.event_marker_count = 3;
        entry.total_tracks = 4;
        entry.channels = vec![
            ChannelSummary {
                name: "accel".to_owned(),
                unit: Some("g".to_owned()),
                description: Some("Frame IMU".to_owned()),
                components: vec!["x".to_owned(), "y".to_owned(), "z".to_owned()],
                sample_count: 12_000,
            },
            ChannelSummary {
                name: "temperature".to_owned(),
                unit: None,
                description: None,
                components: Vec::new(),
                sample_count: 512,
            },
        ];
        entry
    }

    /// Park the pointer on the widget labelled `label` and hold it there until
    /// the hover turns into a tooltip.
    ///
    /// egui only shows a tooltip once the pointer has been *still* on the
    /// widget for `tooltip_delay`, and every `PointerMoved` restarts that
    /// timer - so the position is sent once and the following frames are run
    /// without further events. Re-sending it each frame keeps the pointer
    /// permanently "moving" and no tooltip ever appears.
    ///
    /// Matches the topmost node, which is the table row rather than the footer
    /// summary when both carry the same text (a size, say).
    fn hover_widget(h: &mut TestHarness<HistoryHarness>, label: &str) {
        let target = topmost_labelled(h, label);
        hover_pos(h, target);
    }

    /// Hold the pointer at `target` until the hover settles.
    ///
    /// One frame registers the move, then the rest run without events so
    /// egui's "pointer has been still" timer can actually accumulate.
    fn hover_pos(h: &mut TestHarness<HistoryHarness>, target: egui::Pos2) {
        h.inner.event(egui::Event::PointerMoved(target));
        for _ in 0..4 {
            h.step();
        }
    }

    /// Point at the widget labelled `label` and stop before its tooltip opens.
    ///
    /// For asking what cursor a widget wants. A tooltip is its own layer and a
    /// big one lands over the pointer, which takes the hover off the widget
    /// underneath and resets the cursor - so the cursor has to be read while
    /// the widget is still the thing being pointed at.
    fn point_at_widget(h: &mut TestHarness<HistoryHarness>, label: &str) {
        let target = topmost_labelled(h, label);
        h.inner.event(egui::Event::PointerMoved(target));
        h.step();
    }

    /// Like [`point_at_widget`] for a table header (see [`header_node`]).
    fn point_at_header(h: &mut TestHarness<HistoryHarness>, title: &str) {
        let target = header_node(h, title).rect().center();
        h.inner.event(egui::Event::PointerMoved(target));
        h.step();
    }

    /// Centre of the topmost widget whose label contains `label` - the table
    /// row rather than the footer summary when both carry the same text.
    fn topmost_labelled(h: &TestHarness<HistoryHarness>, label: &str) -> egui::Pos2 {
        h.inner
            .get_all_by_label_contains(label)
            .min_by(|a, b| a.rect().top().total_cmp(&b.rect().top()))
            .expect("hover target")
            .rect()
            .center()
    }

    /// Snapshot the hover breakdown for `entry`, rendered through the same
    /// function the tooltip calls.
    ///
    /// Driven directly rather than through a hover so the image is just the
    /// breakdown: what it covers is everything the breakdown itself decides -
    /// which rows appear, how the channels lay out, and where it truncates.
    /// That the hover actually reaches it is covered separately, by the tests
    /// that hover a real row.
    fn snapshot_breakdown(entry: &RecordingEntry, name: &str) {
        let mut h = TestHarness::builder()
            .size(egui::vec2(420.0, 560.0))
            .ui(|ui| data_breakdown_ui(ui, entry));
        for _ in 0..3 {
            h.run();
        }
        h.snapshot(name);
    }

    /// The breakdown of a recording carrying ad-hoc sensor channels: its span,
    /// its shape on disk, a count per kind of data, and the channels - vector
    /// components, units, and sample counts included.
    #[test]
    fn snapshot_history_row_breakdown() {
        snapshot_breakdown(&entry_with_channels(), "history_row_breakdown");
    }

    /// A recording with no channels states that in its breakdown rather than
    /// leaving the question unanswered by rendering nothing. Its hidden tracks
    /// also earn the note explaining where they came from.
    #[test]
    fn snapshot_history_row_breakdown_without_channels() {
        let mut entry = sortable_entry(
            "auto:plain.gtd",
            1_700_000_000_000_000,
            900_000_000,
            42,
            4_096,
        );
        entry.total_tracks = 3;
        entry.hidden_tracks = 1;
        snapshot_breakdown(&entry, "history_row_breakdown_no_channels");
    }

    /// A recording with more channels than the hover lists shows the first
    /// [`MAX_HOVER_CHANNELS`] and counts the rest, so the tooltip cannot grow
    /// past the screen.
    #[test]
    fn snapshot_history_row_breakdown_truncates_long_channel_list() {
        let mut entry = entry_with_channels();
        entry.channels = (0..MAX_HOVER_CHANNELS + 3)
            .map(|i| ChannelSummary {
                // Zero-padded so the name order matches the numeric order.
                name: format!("channel_{i:02}"),
                unit: None,
                description: None,
                components: Vec::new(),
                sample_count: 10,
            })
            .collect();
        snapshot_breakdown(&entry, "history_row_breakdown_many_channels");
    }

    /// A vector channel shows its component labels; a scalar one is just its
    /// name.
    #[rstest::rstest]
    #[case(&[], "accel")]
    #[case(&["x", "y", "z"], "accel (x, y, z)")]
    fn channel_title_appends_vector_components(
        #[case] components: &[&str],
        #[case] expected: &str,
    ) {
        let channel = ChannelSummary {
            name: "accel".to_owned(),
            unit: None,
            description: None,
            components: components.iter().map(|s| (*s).to_owned()).collect(),
            sample_count: 0,
        };

        assert_eq!(channel_title(&channel), expected);
    }

    /// The cursor the window is asking for right now.
    fn cursor_icon(h: &TestHarness<HistoryHarness>) -> egui::CursorIcon {
        h.inner.output().platform_output.cursor_icon
    }

    /// Each part of the window asks for the cursor that matches what it does.
    ///
    /// egui makes labels selectable by default, which puts a text-editing
    /// I-beam over every one of them - so a column header that sorts on click
    /// looked exactly like a text field. Only real text entry should show the
    /// I-beam here.
    #[rstest::rstest]
    // Sortable headers act on click.
    #[case::points_header(point_at_header, "Points", egui::CursorIcon::PointingHand)]
    #[case::identity_header(point_at_header, "Identity", egui::CursorIcon::PointingHand)]
    // The identity cell renames on double-click and has a context menu.
    #[case::identity_cell(point_at_widget, "sensors.gtd", egui::CursorIcon::PointingHand)]
    // The toolbar's "Identity" is a term with an explanation, not a control.
    #[case::term_label(point_at_widget, "Identity", egui::CursorIcon::Help)]
    // Values and captions do nothing on click.
    #[case::date_cell(point_at_widget, "2023-11-14 22:13", egui::CursorIcon::Default)]
    #[case::duration_cell(point_at_widget, "1h 00m", egui::CursorIcon::Default)]
    #[case::points_cell(point_at_widget, "8.9k", egui::CursorIcon::Default)]
    #[case::static_caption(point_at_widget, "GB", egui::CursorIcon::Default)]
    #[case::button(point_at_widget, "Prune…", egui::CursorIcon::Default)]
    #[case::checkbox(point_at_widget, "Auto-store recordings", egui::CursorIcon::Default)]
    fn elements_ask_for_a_cursor_that_matches_what_they_do(
        #[case] hover: fn(&mut TestHarness<HistoryHarness>, &str),
        #[case] label: &str,
        #[case] expected: egui::CursorIcon,
    ) {
        let mut h = channel_row_harness();

        hover(&mut h, label);

        assert_eq!(
            cursor_icon(&h),
            expected,
            "hovering {label:?} should ask for {expected:?}",
        );
    }

    /// The identity filter is real text entry, so it does get the I-beam - the
    /// contrast that makes the cursor meaningful everywhere else.
    #[test]
    fn the_filter_field_still_shows_a_text_cursor() {
        let mut h = channel_row_harness();
        let field = h
            .inner
            .get_all_by_role(egui::accesskit::Role::TextInput)
            .map(|n| n.rect())
            .next()
            .expect("identity filter field");

        h.inner.event(egui::Event::PointerMoved(field.center()));
        h.step();

        assert_eq!(cursor_icon(&h), egui::CursorIcon::Text);
    }

    /// A History window showing one recording that carries channels, settled so
    /// the auto columns have measured their content.
    fn channel_row_harness() -> TestHarness<'static, HistoryHarness> {
        let harness = history_harness(vec![entry_with_channels()]);
        let mut h = TestHarness::builder()
            .size(egui::vec2(900.0, 500.0))
            .ui_state(show_history, harness);
        for _ in 0..4 {
            h.run();
        }
        h
    }

    /// Hovering *any* of a row's value cells brings up the breakdown, not just
    /// the one column whose value is being pointed at - the cells are wired up
    /// individually, so each one has to be checked.
    #[rstest::rstest]
    #[case::date("2023-11-14 22:13")]
    #[case::duration("1h 00m")]
    #[case::points("8.9k")]
    #[case::size("4.0 KB")]
    fn hovering_any_value_cell_reveals_the_breakdown(#[case] cell_text: &str) {
        let mut h = channel_row_harness();
        assert!(
            h.inner.query_by_label_contains("custom channel").is_none(),
            "probe: the breakdown must not be visible before the hover",
        );

        hover_widget(&mut h, cell_text);

        assert!(
            h.inner
                .query_by_label_contains("2 custom channels")
                .is_some(),
            "hovering the {cell_text:?} cell should reveal the row's breakdown",
        );
    }

    /// The breakdown names the recording's ad-hoc sensor channels - their
    /// component labels, units, and sample counts - which no table column
    /// shows. This is the whole point of the hover.
    #[test]
    fn the_breakdown_names_the_recordings_channels() {
        let mut h = channel_row_harness();

        hover_widget(&mut h, "8.9k");

        for expected in [
            "2 custom channels",
            "accel (x, y, z)",
            "Frame IMU",
            "temperature",
            "12,000 samples",
            "512 samples",
            "Satellite reports",
            "1,234",
        ] {
            assert!(
                h.inner.query_by_label_contains(expected).is_some(),
                "the breakdown should mention {expected:?}",
            );
        }
    }

    /// The identity cell keeps its own metadata hover and gains the breakdown,
    /// so the tooltip tells the same story wherever along the row it opens.
    #[test]
    fn hovering_the_identity_cell_shows_metadata_and_the_breakdown() {
        let mut entry = entry_with_channels();
        entry.title = Some("Morning ride".to_owned());
        let harness = history_harness(vec![entry]);
        let mut h = TestHarness::builder()
            .size(egui::vec2(900.0, 500.0))
            .ui_state(show_history, harness);
        for _ in 0..4 {
            h.run();
        }

        hover_widget(&mut h, "sensors.gtd");

        for expected in [
            "Morning ride",
            "2 custom channels",
            "Double-click to rename",
        ] {
            assert!(
                h.inner.query_by_label_contains(expected).is_some(),
                "the identity hover should mention {expected:?}",
            );
        }
    }

    /// An identity too long for its column opens one tooltip, not two: egui
    /// offers the elided text its own tooltip, and the cell's hover already
    /// leads with the full identity.
    #[test]
    fn hovering_a_truncated_identity_opens_a_single_tooltip() {
        let long = "auto:a-recording-identity-far-too-long-for-the-identity-column.gtd";
        let harness = history_harness(vec![entry_with_identity(long)]);
        let mut h = TestHarness::builder()
            .size(egui::vec2(520.0, 300.0))
            .ui_state(show_history, harness);
        for _ in 0..4 {
            h.run();
        }

        hover_widget(&mut h, "a-recording-identity");

        assert!(
            h.inner
                .query_by_label_contains("Double-click to rename")
                .is_some(),
            "probe: the identity hover should be open",
        );
        assert_eq!(
            visible_tooltips(&h),
            1,
            "a truncated identity should not stack egui's elided-text tooltip \
             on top of the cell's own hover",
        );
    }

    /// How many tooltip layers are on screen.
    fn visible_tooltips(h: &TestHarness<HistoryHarness>) -> usize {
        h.inner.ctx.memory(|m| {
            m.areas()
                .visible_layer_ids()
                .iter()
                .filter(|layer| layer.order == egui::Order::Tooltip)
                .count()
        })
    }

    /// A recording with no channels says so on hover rather than leaving the
    /// question unanswered.
    #[test]
    fn hovering_a_channel_free_row_says_it_has_none() {
        let harness = history_harness(vec![sortable_entry(
            "auto:plain.gtd",
            1_700_000_000_000_000,
            900_000_000,
            42,
            4_096,
        )]);
        let mut h = TestHarness::builder()
            .size(egui::vec2(900.0, 500.0))
            .ui_state(show_history, harness);
        for _ in 0..4 {
            h.run();
        }

        hover_widget(&mut h, "42");

        assert!(
            h.inner
                .query_by_label_contains("No custom channels")
                .is_some(),
            "the breakdown should state that the recording carries no channels",
        );
    }

    /// Every value cell of a row gets its own breakdown widget id: one per
    /// column, and different between rows. Dropping either part of the salt
    /// would silently merge neighbouring cells' interaction state.
    #[test]
    fn breakdown_cell_ids_are_distinct_per_cell() {
        let entries = sortable_entries();
        let first = entries.first().expect("first entry");
        let second = entries.get(1).expect("second entry");

        let cells: Vec<egui::Id> = SortColumn::iter()
            .flat_map(|column| {
                [
                    breakdown_cell_id(first, column),
                    breakdown_cell_id(second, column),
                ]
            })
            .collect();
        let unique: std::collections::HashSet<egui::Id> = cells.iter().copied().collect();

        assert_eq!(
            unique.len(),
            cells.len(),
            "two breakdown cells share a widget id: {} cells produced {} ids",
            cells.len(),
            unique.len(),
        );
    }

    /// The track row calls out hidden tracks, and stays quiet when there are
    /// none - it is the only place the breakdown mentions them.
    #[rstest::rstest]
    #[case(4, 0, "4")]
    #[case(4, 1, "4 (1 hidden)")]
    #[case(0, 0, "0")]
    fn track_count_text_names_hidden_tracks(
        #[case] total_tracks: usize,
        #[case] hidden_tracks: usize,
        #[case] expected: &str,
    ) {
        let mut entry = entry_with_identity("auto:ride.gtd");
        entry.total_tracks = total_tracks;
        entry.hidden_tracks = hidden_tracks;

        assert_eq!(track_count_text(&entry), expected);
    }

    #[test]
    fn identity_display_keeps_full_manual_identity_visible() {
        let identity = "/example.invalid/history/identity/with/slashes/";

        assert_eq!(identity_display_parts(identity), (identity, false));
    }

    #[test]
    fn identity_display_marks_auto_identity_without_losing_original() {
        let identity = "auto:recording-2026-07-09.gtd";

        assert_eq!(
            identity_display_parts(identity),
            ("recording-2026-07-09.gtd", true)
        );
    }
}

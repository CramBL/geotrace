//! The query window: a small pipeline language for ad-hoc analysis of the
//! loaded data. Editor with syntax highlighting, run on the currently
//! visible tracks, and a results area whose matches also draw on the map as
//! halos.

use std::collections::HashMap;
use std::ops::Range;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use chrono::{DateTime, Utc};
use egui::text::{CCursor, CCursorRange, LayoutJob};
use gt_analysis::loss_of_lock::{self, SECS_PER_MIN, SlipRatePerPoint};
use gt_analysis::satellite_utilization::{self, UtilPerPoint};
use gt_filter::GlobalFilter;
use gt_loaded_files::LoadedFilesView;
use gt_query::lexer::{self, TokenClass};
use gt_query::{
    CheckedQuery, Construct, ConstructKind, Diagnostic, MetricProvider, Quantity, QueryMetric,
    RunOutput, Span, TrackInput, Unit,
};
use gt_types::satellites::Constellation;
use gt_types::{DisplayMode, FileIdx, LoadedFile, NavPoint, TrackIdx, TrackRef};
use gt_ui_theme::{DEGREE_SIGN, EM_DASH};
use gt_ui_types::{MapHighlight, QueryMatches, TrackDataVisibility};

use crate::settings::QueryHistoryEntry;

/// Rows shown per match table before truncating with a "more points" note.
const MATCH_TABLE_ROW_CAP: usize = 100;

/// Unpinned history entries kept before the oldest is evicted. Pinned
/// entries never count against this cap.
const MAX_UNPINNED_HISTORY: usize = 50;

/// Characters of a history entry's first line shown before eliding.
const HISTORY_LINE_MAX_CHARS: usize = 48;

/// Ellipsis appended to an elided history line.
const ELLIPSIS: &str = "…";

/// Id salt for the query editor's text field. Fixed (not derived from the
/// enclosing `Ui`) so the autocomplete caret/focus plumbing - and the UI
/// snapshot test - can address the widget directly.
pub(crate) const EDITOR_ID_SALT: &str = "query_editor";

/// Candidate rows the autocomplete popup shows before it scrolls. A footer
/// notes how many more there are.
const AUTOCOMPLETE_VISIBLE_ROWS: usize = 5;

/// A built-in query offered in the examples list.
struct QueryExample {
    name: &'static str,
    description: &'static str,
    text: &'static str,
}

/// Starter queries, mirroring the documented use cases. Embedded, not
/// persisted; every one is asserted to parse, check, and run by a test.
const EXAMPLES: &[QueryExample] = &[
    QueryExample {
        name: "Steady acceleration",
        description: "Stretches of constant-heading speed-up",
        text: "points\n| window 10\n| where spread(heading) <= 10 deg\n    and avg(accel) >= 0.3 m/s2\n    and avg(velocity) > 30 km/h\n| draw\n| table time, velocity, heading, accel",
    },
    QueryExample {
        name: "Poor accuracy while moving",
        description: "eph worse than 20 m above walking speed",
        text: "points\n| where eph > 20 m and velocity > 5 km/h",
    },
    QueryExample {
        name: "Weak fix",
        description: "Fewer than 6 satellites used in the fix",
        text: "points\n| where sats_fix < 6",
    },
    QueryExample {
        name: "Heading jitter",
        description: "Heading spread above 90 deg within 5 points while moving - a multipath indicator",
        text: "points\n| window 5\n| where spread(heading) > 90 deg and min(velocity) > 15 km/h\n| draw\n| table time, heading, velocity",
    },
    QueryExample {
        name: "Low GPS utilization",
        description: "In-fix share of visible GPS satellites below 50 %",
        text: "points\n| with mask 15 deg\n| where util_gps < 50 %\n| draw\n| table time, util_gps, sats_fix",
    },
    QueryExample {
        name: "Hide stationary points",
        description: "Drop points below walking speed to declutter a parked track",
        text: "points\n| where velocity < 2 km/h\n| hide",
    },
];

/// The floating query window and the results of its last run.
pub struct QueryWindow {
    pub open: bool,
    text: String,
    /// Outcome of checking `text`, kept in sync by `editor_ui`.
    checked: Result<CheckedQuery, Diagnostic>,
    /// The text `checked` was computed from.
    checked_text: String,
    /// Set by the Run button, consumed at the end of `show`.
    run_requested: bool,
    /// Set by the Cancel button while a run is in flight.
    cancel_requested: bool,
    running: Option<RunningQuery>,
    results: Option<QueryResults>,
    /// Previously run queries, newest first. Persisted in settings.
    history: Vec<QueryHistoryEntry>,
    /// Bumped on every history mutation so the config dirty-check (which
    /// compares a flat snapshot and cannot see into a growing `Vec`) notices
    /// and flushes.
    history_revision: u64,
    /// The editor's autocomplete popup, recomputed each frame from the caret.
    autocomplete: Autocomplete,
}

/// The editor's autocomplete popup state.
///
/// Recomputed each frame from the caret by [`QueryWindow::update_autocomplete`]
/// and drawn under the caret. Its key handling runs at the *start* of the next
/// frame ([`QueryWindow::apply_autocomplete_input`]) so it can claim
/// Enter/Tab/arrows before the text editor consumes them.
#[derive(Default)]
struct Autocomplete {
    /// Candidates for the caret as of the last frame, best first. Empty when
    /// the popup is not shown.
    items: Vec<Construct>,
    /// Byte range of the partial word an accepted candidate replaces.
    range: Range<usize>,
    /// The highlighted row.
    selected: usize,
    /// Screen position for the popup (just below the caret), cached so the
    /// popup can still be drawn on the frame a click steals the editor's focus.
    caret_pos: egui::Pos2,
    /// The text at which Esc dismissed the popup; it stays closed until the
    /// text changes again.
    dismissed_text: Option<String>,
    /// Whether the popup was drawn last frame. Key handling keys off this
    /// rather than live focus: egui surrenders a widget's focus on Escape (and
    /// on a click into the popup) *before* the editor renders, so live focus
    /// reads false on the very frame the popup must still act.
    shown: bool,
}

/// A run in flight on the worker thread.
struct RunningQuery {
    cancel: Arc<AtomicBool>,
    /// Tracks whose derived series are prepared, for the progress line.
    tracks_prepared: Arc<AtomicUsize>,
    track_total: usize,
    rx: mpsc::Receiver<RunCompleted>,
    /// Snapshot taken when the run started, attached to its results.
    fingerprint: RunFingerprint,
    /// The display mode the query asked for, carried to the map.
    mode: DisplayMode,
}

/// What the worker sends back. `output: None` means the run was cancelled -
/// previous results stay untouched.
struct RunCompleted {
    output: Option<RunOutput>,
    track_data: HashMap<TrackRef, TrackQueryData>,
}

/// Everything a run's results depend on besides the query text. Results
/// gray out when the current state no longer matches the snapshot.
#[derive(Debug, Clone, PartialEq)]
struct RunFingerprint {
    file_identities: Vec<String>,
    /// The tracks the run evaluated: enabled in the tree and passing the
    /// track-level global filter, in tree order.
    tracks: Vec<TrackRef>,
    filter: GlobalFilter,
}

/// Everything one run produced and the UI needs to show it.
struct QueryResults {
    matches: QueryMatches,
    summary: String,
    columns: Vec<QueryMetric>,
    /// Per-track derived series (only for metrics the query referenced),
    /// kept so match tables show the exact values the run used.
    track_data: HashMap<TrackRef, TrackQueryData>,
    fingerprint: RunFingerprint,
}

/// Owned per-track inputs for [`TrackProvider`], computed once per run.
#[derive(Default)]
struct TrackQueryData {
    util: Option<UtilPerPoint>,
    slip: Option<SlipRatePerPoint>,
    /// Index of the first point inside the global time filter - the offset
    /// between slice-relative evaluation indices and absolute point indices.
    slice_start: usize,
}

impl QueryWindow {
    pub fn new() -> Self {
        let text = String::new();
        Self {
            checked: check_text(&text),
            checked_text: text.clone(),
            text,
            open: false,
            run_requested: false,
            cancel_requested: false,
            running: None,
            results: None,
            history: Vec::new(),
            history_revision: 0,
            autocomplete: Autocomplete::default(),
        }
    }

    /// Matches of the last run, for the map. `None` when there are none.
    pub fn matches(&self) -> Option<&QueryMatches> {
        self.results.as_ref().map(|r| &r.matches)
    }

    /// Whether a query is currently affecting the map: halos for `draw`/`hide`
    /// with matches, or `keep` (which always filters). Drives the toolbar
    /// indicator shown while the window is closed.
    pub fn filter_active(&self) -> bool {
        self.results.as_ref().is_some_and(|results| {
            let has_matches = results.matches.ranges.values().any(|r| !r.is_empty());
            match results.matches.mode {
                DisplayMode::Draw | DisplayMode::Hide => has_matches,
                DisplayMode::Keep => true,
            }
        })
    }

    /// Drop the last run's results so the map returns to normal, abandoning any
    /// run still in flight. Called by the toolbar's clear action and by the
    /// side panel's "Reset filters".
    pub fn clear_filter(&mut self) {
        self.results = None;
        // Dropping the handle detaches the worker; its result is discarded.
        self.running = None;
    }

    /// Replace the editor text, e.g. when loading a history entry or an
    /// example. Never runs - running stays an explicit action.
    pub fn set_text(&mut self, text: String) {
        self.text = text;
    }

    /// The current editor text (used by tests to observe loads).
    #[cfg(test)]
    pub fn text(&self) -> &str {
        &self.text
    }

    /// The names currently offered by the autocomplete popup, best first (used
    /// by the UI test to assert the popup's contents).
    #[cfg(test)]
    pub fn autocomplete_names(&self) -> Vec<&'static str> {
        self.autocomplete.items.iter().map(|c| c.name).collect()
    }

    /// The persisted query history, newest first.
    pub fn history(&self) -> &[QueryHistoryEntry] {
        &self.history
    }

    /// Load the history from settings at startup.
    pub fn set_history(&mut self, history: Vec<QueryHistoryEntry>) {
        self.history = history;
    }

    /// Monotonic counter of history mutations, for the config dirty-check.
    pub fn history_revision(&self) -> u64 {
        self.history_revision
    }

    /// Record a query that is about to run: deduplicated by trimmed text
    /// (rerunning moves the entry to the top and keeps its pin), capped at
    /// [`MAX_UNPINNED_HISTORY`] unpinned entries with the oldest evicted.
    fn record_run(&mut self, text: &str) {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return;
        }
        // Drop any prior entry with the same text, carrying its pin forward.
        let pinned = match self.history.iter().position(|e| e.text.trim() == trimmed) {
            Some(index) => self.history.remove(index).pinned,
            None => false,
        };
        self.history.insert(
            0,
            QueryHistoryEntry {
                text: text.to_owned(),
                pinned,
                last_run_unix_ms: Utc::now().timestamp_millis(),
            },
        );
        // `retain` runs newest-first, so this keeps the first
        // MAX_UNPINNED_HISTORY unpinned entries and drops the older ones.
        let mut unpinned = 0;
        self.history.retain(|e| {
            if e.pinned {
                return true;
            }
            unpinned += 1;
            unpinned <= MAX_UNPINNED_HISTORY
        });
        self.history_revision += 1;
    }

    /// Render the window and handle runs. Call after the plot-hover
    /// forwarding: match-table row hover writes the same cross-highlight
    /// fields and must win for the frame.
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        loaded_files: LoadedFilesView<'_>,
        visibility: &TrackDataVisibility,
        filter: &GlobalFilter,
        highlight: &mut MapHighlight,
    ) {
        // Collect a finished worker even while the window is closed, so its
        // results are there on reopen.
        self.drain_completed();

        if !self.open {
            return;
        }

        // Results gray out when anything they depend on changed: loaded
        // files, track visibility, or the global filter.
        if let Some(results) = &mut self.results {
            results.matches.stale =
                current_fingerprint(loaded_files, visibility, filter) != results.fingerprint;
        }

        let files = loaded_files.files();
        let mut open = self.open;
        egui::Window::new("Query")
            .open(&mut open)
            .default_width(460.0)
            .default_height(520.0)
            .resizable(true)
            .vscroll(true)
            .show(ctx, |ui| {
                self.editor_ui(ui);
                ui.separator();
                self.results_ui(ui, files, highlight);
                ui.separator();
                self.history_examples_ui(ui);
            });

        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
            open = false;
        }
        self.open = open;

        // Ctrl+Enter (Cmd+Enter on macOS) runs, mirroring the Run button.
        // Consumed only while the window is open, so it never steals the
        // chord from other widgets.
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::Enter))
            && self.checked.is_ok()
            && self.running.is_none()
        {
            self.run_requested = true;
        }

        if self.cancel_requested {
            self.cancel_requested = false;
            if let Some(running) = &self.running {
                running.cancel.store(true, Ordering::Relaxed);
            }
        }
        // The run button lives inside `editor_ui`; it sets this flag. One
        // run at a time - the button is disabled while one is in flight.
        if self.run_requested {
            self.run_requested = false;
            if self.running.is_none() {
                self.record_run(&self.text.clone());
                self.spawn_run(ctx, loaded_files, visibility, filter);
            }
        }
    }

    /// The collapsible query-history and examples lists below the results.
    /// Loading an entry only fills the editor - running stays explicit.
    fn history_examples_ui(&mut self, ui: &mut egui::Ui) {
        // Gather actions during the borrow of `self.history`, apply after.
        let mut load: Option<String> = None;
        let mut toggle_pin: Option<usize> = None;
        let mut delete: Option<usize> = None;
        let mut clear_history = false;
        let now = Utc::now();

        egui::CollapsingHeader::new("Query history")
            .default_open(false)
            .show(ui, |ui| {
                if self.history.is_empty() {
                    ui.label(egui::RichText::new("No queries run yet").weak());
                    return;
                }
                if ui
                    .small_button(egui_phosphor::regular::TRASH)
                    .on_hover_text("Clear the query history (pinned queries are kept)")
                    .clicked()
                {
                    clear_history = true;
                }
                // A table so the age and remove columns line up across rows.
                egui::Grid::new(ui.id().with("query_history_grid"))
                    .num_columns(4)
                    .spacing(egui::vec2(8.0, 4.0))
                    .show(ui, |ui| {
                        for (index, entry) in self.history.iter().enumerate() {
                            let pin_hover = if entry.pinned {
                                "Pinned, never evicted. Click to unpin."
                            } else {
                                "Pin so this query is never evicted"
                            };
                            if ui
                                .selectable_label(entry.pinned, egui_phosphor::regular::PUSH_PIN)
                                .on_hover_text(pin_hover)
                                .clicked()
                            {
                                toggle_pin = Some(index);
                            }
                            // The button flattens the query and drops comments;
                            // its hover shows the full verbatim text (comments
                            // included). Loading restores that text unchanged.
                            if ui
                                .button(query_one_line(&entry.text))
                                .on_hover_text(&entry.text)
                                .clicked()
                            {
                                load = Some(entry.text.clone());
                            }
                            let age =
                                DateTime::<Utc>::from_timestamp_millis(entry.last_run_unix_ms)
                                    .map_or_else(String::new, |last_run| {
                                        format_history_age(now - last_run)
                                    });
                            ui.label(egui::RichText::new(age).weak());
                            if ui
                                .small_button(egui_phosphor::regular::X)
                                .on_hover_text("Remove from history")
                                .clicked()
                            {
                                delete = Some(index);
                            }
                            ui.end_row();
                        }
                    });
            });

        egui::CollapsingHeader::new("Examples")
            .default_open(false)
            .show(ui, |ui| {
                for example in EXAMPLES {
                    if ui
                        .button(example.name)
                        .on_hover_text(example.description)
                        .clicked()
                    {
                        load = Some(example.text.to_owned());
                    }
                }
            });

        if let Some(text) = load {
            self.set_text(text);
        }
        if let Some(index) = toggle_pin
            && let Some(entry) = self.history.get_mut(index)
        {
            entry.pinned = !entry.pinned;
            self.history_revision += 1;
        }
        if let Some(index) = delete
            && index < self.history.len()
        {
            self.history.remove(index);
            self.history_revision += 1;
        }
        if clear_history {
            // Pins mark queries to keep, so clear-all still respects them.
            self.history.retain(|entry| entry.pinned);
            self.history_revision += 1;
        }
    }

    /// Collect the worker's completion message, if any.
    fn drain_completed(&mut self) {
        let Some(running) = &self.running else {
            return;
        };
        let completed = match running.rx.try_recv() {
            Ok(completed) => completed,
            Err(mpsc::TryRecvError::Empty) => return,
            // The worker is gone without a message; nothing to keep.
            Err(mpsc::TryRecvError::Disconnected) => {
                log::error!("query worker disappeared without completing");
                self.running = None;
                return;
            }
        };
        let Some(running) = self.running.take() else {
            return;
        };
        // A cancelled run keeps the previous results - partial output is
        // never shown.
        let Some(output) = completed.output else {
            return;
        };

        let summary = summary_line(&output, running.mode);
        let mut ranges: HashMap<TrackRef, Vec<Range<usize>>> = HashMap::new();
        let mut track_data = completed.track_data;
        for track_matches in &output.matches {
            let start = track_data
                .get(&track_matches.track)
                .map_or(0, |d| d.slice_start);
            // Evaluation ran on the time-filtered slice; map indices back to
            // absolute positions in the track.
            let absolute = track_matches
                .ranges
                .iter()
                .map(|r| r.start + start..r.end + start)
                .collect();
            ranges.insert(track_matches.track, absolute);
        }
        // Tracks without matches keep no entry, and unreferenced derived
        // series were never computed - drop the empties.
        track_data.retain(|track_ref, _| ranges.contains_key(track_ref));

        self.results = Some(QueryResults {
            matches: QueryMatches {
                ranges,
                mode: running.mode,
                stale: false,
            },
            summary,
            columns: output.columns,
            track_data,
            fingerprint: running.fingerprint,
        });
    }

    fn editor_ui(&mut self, ui: &mut egui::Ui) {
        let editor_id = egui::Id::new(EDITOR_ID_SALT);
        // Runs before the editor so the open popup can claim Enter/Tab/arrows,
        // and may edit the text (accepting a candidate) - so re-check after.
        self.apply_autocomplete_input(ui, editor_id);

        if self.checked_text != self.text {
            self.checked = check_text(&self.text);
            self.checked_text = self.text.clone();
        }
        let diagnostic_span = self.checked.as_ref().err().map(|d| d.span);

        let mut layouter = |ui: &egui::Ui, buf: &dyn egui::TextBuffer, wrap_width: f32| {
            let mut job = highlight_layout(ui, buf.as_str(), diagnostic_span);
            job.wrap.max_width = wrap_width;
            ui.fonts_mut(|f| f.layout_job(job))
        };
        let output = egui::TextEdit::multiline(&mut self.text)
            .id(editor_id)
            .code_editor()
            .desired_rows(5)
            .desired_width(f32::INFINITY)
            .hint_text("points | where velocity > 30 km/h")
            .layouter(&mut layouter)
            .show(ui);

        self.update_autocomplete(ui, editor_id, &output);
        self.hover_docs(ui, &output);

        if let Err(diagnostic) = &self.checked
            && !self.text.trim().is_empty()
        {
            // The message shows in red with an error icon (the quoted token
            // lifts into the code font); the fix, carried in the structured
            // `help`, is a plain "Hint:" line below.
            ui.label(error_message_layout(ui, &diagnostic.message));
            if let Some(hint) = &diagnostic.help {
                ui.label(format!("Hint: {hint}"));
            }
        }

        ui.horizontal(|ui| {
            let in_flight = self.running.is_some();
            let runnable = self.checked.is_ok() && !in_flight;
            let run = ui.add_enabled(runnable, egui::Button::new("Run"));
            let run = match (self.checked.is_ok(), in_flight) {
                (false, _) => run.on_disabled_hover_text("Fix the error above to run"),
                (true, true) => run.on_disabled_hover_text("A run is in progress"),
                (true, false) => run,
            };
            if run.clicked() {
                self.run_requested = true;
            }

            let cancel = ui.add_enabled(in_flight, egui::Button::new("Cancel"));
            let cancel = if in_flight {
                cancel
            } else {
                cancel.on_disabled_hover_text("No run in progress")
            };
            if cancel.clicked() {
                self.cancel_requested = true;
            }

            let clearable = !self.text.is_empty();
            let clear = ui.add_enabled(clearable, egui::Button::new("Clear"));
            let clear = if clearable {
                clear
            } else {
                clear.on_disabled_hover_text("The editor is already empty")
            };
            if clear.clicked() {
                self.text.clear();
                self.autocomplete = Autocomplete::default();
            }

            if let Some(running) = &self.running {
                ui.spinner();
                let prepared = running.tracks_prepared.load(Ordering::Relaxed);
                if prepared < running.track_total {
                    ui.label(format!(
                        "Preparing {prepared}/{} tracks",
                        running.track_total
                    ));
                } else {
                    ui.label("Evaluating");
                }
                // Keep repainting so progress and completion show promptly.
                ui.ctx().request_repaint_after(Duration::from_millis(100));
            }
        });
    }

    /// Handle keyboard acceptance of the autocomplete popup, using the
    /// candidates computed last frame. Called before the editor renders so the
    /// popup can consume Enter/Tab/arrows the text field would otherwise take.
    /// (Mouse clicks are handled inline in `update_autocomplete`.)
    fn apply_autocomplete_input(&mut self, ui: &egui::Ui, editor_id: egui::Id) {
        // Keyed off `shown` (last frame), not live focus, because egui
        // surrenders the editor's focus on Escape before this runs. Key state
        // is read inside `input_mut`, but the follow-up (focus, text edits)
        // happens after - re-entering the context lock inside would deadlock.
        let mut accept = None;
        let mut dismissed = false;
        if self.autocomplete.shown && !self.autocomplete.items.is_empty() {
            let len = self.autocomplete.items.len();
            ui.input_mut(|input| {
                if input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown) {
                    self.autocomplete.selected = (self.autocomplete.selected + 1) % len;
                }
                if input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp) {
                    self.autocomplete.selected = (self.autocomplete.selected + len - 1) % len;
                }
                if input.consume_key(egui::Modifiers::NONE, egui::Key::Escape) {
                    dismissed = true;
                    return;
                }
                // Enter and Tab both accept; Ctrl+Enter carries the COMMAND
                // modifier, so it is left for the window's run shortcut.
                if input.consume_key(egui::Modifiers::NONE, egui::Key::Enter)
                    || input.consume_key(egui::Modifiers::NONE, egui::Key::Tab)
                {
                    accept = Some(self.autocomplete.selected);
                }
            });
        }

        if dismissed {
            self.autocomplete.dismissed_text = Some(self.text.clone());
            self.autocomplete.items.clear();
            self.autocomplete.shown = false;
            // egui already dropped the editor's focus for this Escape; put it
            // back so dismissing the popup keeps the caret in the editor.
            ui.ctx().memory_mut(|m| m.request_focus(editor_id));
            return;
        }

        if let Some(index) = accept
            && let Some(&construct) = self.autocomplete.items.get(index)
        {
            self.accept_completion(ui, editor_id, construct);
        }
    }

    /// Replace the partial word under the caret with `construct`, then move the
    /// caret past the insertion and close the popup for that word.
    fn accept_completion(&mut self, ui: &egui::Ui, editor_id: egui::Id, construct: Construct) {
        let range = self.autocomplete.range.clone();
        // Guard against a range that a same-frame edit already invalidated.
        if range.end > self.text.len() {
            return;
        }
        let space = if inserts_trailing_space(construct.kind) {
            " "
        } else {
            ""
        };
        let insertion = format!("{}{space}", construct.name);
        let caret_byte = range.start + insertion.len();
        self.text.replace_range(range, &insertion);

        let caret_char = self.text.get(..caret_byte).map_or(0, |s| s.chars().count());
        let mut state = egui::TextEdit::load_state(ui.ctx(), editor_id).unwrap_or_default();
        state
            .cursor
            .set_char_range(Some(CCursorRange::one(CCursor::new(caret_char))));
        egui::TextEdit::store_state(ui.ctx(), editor_id, state);
        ui.ctx().memory_mut(|m| m.request_focus(editor_id));

        self.autocomplete.items.clear();
        self.autocomplete.selected = 0;
        self.autocomplete.dismissed_text = None;
        self.autocomplete.shown = false;
    }

    /// Recompute the candidates for the current caret, draw the popup under it,
    /// and accept a clicked row. While the editor is focused the candidates are
    /// recomputed; on the frame a click into the popup steals that focus, the
    /// popup is redrawn from the cached candidates so the click still lands.
    fn update_autocomplete(
        &mut self,
        ui: &egui::Ui,
        editor_id: egui::Id,
        output: &egui::widgets::text_edit::TextEditOutput,
    ) {
        let focused = output.response.has_focus();
        if let (true, Some(caret_char)) = (
            focused,
            output.cursor_range.map(|range| range.primary.index),
        ) {
            // Esc keeps the popup closed until the text changes.
            if self.autocomplete.dismissed_text.as_deref() == Some(self.text.as_str()) {
                self.autocomplete.items.clear();
                self.autocomplete.shown = false;
                return;
            }
            self.autocomplete.dismissed_text = None;

            let caret_byte = char_to_byte(&self.text, caret_char);
            let completions = gt_query::completions_at(&self.text, caret_byte);
            let items = completions.items;

            // Keep the highlighted row while the candidate set is unchanged;
            // otherwise start at the top.
            let unchanged = items.iter().map(|c| c.name).eq(self
                .autocomplete
                .items
                .iter()
                .map(|c| c.name));
            self.autocomplete.selected = if unchanged {
                self.autocomplete
                    .selected
                    .min(items.len().saturating_sub(1))
            } else {
                0
            };
            self.autocomplete.items = items;
            self.autocomplete.range = completions.range;
            let caret_rect = output.galley.pos_from_cursor(CCursor::new(caret_char));
            self.autocomplete.caret_pos =
                output.galley_pos + caret_rect.left_bottom().to_vec2() + egui::vec2(0.0, 2.0);
        } else if !self.autocomplete.shown {
            // Not editing and nothing was open: keep it closed.
            self.autocomplete.items.clear();
            return;
        }

        if self.autocomplete.items.is_empty() {
            self.autocomplete.shown = false;
            return;
        }
        let clicked = draw_autocomplete_popup(
            ui,
            output.response.id,
            self.autocomplete.caret_pos,
            &self.autocomplete,
        );
        self.autocomplete.shown = true;

        if let Some(index) = clicked {
            if let Some(&construct) = self.autocomplete.items.get(index) {
                self.accept_completion(ui, editor_id, construct);
            }
        } else if !focused {
            // Focus left the editor without a click into the popup (e.g. a
            // click elsewhere) - close it.
            self.autocomplete.items.clear();
            self.autocomplete.shown = false;
        }
    }

    /// Show a documentation tooltip for the construct under the pointer, in the
    /// editor. Suppressed while the completion popup is up, so the two don't
    /// stack.
    fn hover_docs(&self, ui: &egui::Ui, output: &egui::widgets::text_edit::TextEditOutput) {
        if self.autocomplete.shown {
            return;
        }
        let Some(pointer) = ui.ctx().pointer_hover_pos() else {
            return;
        };
        if !output.response.rect.contains(pointer) {
            return;
        }
        let ccursor = output.galley.cursor_from_pos(pointer - output.galley_pos);
        let byte = char_to_byte(&self.text, ccursor.index);
        let Some(construct) = gt_query::construct_at(&self.text, byte) else {
            return;
        };
        // Drawn as an Area (rather than a hover tooltip) so it is anchored to
        // the token under the pointer and shows without a hover delay.
        egui::Area::new(egui::Id::new("query_hover_doc"))
            .order(egui::Order::Tooltip)
            .fixed_pos(pointer + egui::vec2(12.0, 18.0))
            .constrain(true)
            .interactable(false)
            .show(ui.ctx(), |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    construct_tooltip_ui(ui, construct);
                });
            });
    }

    fn results_ui(&self, ui: &mut egui::Ui, files: &[LoadedFile], highlight: &mut MapHighlight) {
        let Some(results) = &self.results else {
            ui.label(egui::RichText::new("No runs yet").weak());
            return;
        };
        let stale = results.matches.stale;

        // The run summary is the collapsible header; the match tables are its
        // body. A stable id keeps the open/closed state across reruns even as
        // the summary text changes. No inner scroll - the window scrolls as a
        // whole (see `show`), so the scrollbar sits at its edge.
        egui::CollapsingHeader::new(&results.summary)
            .id_salt("query_matches")
            .default_open(true)
            .show(ui, |ui| {
                for (track_ref, ranges) in matches_in_order(&results.matches) {
                    for range in ranges {
                        match_ui(ui, files, results, track_ref, range, stale, highlight);
                    }
                }
            });
        if stale {
            ui.label(
                egui::RichText::new(format!("Data changed since this run {EM_DASH} run again"))
                    .weak()
                    .italics(),
            );
        }
    }

    /// Parse/check are already done (`self.checked`); snapshot the visible
    /// data and hand the evaluation to a worker thread.
    #[expect(
        clippy::expect_used,
        reason = "thread spawn can only fail under extreme system resource exhaustion"
    )]
    fn spawn_run(
        &mut self,
        ctx: &egui::Context,
        loaded_files: LoadedFilesView<'_>,
        visibility: &TrackDataVisibility,
        filter: &GlobalFilter,
    ) {
        let Ok(query) = &self.checked else {
            return;
        };
        let mode = query.mode();
        let query = query.clone();
        let fingerprint = current_fingerprint(loaded_files, visibility, filter);

        // Owned snapshot for the worker: each evaluated track's full point
        // vector plus the sub-range passing the time filter. Cloning is the
        // simple-and-correct baseline; an Arc-based snapshot is the known
        // follow-up if this shows up in profiling.
        let files = loaded_files.files();
        let tracks: Vec<(TrackRef, Vec<NavPoint>, Range<usize>)> = fingerprint
            .tracks
            .iter()
            .filter_map(|&track_ref| {
                let points = points_of(files, track_ref)?;
                let slice = gt_filter::time_filtered_range(points, filter);
                Some((track_ref, points.to_vec(), slice))
            })
            .collect();

        let cancel = Arc::new(AtomicBool::new(false));
        let tracks_prepared = Arc::new(AtomicUsize::new(0));
        let track_total = tracks.len();
        let (tx, rx) = mpsc::channel();

        let worker_cancel = Arc::clone(&cancel);
        let worker_prepared = Arc::clone(&tracks_prepared);
        let worker_ctx = ctx.clone();
        thread::Builder::new()
            .name("query-run".to_owned())
            .spawn(move || {
                let completed = run_worker(&query, &tracks, &worker_cancel, &worker_prepared);
                // A send failure means the window dropped the receiver;
                // nothing left to notify.
                tx.send(completed).ok();
                worker_ctx.request_repaint();
            })
            .expect("failed to spawn query worker thread");

        self.running = Some(RunningQuery {
            cancel,
            tracks_prepared,
            track_total,
            rx,
            fingerprint,
            mode,
        });
    }
}

/// The worker body: derived series per track, then the evaluation, with
/// cancellation checks between tracks (gt-query checks within them).
fn run_worker(
    query: &CheckedQuery,
    tracks: &[(TrackRef, Vec<NavPoint>, Range<usize>)],
    cancel: &AtomicBool,
    prepared: &AtomicUsize,
) -> RunCompleted {
    let cancelled = || cancel.load(Ordering::Relaxed);
    let uses_util = query.referenced_metrics().iter().any(|m| m.is_util());
    let uses_slip = query.referenced_metrics().iter().any(|m| m.is_slip());
    let params = query.params();

    let mut track_data: HashMap<TrackRef, TrackQueryData> = HashMap::new();
    for (track_ref, points, slice) in tracks {
        if cancelled() {
            return RunCompleted {
                output: None,
                track_data,
            };
        }
        track_data.insert(
            *track_ref,
            compute_track_data(points, params, uses_util, uses_slip, slice.start),
        );
        prepared.fetch_add(1, Ordering::Relaxed);
    }

    let providers: Vec<(TrackRef, SliceProvider<'_>)> = tracks
        .iter()
        .map(|(track_ref, points, slice)| {
            let provider = provider_for(points, track_data.get(track_ref));
            (
                *track_ref,
                SliceProvider {
                    inner: provider,
                    start: slice.start,
                    len: slice.len(),
                },
            )
        })
        .collect();
    let inputs: Vec<TrackInput<'_>> = providers
        .iter()
        .map(|(track_ref, provider)| TrackInput {
            track: *track_ref,
            provider,
        })
        .collect();

    RunCompleted {
        output: gt_query::run_cancellable(query, &inputs, &cancelled),
        track_data,
    }
}

/// One match: a collapsing header with the point table inside. Row hover
/// echoes the point on the map through the plot cross-highlight ring.
fn match_ui(
    ui: &mut egui::Ui,
    files: &[LoadedFile],
    results: &QueryResults,
    track_ref: TrackRef,
    range: &Range<usize>,
    stale: bool,
    highlight: &mut MapHighlight,
) {
    let header = match_header_text(files, track_ref, range);
    let id = ui.id().with(("query_match", track_ref, range.start));
    if stale {
        // Grayed out, not hidden: the rows reference point indices that may
        // no longer address the same data.
        ui.add_enabled(false, egui::Label::new(header))
            .on_disabled_hover_text(format!("Data changed since this run {EM_DASH} run again"));
        return;
    }
    egui::CollapsingHeader::new(header)
        .id_salt(id)
        .show(ui, |ui| {
            match_table_ui(ui, files, results, track_ref, range, highlight);
        });
}

fn match_table_ui(
    ui: &mut egui::Ui,
    files: &[LoadedFile],
    results: &QueryResults,
    track_ref: TrackRef,
    range: &Range<usize>,
    highlight: &mut MapHighlight,
) {
    let Some(points) = points_of(files, track_ref) else {
        return;
    };
    let data = results.track_data.get(&track_ref);
    let provider = provider_for(points, data);
    // accel derives through the same slice the evaluator saw, so the first
    // point of a time-filtered run shows the missing value the predicate
    // used, not a value reaching before the filter window.
    let slice_start = data.map_or(0, |d| d.slice_start);
    let slice = SliceProvider {
        inner: provider,
        start: slice_start,
        len: points.len().saturating_sub(slice_start),
    };

    egui::Grid::new(ui.id().with("match_table"))
        .striped(true)
        .show(ui, |ui| {
            for column in &results.columns {
                ui.strong(column.to_string());
            }
            ui.end_row();

            for pi in range.clone().take(MATCH_TABLE_ROW_CAP) {
                let mut row_hovered = false;
                for column in &results.columns {
                    let value = if *column == QueryMetric::Accel {
                        pi.checked_sub(slice_start)
                            .and_then(|rel| gt_query::derived_accel(&slice, rel))
                    } else {
                        provider.value(*column, pi)
                    };
                    let response = ui.label(format_value(*column, value));
                    row_hovered |= response.hovered();
                }
                if row_hovered {
                    // Echo the hovered row on the map, same ring as the
                    // plot cursor cross-highlight.
                    highlight.plot_hover_point =
                        Some((track_ref.fi, track_ref.index, gt_types::PointIdx::new(pi)));
                }
                ui.end_row();
            }
        });
    if range.len() > MATCH_TABLE_ROW_CAP {
        ui.label(
            egui::RichText::new(format!(
                "{EM_DASH} {} more points",
                range.len() - MATCH_TABLE_ROW_CAP
            ))
            .weak(),
        );
    }
}

fn match_header_text(files: &[LoadedFile], track_ref: TrackRef, range: &Range<usize>) -> String {
    let start_time = points_of(files, track_ref)
        .and_then(|points| points.get(range.start))
        .map(|p| p.tpv.time().utc().format("%H:%M:%S").to_string());
    let file = track_ref.fi.get(files).map_or_else(
        || format!("file {}", track_ref.fi),
        |f| f.metadata.filename.clone(),
    );
    let count = range.len();
    match start_time {
        Some(time) => format!(
            "{file} #{} {EM_DASH} {time} {EM_DASH} {count} points",
            track_ref.index
        ),
        None => format!("{file} #{} {EM_DASH} {count} points", track_ref.index),
    }
}

/// Coarse age for a history entry, at minute granularity or coarser.
///
/// Deliberately omits seconds: a second-resolution age changes every frame,
/// which reads as needless flicker in a list that is otherwise static.
fn format_history_age(age: chrono::Duration) -> String {
    let minutes = age.num_minutes();
    if minutes < 1 {
        return "now".to_owned();
    }
    if minutes < 60 {
        return format!("{minutes}m ago");
    }
    let hours = age.num_hours();
    if hours < 24 {
        return format!("{hours}h ago");
    }
    format!("{}d ago", age.num_days())
}

/// A query flattened to one line for the history label: comments dropped,
/// whitespace collapsed, elided to [`HISTORY_LINE_MAX_CHARS`].
///
/// Every query's first line is the bare `points` source, so the full
/// flattened pipeline distinguishes entries where the first line alone would
/// not. The untruncated text is available on hover.
///
/// Comment removal goes through the shared lexer (dropping its `Comment`
/// spans) rather than re-deriving comment syntax, so the two cannot drift.
/// Only comment spans are removed - the original spacing of the remaining
/// code is preserved, then whitespace is collapsed.
fn query_one_line(text: &str) -> String {
    let mut kept = String::with_capacity(text.len());
    let mut cursor = 0;
    for (span, class) in lexer::highlight_classes(text) {
        if matches!(class, TokenClass::Comment) {
            kept.push_str(text.get(cursor..span.start).unwrap_or(""));
            cursor = span.end;
        }
    }
    kept.push_str(text.get(cursor..).unwrap_or(""));

    let flat = kept.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= HISTORY_LINE_MAX_CHARS {
        return flat;
    }
    let truncated: String = flat.chars().take(HISTORY_LINE_MAX_CHARS).collect();
    format!("{truncated}{ELLIPSIS}")
}

/// Matches ordered by track then start index, for a stable list.
fn matches_in_order(matches: &QueryMatches) -> Vec<(TrackRef, &Vec<Range<usize>>)> {
    let mut entries: Vec<(TrackRef, &Vec<Range<usize>>)> =
        matches.ranges.iter().map(|(t, r)| (*t, r)).collect();
    entries.sort_by_key(|(t, _)| *t);
    entries
}

fn points_of(files: &[LoadedFile], track_ref: TrackRef) -> Option<&[NavPoint]> {
    track_ref.resolve(files).map(|t| t.points.as_slice())
}

/// The provider both the run and the match tables read through - one code
/// path, so tables always show the values the evaluator saw.
fn provider_for<'a>(points: &'a [NavPoint], data: Option<&'a TrackQueryData>) -> TrackProvider<'a> {
    TrackProvider {
        points,
        util: data.and_then(|d| d.util.as_ref()),
        slip: data.and_then(|d| d.slip.as_ref()),
    }
}

/// Snapshot of the state a run depends on, compared each frame against the
/// stored one to gray out outdated results.
fn current_fingerprint(
    loaded_files: LoadedFilesView<'_>,
    visibility: &TrackDataVisibility,
    filter: &GlobalFilter,
) -> RunFingerprint {
    let mut file_identities = Vec::with_capacity(loaded_files.entries().len());
    let mut tracks = Vec::new();
    for (fi, entry) in loaded_files.entries().enumerate() {
        file_identities.push(entry.identity_key().into_owned());
        let file = entry.file();
        let fi = FileIdx::new(fi);
        for (ti, track) in file.tracks.iter().enumerate() {
            let track_ref = TrackRef::new(fi, TrackIdx::new(ti));
            if visibility.track_enabled(track_ref)
                && gt_filter::track_passes_filter(&track.metadata, filter)
            {
                tracks.push(track_ref);
            }
        }
    }
    RunFingerprint {
        file_identities,
        tracks,
        filter: *filter,
    }
}

fn compute_track_data(
    points: &[NavPoint],
    params: gt_query::Params,
    uses_util: bool,
    uses_slip: bool,
    slice_start: usize,
) -> TrackQueryData {
    // gt_query::check::require_params guarantees these parameters whenever
    // the corresponding metrics are referenced - defaulting below is for the
    // Option unwrap only, never a real fallback.
    debug_assert!(
        !(uses_util || uses_slip) || params.mask_deg.is_some(),
        "checker must reject util/slip metrics without a mask"
    );
    debug_assert!(
        !uses_slip || (params.snr_drop_db_hz.is_some() && params.slip_window_s.is_some()),
        "checker must reject slip metrics without snr_drop and slip_window"
    );
    let mask_deg = params.mask_deg.unwrap_or_default() as f32;
    let util = uses_util.then(|| satellite_utilization::util_per_point(points, mask_deg));
    let slip = uses_slip.then(|| {
        loss_of_lock::slip_rate_per_point(
            points,
            mask_deg,
            params.snr_drop_db_hz.unwrap_or_default() as f32,
            (params.slip_window_s.unwrap_or_default() / SECS_PER_MIN) as f32,
        )
    });
    TrackQueryData {
        util,
        slip,
        slice_start,
    }
}

/// A window onto another provider: the evaluator sees only the points inside
/// the global time filter, while `inner` (and the derived series it carries)
/// stays indexed by absolute track position.
struct SliceProvider<'a> {
    inner: TrackProvider<'a>,
    start: usize,
    len: usize,
}

impl MetricProvider for SliceProvider<'_> {
    fn len(&self) -> usize {
        self.len
    }

    fn value(&self, metric: QueryMetric, index: usize) -> Option<f64> {
        if index >= self.len {
            return None;
        }
        self.inner.value(metric, self.start + index)
    }
}

fn summary_line(output: &RunOutput, mode: DisplayMode) -> String {
    let summary = &output.summary;
    let mut parts = vec![format!(
        "{} {} on {} {}",
        summary.match_count,
        gt_fmt::pluralize(summary.match_count, "match", "matches"),
        summary.tracks_with_matches,
        gt_fmt::pluralize(summary.tracks_with_matches, "track", "tracks"),
    )];
    // keep/hide remove points from the map; always say how many, so hidden
    // data stays accounted for.
    let hidden = match mode {
        DisplayMode::Draw => None,
        DisplayMode::Keep => Some(summary.total_points - summary.matched_points),
        DisplayMode::Hide => Some(summary.matched_points),
    };
    if let Some(hidden) = hidden {
        parts.push(format!(
            "{hidden} of {} points hidden",
            summary.total_points
        ));
    }
    for (metric, count) in &summary.skipped {
        parts.push(format!("{count} skipped (missing {metric})"));
    }
    if summary.skipped_non_finite > 0 {
        parts.push(format!(
            "{} skipped (undefined arithmetic)",
            summary.skipped_non_finite
        ));
    }
    if summary.tracks_shorter_than_window > 0 {
        parts.push(format!(
            "{} {} shorter than window",
            summary.tracks_shorter_than_window,
            gt_fmt::pluralize(summary.tracks_shorter_than_window, "track", "tracks"),
        ));
    }
    for param in &summary.unused_params {
        parts.push(format!("{param} declared but unused"));
    }
    parts.join(&format!(" {EM_DASH} "))
}

/// Provider over one track's points plus the run's derived series, in the
/// evaluator's base units (m/s, degrees, seconds, 0-1 ratios, per minute).
#[derive(Clone, Copy)]
struct TrackProvider<'a> {
    points: &'a [NavPoint],
    util: Option<&'a UtilPerPoint>,
    slip: Option<&'a SlipRatePerPoint>,
}

impl TrackProvider<'_> {
    fn util_value(
        &self,
        index: usize,
        series: impl Fn(&UtilPerPoint) -> &[Option<f64>],
    ) -> Option<f64> {
        let percent = self
            .util
            .and_then(|u| series(u).get(index).copied().flatten())?;
        // gt-analysis reports percent; the evaluator's ratio base is the 0-1
        // fraction, converted through the language's canonical % factor.
        Some(percent * Unit::Percent.to_base())
    }

    fn slip_value(
        &self,
        index: usize,
        series: impl Fn(&SlipRatePerPoint) -> &[Option<f64>],
    ) -> Option<f64> {
        self.slip
            .and_then(|s| series(s).get(index).copied().flatten())
    }

    fn counts(&self, index: usize, constellation: Constellation) -> SatCounts {
        self.points
            .get(index)
            .and_then(|p| p.satellites.as_ref())
            .map_or(SatCounts::default(), |sats| {
                sats.by_constellation(constellation)
                    .fold(SatCounts::default(), |acc, sat| SatCounts {
                        seen: acc.seen + 1,
                        fix: acc.fix + usize::from(sat.in_fix()),
                    })
            })
    }
}

/// Seen/in-fix satellite counts of one constellation at one point.
#[derive(Debug, Clone, Copy, Default)]
struct SatCounts {
    seen: usize,
    fix: usize,
}

impl MetricProvider for TrackProvider<'_> {
    fn len(&self) -> usize {
        self.points.len()
    }

    fn value(&self, metric: QueryMetric, index: usize) -> Option<f64> {
        use uom::si::angle::degree;
        use uom::si::velocity::meter_per_second;

        let point = self.points.get(index)?;
        let sats = point.satellites.as_ref();
        match metric {
            QueryMetric::Time => Some(point.tpv.time().as_secs_f64()),
            QueryMetric::SysTime => point
                .tpv
                .sys_time()
                .map(|s| s.utc().timestamp_millis() as f64 / 1_000.0),
            QueryMetric::Lat => Some(point.tpv.lat().as_degrees()),
            QueryMetric::Lon => Some(point.tpv.lon().as_degrees()),
            QueryMetric::Velocity => point.tpv.velocity().map(|v| v.get::<meter_per_second>()),
            QueryMetric::Heading => point.tpv.heading().map(|h| h.get::<degree>()),
            // Derived by the evaluator (`gt_query::derived_accel`), never
            // asked of providers.
            QueryMetric::Accel => None,
            QueryMetric::Eph => point.tpv.eph_m().map(f64::from),
            QueryMetric::ClockDelta => point.tpv.sys_time().map(|sys| {
                point.tpv.time().offset_from_sys(sys).num_milliseconds() as f64 / 1_000.0
            }),
            QueryMetric::SatsSeen => sats.map(|s| f64::from(s.satellite_count())),
            QueryMetric::SatsFix => sats.map(|s| f64::from(s.fix_count())),
            QueryMetric::GpsSeen => {
                sats.map(|_| self.counts(index, Constellation::Gps).seen as f64)
            }
            QueryMetric::GpsFix => sats.map(|_| self.counts(index, Constellation::Gps).fix as f64),
            QueryMetric::GlonassSeen => {
                sats.map(|_| self.counts(index, Constellation::Glonass).seen as f64)
            }
            QueryMetric::GlonassFix => {
                sats.map(|_| self.counts(index, Constellation::Glonass).fix as f64)
            }
            QueryMetric::GalileoSeen => {
                sats.map(|_| self.counts(index, Constellation::Galileo).seen as f64)
            }
            QueryMetric::GalileoFix => {
                sats.map(|_| self.counts(index, Constellation::Galileo).fix as f64)
            }
            QueryMetric::BeidouSeen => {
                sats.map(|_| self.counts(index, Constellation::Beidou).seen as f64)
            }
            QueryMetric::BeidouFix => {
                sats.map(|_| self.counts(index, Constellation::Beidou).fix as f64)
            }
            QueryMetric::NavicSeen => {
                sats.map(|_| self.counts(index, Constellation::Navic).seen as f64)
            }
            QueryMetric::NavicFix => {
                sats.map(|_| self.counts(index, Constellation::Navic).fix as f64)
            }
            QueryMetric::QzssSeen => {
                sats.map(|_| self.counts(index, Constellation::Qzss).seen as f64)
            }
            QueryMetric::QzssFix => {
                sats.map(|_| self.counts(index, Constellation::Qzss).fix as f64)
            }
            QueryMetric::UtilAll => self.util_value(index, |u| &u.all),
            QueryMetric::UtilGps => self.util_value(index, |u| &u.gps),
            QueryMetric::UtilGlonass => self.util_value(index, |u| &u.glonass),
            QueryMetric::UtilGalileo => self.util_value(index, |u| &u.galileo),
            QueryMetric::UtilBeidou => self.util_value(index, |u| &u.beidou),
            QueryMetric::UtilNavic => self.util_value(index, |u| &u.navic),
            QueryMetric::UtilQzss => self.util_value(index, |u| &u.qzss),
            QueryMetric::SlipAll => self.slip_value(index, |s| &s.all),
            QueryMetric::SlipGps => self.slip_value(index, |s| &s.gps),
            QueryMetric::SlipGlonass => self.slip_value(index, |s| &s.glonass),
            QueryMetric::SlipGalileo => self.slip_value(index, |s| &s.galileo),
            QueryMetric::SlipBeidou => self.slip_value(index, |s| &s.beidou),
            QueryMetric::SlipNavic => self.slip_value(index, |s| &s.navic),
            QueryMetric::SlipQzss => self.slip_value(index, |s| &s.qzss),
        }
    }
}

fn check_text(text: &str) -> Result<CheckedQuery, Diagnostic> {
    gt_query::check(&gt_query::parse(text)?)
}

/// Whether accepting a construct of this kind should append a space, because
/// something always follows it (a value, an operand, the next stage). Kinds
/// that lead straight into punctuation - `avg` into `(`, a metric into an
/// operator, a unit into `|` - get none.
fn inserts_trailing_space(kind: ConstructKind) -> bool {
    matches!(
        kind,
        ConstructKind::Source | ConstructKind::Stage | ConstructKind::Param
    )
}

/// Byte offset of the `char_index`-th character, or the text length when the
/// index is at or past the end. The egui caret is a char index; the query
/// position model works in bytes.
fn char_to_byte(text: &str, char_index: usize) -> usize {
    text.char_indices()
        .nth(char_index)
        .map_or(text.len(), |(byte, _)| byte)
}

/// Draw the completion popup at `pos`, returning the index of a clicked row.
fn draw_autocomplete_popup(
    ui: &egui::Ui,
    editor_id: egui::Id,
    pos: egui::Pos2,
    autocomplete: &Autocomplete,
) -> Option<usize> {
    let mut clicked = None;
    // Size the scroll area to exactly five rows. A `selectable_label` is the
    // text height plus its button padding, and rows are separated by the item
    // spacing.
    let spacing = ui.spacing();
    let row_height = ui.text_style_height(&egui::TextStyle::Body)
        + 2.0 * spacing.button_padding.y
        + spacing.item_spacing.y;
    let max_height = row_height * AUTOCOMPLETE_VISIBLE_ROWS as f32;
    let overflow = autocomplete
        .items
        .len()
        .saturating_sub(AUTOCOMPLETE_VISIBLE_ROWS);

    egui::Area::new(editor_id.with("autocomplete"))
        .order(egui::Order::Foreground)
        .fixed_pos(pos)
        .constrain(true)
        .show(ui.ctx(), |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_min_width(220.0);
                ui.set_max_width(380.0);
                egui::ScrollArea::vertical()
                    .max_height(max_height)
                    .show(ui, |ui| {
                        for (index, construct) in autocomplete.items.iter().enumerate() {
                            let selected = index == autocomplete.selected;
                            let response =
                                ui.selectable_label(selected, autocomplete_row(ui, construct));
                            if response.clicked() {
                                clicked = Some(index);
                            }
                            if selected {
                                response.scroll_to_me(None);
                            }
                        }
                    });
                if overflow > 0 {
                    ui.label(
                        egui::RichText::new(format!(
                            "{} {} more below",
                            egui_phosphor::regular::CARET_DOWN,
                            overflow,
                        ))
                        .weak()
                        .small(),
                    );
                }
            });
        });
    clicked
}

/// One popup row: the construct's name in code font, its summary dimmed beside
/// it.
fn autocomplete_row(ui: &egui::Ui, construct: &Construct) -> LayoutJob {
    let mut job = LayoutJob::default();
    job.append(
        construct.name,
        0.0,
        egui::TextFormat {
            font_id: egui::TextStyle::Monospace.resolve(ui.style()),
            color: ui.visuals().strong_text_color(),
            ..Default::default()
        },
    );
    job.append(
        &format!("  {}", construct.summary),
        0.0,
        egui::TextFormat {
            font_id: egui::TextStyle::Body.resolve(ui.style()),
            color: ui.visuals().weak_text_color(),
            italics: true,
            ..Default::default()
        },
    );
    job
}

/// A Rust-doc-style hover: the construct's name and kind, its summary, then -
/// when present - the fuller explanation and example snippets. Language
/// constructs (backticked in the doc, and whole example snippets) are syntax
/// colored the way the editor colors them, rather than shown in raw backticks.
fn construct_tooltip_ui(ui: &mut egui::Ui, construct: &Construct) {
    ui.set_max_width(360.0);
    let mono = egui::TextStyle::Monospace.resolve(ui.style());
    let body = egui::TextStyle::Body.resolve(ui.style());
    let default = ui.visuals().text_color();
    let dark = ui.visuals().dark_mode;

    ui.horizontal(|ui| {
        // The name is itself a construct, so color it like the editor would.
        let mut name = LayoutJob::default();
        append_query_syntax(&mut name, &mono, default, dark, construct.name);
        ui.label(name);
        ui.label(egui::RichText::new(construct.kind.label()).weak().small());
    });
    ui.label(construct.summary);
    if !construct.doc.is_empty() || !construct.examples.is_empty() {
        ui.separator();
    }
    if !construct.doc.is_empty() {
        let mut doc = LayoutJob::default();
        // The doc is prose with `backticked` code spans; color the code.
        let mut in_code = false;
        for part in construct.doc.split('`') {
            if !part.is_empty() {
                if in_code {
                    append_query_syntax(&mut doc, &mono, default, dark, part);
                } else {
                    doc.append(part, 0.0, text_format(&body, default));
                }
            }
            in_code = !in_code;
        }
        doc.wrap.max_width = ui.available_width();
        ui.label(doc);
    }
    for example in construct.examples {
        let mut job = LayoutJob::default();
        append_query_syntax(&mut job, &mono, default, dark, example);
        job.wrap.max_width = ui.available_width();
        ui.label(job);
    }
}

/// A single-color text section for a [`LayoutJob`].
fn text_format(font: &egui::FontId, color: egui::Color32) -> egui::TextFormat {
    egui::TextFormat {
        font_id: font.clone(),
        color,
        ..Default::default()
    }
}

/// The syntax-highlight color for a token class in the current theme; `default`
/// colors whitespace and punctuation. Shared by the editor's layouter and the
/// hover doc so they can't diverge.
fn syntax_color(class: TokenClass, default: egui::Color32, dark_mode: bool) -> egui::Color32 {
    match class {
        TokenClass::Keyword => gt_ui_theme::QUERY_SYNTAX_KEYWORD,
        TokenClass::Number => gt_ui_theme::QUERY_SYNTAX_NUMBER,
        TokenClass::Ident => gt_ui_theme::query_syntax_ident(dark_mode),
        TokenClass::Comment => gt_ui_theme::QUERY_SYNTAX_COMMENT,
        TokenClass::Punctuation => default,
        TokenClass::Error => gt_ui_theme::ERROR_INDICATOR,
    }
}

/// Append `text` to `job` in `font`, coloring query tokens the way the editor
/// does. `default` colors whitespace and punctuation.
fn append_query_syntax(
    job: &mut LayoutJob,
    font: &egui::FontId,
    default: egui::Color32,
    dark_mode: bool,
    text: &str,
) {
    let color = |class| syntax_color(class, default, dark_mode);
    let mut cursor = 0;
    for (span, class) in lexer::highlight_classes(text) {
        // Whitespace between tokens is not covered by a span.
        if let Some(gap) = text.get(cursor..span.start).filter(|g| !g.is_empty()) {
            job.append(gap, 0.0, text_format(font, default));
        }
        if let Some(slice) = text.get(span.start..span.end) {
            job.append(slice, 0.0, text_format(font, color(class)));
        }
        cursor = span.end;
    }
    if let Some(rest) = text.get(cursor..).filter(|r| !r.is_empty()) {
        job.append(rest, 0.0, text_format(font, default));
    }
}

/// The query error's problem as a [`LayoutJob`]: an error icon and red prose,
/// with the quoted token (backticked in the message) lifted out of the red into
/// the code font and italicized so it stands out.
fn error_message_layout(ui: &egui::Ui, message: &str) -> LayoutJob {
    let body = egui::TextStyle::Body.resolve(ui.style());
    let mono = egui::TextStyle::Monospace.resolve(ui.style());
    let mut job = LayoutJob::default();
    job.append(
        &format!("{} ", egui_phosphor::regular::WARNING_OCTAGON),
        0.0,
        text_format(&body, gt_ui_theme::ERROR_INDICATOR),
    );
    let mut in_code = false;
    for part in message.split('`') {
        if !part.is_empty() {
            let format = if in_code {
                egui::TextFormat {
                    font_id: mono.clone(),
                    color: ui.visuals().strong_text_color(),
                    italics: true,
                    ..Default::default()
                }
            } else {
                text_format(&body, gt_ui_theme::ERROR_INDICATOR)
            };
            job.append(part, 0.0, format);
        }
        in_code = !in_code;
    }
    job.wrap.max_width = ui.available_width();
    job
}

/// Display formatting per metric quantity, for match tables. Values arrive
/// in the evaluator's base units.
fn format_value(metric: QueryMetric, value: Option<f64>) -> String {
    let Some(v) = value else {
        return EM_DASH.to_owned();
    };
    match metric.quantity() {
        Quantity::Timestamp => DateTime::<Utc>::from_timestamp_millis((v * 1_000.0) as i64)
            .map_or_else(|| EM_DASH.to_owned(), |t| t.format("%H:%M:%S").to_string()),
        Quantity::Angle | Quantity::Direction => format!("{v:.1}{DEGREE_SIGN}"),
        Quantity::Speed => format!("{:.1} km/h", v * 3.6),
        Quantity::Acceleration => format!("{v:.2} m/s²"),
        Quantity::Length => format!("{v:.1} m"),
        Quantity::Duration => format!("{v:.3} s"),
        Quantity::Count => format!("{v:.0}"),
        Quantity::Ratio => format!("{:.0} %", v / Unit::Percent.to_base()),
        Quantity::Rate => format!("{v:.2}/min"),
        Quantity::Condition => EM_DASH.to_owned(),
    }
}

/// Token-driven syntax highlighting plus the diagnostic underline, built
/// from the same lexer the parser uses.
fn highlight_layout(ui: &egui::Ui, text: &str, diagnostic: Option<Span>) -> LayoutJob {
    let font = egui::TextStyle::Monospace.resolve(ui.style());
    let default_color = ui.visuals().text_color();
    let underline = diagnostic
        .map(|span| (span.start, span.end.max(span.start + 1)))
        .map(|(start, end)| (start, end.min(text.len().max(start))));

    let mut job = LayoutJob::default();
    let mut append = |range: Range<usize>, color: egui::Color32| {
        let Some(slice) = text.get(range.clone()) else {
            return;
        };
        if slice.is_empty() {
            return;
        }
        let underlined =
            underline.is_some_and(|(start, end)| range.start < end && start < range.end);
        let format = egui::TextFormat {
            font_id: font.clone(),
            color,
            underline: if underlined {
                egui::Stroke::new(2.0, gt_ui_theme::ERROR_INDICATOR)
            } else {
                egui::Stroke::NONE
            },
            ..Default::default()
        };
        job.append(slice, 0.0, format);
    };

    // Cut at token boundaries and at the diagnostic edges so the underline
    // starts and ends exactly on the reported span.
    let mut cursor = 0;
    for (span, class) in lexer::highlight_classes(text) {
        for range in segments(cursor..span.start, underline) {
            append(range, default_color);
        }
        let color = syntax_color(class, default_color, ui.visuals().dark_mode);
        for range in segments(span.start..span.end, underline) {
            append(range, color);
        }
        cursor = span.end;
    }
    for range in segments(cursor..text.len(), underline) {
        append(range, default_color);
    }
    job
}

/// Split a byte range at the diagnostic edges so each piece is uniformly
/// underlined or not.
fn segments(range: Range<usize>, underline: Option<(usize, usize)>) -> Vec<Range<usize>> {
    let Some((start, end)) = underline else {
        return vec![range];
    };
    let mut cuts = vec![range.start, range.end];
    for edge in [start, end] {
        if range.start < edge && edge < range.end {
            cuts.push(edge);
        }
    }
    cuts.sort_unstable();
    cuts.windows(2)
        .filter_map(|pair| match pair {
            [a, b] if a < b => Some(*a..*b),
            _ => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every built-in example parses, type-checks, and runs against a real
    /// track - the guard that keeps embedded queries valid as the language
    /// evolves.
    #[test]
    fn examples_parse_check_and_run() {
        let points = gt_test_utils::nav_test_data();
        let provider = provider_for(&points, None);
        let inputs = [TrackInput {
            track: TrackRef::new(FileIdx::new(0), TrackIdx::new(0)),
            provider: &provider,
        }];
        for example in EXAMPLES {
            let parsed = gt_query::parse(example.text).unwrap_or_else(|e| {
                panic!("example {:?} failed to parse: {}", example.name, e.message)
            });
            let checked = gt_query::check(&parsed).unwrap_or_else(|e| {
                panic!("example {:?} failed to check: {}", example.name, e.message)
            });
            // Runs without panicking; util/slip series are absent here, which
            // the poison rules handle, so we only assert it completes.
            let _ = gt_query::run(&checked, &inputs);
        }
    }

    #[test]
    fn examples_have_unique_names() {
        let mut names: Vec<&str> = EXAMPLES.iter().map(|e| e.name).collect();
        names.sort_unstable();
        let count = names.len();
        names.dedup();
        assert_eq!(names.len(), count, "example names must be unique");
    }

    #[test]
    fn history_dedup_moves_to_top_and_keeps_pin() {
        let mut window = QueryWindow::new();
        window.record_run("points | where velocity > 10 km/h");
        window.record_run("points | where eph > 20 m");
        // Pin the older entry, then rerun it.
        window.history[1].pinned = true;
        window.record_run("  points | where velocity > 10 km/h  ");

        assert_eq!(window.history.len(), 2, "rerun deduplicates, not appends");
        assert_eq!(
            window.history[0].text, "  points | where velocity > 10 km/h  ",
            "rerun moves the entry to the top with its new text"
        );
        assert!(window.history[0].pinned, "the pin carries across a rerun");
    }

    #[test]
    fn history_evicts_oldest_unpinned_beyond_cap() {
        let mut window = QueryWindow::new();
        // Pin the very first entry, then overflow the cap.
        window.record_run("points | where sats_fix < 4");
        window.history[0].pinned = true;
        for i in 0..MAX_UNPINNED_HISTORY + 5 {
            window.record_run(&format!("points | where velocity > {i} km/h"));
        }
        let unpinned = window.history.iter().filter(|e| !e.pinned).count();
        assert_eq!(
            unpinned, MAX_UNPINNED_HISTORY,
            "unpinned entries are capped"
        );
        assert!(
            window.history.iter().any(|e| e.pinned),
            "the pinned entry survives eviction"
        );
        assert_eq!(
            window.history.len(),
            MAX_UNPINNED_HISTORY + 1,
            "pinned entries do not count against the cap"
        );
    }

    #[test]
    fn recording_bumps_the_revision_and_skips_blank() {
        let mut window = QueryWindow::new();
        let before = window.history_revision();
        window.record_run("   \n  # just a comment is still text \n  ");
        assert_eq!(
            window.history.len(),
            1,
            "a comment-only query is still text"
        );
        assert!(window.history_revision() > before);

        let after_one = window.history_revision();
        window.record_run("   ");
        assert_eq!(window.history.len(), 1, "blank text is not recorded");
        assert_eq!(window.history_revision(), after_one, "blank does not bump");
    }

    #[test]
    fn history_keeps_comments_verbatim() {
        // Comments are documentation - the stored entry must keep them, even
        // though the compact list label strips them.
        let documented = "# average speed over a 10-point window\npoints\n| window 10\n| where avg(velocity) > 30 km/h # only when moving";
        let mut window = QueryWindow::new();
        window.record_run(documented);
        assert_eq!(
            window.history()[0].text,
            documented,
            "the stored query keeps its comments"
        );
        assert!(
            !query_one_line(documented).contains('#'),
            "only the compact label drops comments"
        );
    }

    #[test]
    fn history_age_omits_seconds() {
        use chrono::Duration;
        assert_eq!(format_history_age(Duration::seconds(0)), "now");
        assert_eq!(format_history_age(Duration::seconds(59)), "now");
        assert_eq!(format_history_age(Duration::seconds(90)), "1m ago");
        assert_eq!(format_history_age(Duration::minutes(59)), "59m ago");
        assert_eq!(format_history_age(Duration::minutes(90)), "1h ago");
        assert_eq!(format_history_age(Duration::hours(25)), "1d ago");
        // A clock skew putting the run "in the future" reads as now.
        assert_eq!(format_history_age(Duration::seconds(-5)), "now");
    }

    #[test]
    fn query_one_line_flattens_drops_comments_and_elides() {
        assert_eq!(
            query_one_line("points\n| window 10 # every ten\n| draw"),
            "points | window 10 | draw"
        );
        assert_eq!(query_one_line("# only a comment\n\n"), "");
        let long = format!("points | where velocity > {} km/h", "9".repeat(60));
        let shown = query_one_line(&long);
        assert!(
            shown.ends_with(ELLIPSIS),
            "over-long lines are elided with dots"
        );
        assert_eq!(shown.chars().count(), HISTORY_LINE_MAX_CHARS + 1);
    }

    #[test]
    fn segments_split_exactly_at_diagnostic_edges() {
        assert_eq!(segments(0..10, None), vec![0..10]);
        assert_eq!(segments(0..10, Some((3, 7))), vec![0..3, 3..7, 7..10]);
        assert_eq!(segments(4..6, Some((3, 7))), vec![4..6]);
        assert_eq!(segments(0..5, Some((3, 9))), vec![0..3, 3..5]);
        assert_eq!(segments(5..5, Some((3, 9))), Vec::<Range<usize>>::new());
    }

    #[test]
    fn values_format_by_quantity_in_base_units() {
        assert_eq!(format_value(QueryMetric::Velocity, Some(10.0)), "36.0 km/h");
        assert_eq!(format_value(QueryMetric::Heading, Some(271.53)), "271.5°");
        assert_eq!(format_value(QueryMetric::SatsFix, Some(7.0)), "7");
        assert_eq!(format_value(QueryMetric::UtilGps, Some(0.5)), "50 %");
        assert_eq!(format_value(QueryMetric::SlipAll, Some(2.0)), "2.00/min");
        assert_eq!(format_value(QueryMetric::Eph, None), EM_DASH);
    }

    /// One point with a satellite report, one without, exercising the
    /// provider's unit conversions and count folds directly.
    fn test_points() -> Vec<NavPoint> {
        use gt_types::coordinates::{Latitude, Longitude};
        use gt_types::satellites::{Satellite, Satellites};
        use gt_types::time_types::GpsTime;
        use gt_types::tpv::TimePositionVelocity;
        use uom::si::angle::degree;
        use uom::si::f64::{Angle, Velocity};
        use uom::si::velocity::kilometer_per_hour;

        let time = |secs: i64| {
            GpsTime::from_utc(
                chrono::DateTime::from_timestamp(1_700_000_000 + secs, 0).expect("valid"),
            )
        };
        let sat = |constellation, in_fix| {
            Satellite::new(constellation, 1, Some(45.0), None, Some(40.0), in_fix)
        };
        let with_sats = TimePositionVelocity::builder()
            .time(time(0))
            .lat(Latitude::new(55.5))
            .lon(Longitude::new(12.25))
            .velocity(Velocity::new::<kilometer_per_hour>(36.0))
            .heading(Angle::new::<degree>(90.0))
            .eph_m(2.5)
            .build();
        let bare = TimePositionVelocity::builder()
            .time(time(1))
            .lat(Latitude::new(55.6))
            .lon(Longitude::new(12.35))
            .build();
        vec![
            NavPoint::new(
                with_sats,
                Some(Satellites::new(
                    Some(time(0)),
                    None,
                    vec![
                        sat(Constellation::Gps, true),
                        sat(Constellation::Gps, false),
                        sat(Constellation::Galileo, true),
                    ],
                )),
            ),
            NavPoint::new(bare, None),
        ]
    }

    #[test]
    fn provider_maps_metrics_to_base_units() {
        let points = test_points();
        let util = UtilPerPoint {
            gps: vec![Some(50.0), None],
            ..UtilPerPoint::default()
        };
        let slip = SlipRatePerPoint {
            all: vec![Some(2.0), None],
            ..SlipRatePerPoint::default()
        };
        let data = TrackQueryData {
            util: Some(util),
            slip: Some(slip),
            slice_start: 0,
        };
        let provider = provider_for(&points, Some(&data));

        // (metric, point index, expected base-unit value)
        let cases = [
            (QueryMetric::Lat, 0, Some(55.5)),
            (QueryMetric::Lon, 0, Some(12.25)),
            (QueryMetric::Velocity, 0, Some(10.0)), // 36 km/h in m/s
            (QueryMetric::Heading, 0, Some(90.0)),
            (QueryMetric::Eph, 0, Some(2.5)),
            (QueryMetric::SatsSeen, 0, Some(3.0)),
            (QueryMetric::SatsFix, 0, Some(2.0)),
            (QueryMetric::GpsSeen, 0, Some(2.0)),
            (QueryMetric::GpsFix, 0, Some(1.0)),
            (QueryMetric::GalileoFix, 0, Some(1.0)),
            (QueryMetric::BeidouSeen, 0, Some(0.0)),
            (QueryMetric::UtilGps, 0, Some(0.5)), // 50 % as a fraction
            (QueryMetric::SlipAll, 0, Some(2.0)), // already per minute
            // The reportless point: counts and derived series are missing,
            // never zero.
            (QueryMetric::Velocity, 1, None),
            (QueryMetric::SatsSeen, 1, None),
            (QueryMetric::GpsSeen, 1, None),
            (QueryMetric::UtilGps, 1, None),
            (QueryMetric::SlipAll, 1, None),
        ];
        for (metric, index, expected) in cases {
            let value = provider.value(metric, index);
            match expected {
                Some(want) => {
                    let got = value.unwrap_or_else(|| panic!("{metric} at {index} missing"));
                    assert!(
                        (got - want).abs() < 1e-9,
                        "{metric} at {index}: {got} != {want}"
                    );
                }
                None => assert_eq!(value, None, "{metric} at {index}"),
            }
        }
        assert_eq!(provider.len(), 2);
    }

    #[test]
    fn slice_provider_offsets_and_bounds() {
        let points = test_points();
        let slice = SliceProvider {
            inner: provider_for(&points, None),
            start: 1,
            len: 1,
        };
        assert_eq!(slice.len(), 1);
        // Index 0 of the slice is point 1 of the track (the bare point).
        assert_eq!(slice.value(QueryMetric::Lat, 0), Some(55.6));
        assert_eq!(slice.value(QueryMetric::Lat, 1), None, "out of the slice");
    }

    #[test]
    fn fingerprint_changes_with_files_visibility_and_filter() {
        let loaded_files = gt_loaded_files::LoadedFiles::new();
        let visibility = TrackDataVisibility::from_loaded(loaded_files.files());
        let base = current_fingerprint(loaded_files.view(), &visibility, &GlobalFilter::default());
        assert_eq!(
            base,
            current_fingerprint(loaded_files.view(), &visibility, &GlobalFilter::default())
        );
        let filtered = GlobalFilter {
            min_distance_km: Some(uom::si::f64::Length::new::<uom::si::length::kilometer>(1.0)),
            ..GlobalFilter::default()
        };
        assert_ne!(
            base,
            current_fingerprint(loaded_files.view(), &visibility, &filtered)
        );
    }

    #[test]
    fn summary_reports_skips_and_unused_params() {
        let query = gt_query::check(
            &gt_query::parse("points | with mask 15 deg, snr_drop 10 | where util_all < 50 %")
                .expect("parses"),
        )
        .expect("checks");
        let provider = EmptyProvider { len: 3 };
        let output = gt_query::run(
            &query,
            &[TrackInput {
                track: TrackRef::new(FileIdx::new(0), TrackIdx::new(0)),
                provider: &provider,
            }],
        );
        let line = summary_line(&output, DisplayMode::Draw);
        assert_eq!(
            line,
            format!(
                "0 matches on 0 tracks {EM_DASH} 3 skipped (missing util_all) \
                 {EM_DASH} snr_drop declared but unused"
            )
        );
    }

    #[test]
    fn summary_reports_hidden_count_for_keep_and_hide() {
        // 5 points, 2 matched.
        let query = gt_query::check(&gt_query::parse("points | where velocity > 30 km/h").unwrap())
            .unwrap();
        let provider = TestSpeeds(vec![
            Some(40.0),
            Some(40.0),
            Some(1.0),
            Some(1.0),
            Some(1.0),
        ]);
        let output = gt_query::run(
            &query,
            &[TrackInput {
                track: TrackRef::new(FileIdx::new(0), TrackIdx::new(0)),
                provider: &provider,
            }],
        );
        // keep hides the 3 non-matching points; hide hides the 2 matching.
        assert!(summary_line(&output, DisplayMode::Keep).contains("3 of 5 points hidden"));
        assert!(summary_line(&output, DisplayMode::Hide).contains("2 of 5 points hidden"));
        assert!(!summary_line(&output, DisplayMode::Draw).contains("hidden"));
    }

    /// Velocity in m/s per point, everything else missing.
    struct TestSpeeds(Vec<Option<f64>>);

    impl MetricProvider for TestSpeeds {
        fn len(&self) -> usize {
            self.0.len()
        }

        fn value(&self, metric: QueryMetric, index: usize) -> Option<f64> {
            match metric {
                QueryMetric::Velocity => self.0.get(index).copied().flatten(),
                _ => None,
            }
        }
    }

    struct EmptyProvider {
        len: usize,
    }

    impl MetricProvider for EmptyProvider {
        fn len(&self) -> usize {
            self.len
        }

        fn value(&self, _metric: QueryMetric, _index: usize) -> Option<f64> {
            None
        }
    }
}

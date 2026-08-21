//! The query window: a small pipeline language for ad-hoc analysis of the
//! loaded data. Editor with syntax highlighting, run on the currently
//! visible tracks, and a results area whose matches also draw on the map as
//! halos.

use egui::{
    Area, Button, CollapsingHeader, Frame, Grid, Label, RichText, ScrollArea, TextEdit, Window,
};
use egui_phosphor::regular::ARROWS_IN as ICON_ARROWS_IN;
use egui_phosphor::regular::ARROWS_OUT as ICON_ARROWS_OUT;
use egui_phosphor::regular::CARET_DOWN as ICON_CARET_DOWN;
use egui_phosphor::regular::COPY as ICON_COPY;
use egui_phosphor::regular::CROSSHAIR as ICON_CROSSHAIR;
use egui_phosphor::regular::PUSH_PIN as ICON_PUSH_PIN;
use egui_phosphor::regular::TRASH as ICON_TRASH;
use egui_phosphor::regular::WARNING_OCTAGON as ICON_WARNING_OCTAGON;
use egui_phosphor::regular::X as ICON_X;
use std::num::NonZeroUsize;
use std::ops::Range;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use chrono::{DateTime, Utc};
use egui::text::{CCursor, CCursorRange, LayoutJob};
use gt_query::lexer::{self, TokenClass};
use gt_query::{ChannelSchema, CompletionTrigger, Construct, ConstructKind, Diagnostic, Span};
use gt_query_run::{
    ChannelResults, ChannelTrackResult, CheckRefresh, PanelQuery, PointsResults, QuerySession,
    RunInputs, RunKind, RunOutcome, RunResults, schema_from_files,
};
use gt_side_panel::widgets::PointClickRequests;
use gt_types::{LoadedFile, NavPoint, TrackRef};
use gt_ui_theme::{EM_DASH, MIDDLE_DOT, RIGHTWARDS_ARROW};
use gt_ui_types::{DisplayMask, MapHighlight, MapScope, MatchRevealTarget, QueryMatches};

use crate::settings::QueryHistoryEntry;

use self::column_format::ColumnFormat;
use self::match_table::{FoldedMatches, MatchTableContext, MatchTableOutputs};

mod column_format;
mod match_table;

/// Samples shown per channel table before truncating with a "more samples"
/// note.
const CHANNEL_SAMPLE_ROW_CAP: usize = 100;

/// Unpinned history entries kept before the oldest is evicted. Pinned
/// entries never count against this cap.
const MAX_UNPINNED_HISTORY: usize = 50;

/// Characters of a history entry's first line shown before eliding.
const HISTORY_LINE_MAX_CHARS: NonZeroUsize = match NonZeroUsize::new(48) {
    Some(chars) => chars,
    None => NonZeroUsize::MIN,
};

/// Width the query window opens at. It grows only when the user drags it
/// wider: nothing inside it may widen it, or the window covers the map.
pub(crate) const DEFAULT_WINDOW_WIDTH: f32 = 460.0;

/// Max width of an editor hover tooltip, shared by the construct and channel
/// tooltips so they stay the same size.
const TOOLTIP_MAX_WIDTH: f32 = 360.0;

/// Id salt for the query editor's text field. Fixed (not derived from the
/// enclosing `Ui`) so the autocomplete caret/focus integration - and the UI
/// snapshot test - can address the widget directly.
pub(crate) const EDITOR_ID_SALT: &str = "query_editor";

/// Candidate rows the autocomplete popup shows before it scrolls. A footer
/// notes how many more there are.
const AUTOCOMPLETE_VISIBLE_ROWS: usize = 5;

/// Seconds the pointer must rest before the editor hover doc appears, so the
/// tooltip does not flicker over every token the pointer crosses. Only entering
/// a token arms the delay. The doc already on display survives pointer motion
/// within its token.
const HOVER_DOC_DELAY_SECS: f32 = 0.15;

/// Seconds after the last keystroke before the caret chunk's diagnostic shows.
/// A query is structurally broken for most of the time it is being typed
/// (`points |` until the keyword lands). Flashing red on every keystroke reads
/// as noise, so the chunk under the caret gets this grace period. Errors in
/// other chunks (and the disabled Run button) are immediate.
const DIAGNOSTIC_IDLE_SECS: f64 = 0.6;

/// A built-in query offered in the examples list.
struct QueryExample {
    name: &'static str,
    description: &'static str,
    text: &'static str,
}

/// Starter queries, mirroring the documented use cases. Embedded, not
/// persisted. Every one is asserted to parse, check, and run by a test.
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

/// The floating query window: the editor, the results panel, and the query
/// history list around a [`QuerySession`].
pub struct QueryWindow {
    pub open: bool,
    /// The text, its checks, the run in flight and the last results - the whole
    /// run lifecycle, free of egui.
    session: QuerySession,
    /// Receives the run in flight from the worker thread. Set and cleared
    /// together with the session's in-flight run, so the two never disagree.
    worker: Option<mpsc::Receiver<RunOutcome>>,
    /// Set by the Run button, consumed at the end of `show`.
    run_requested: bool,
    /// Set by the Cancel button while a run is in flight.
    cancel_requested: bool,
    /// Previously run queries, newest first. Persisted in settings.
    history: Vec<QueryHistoryEntry>,
    /// Bumped on every history mutation so the config dirty-check (which
    /// compares a flat snapshot and cannot see into a growing `Vec`) notices
    /// and flushes.
    history_revision: u64,
    /// The editor's autocomplete popup, recomputed from the caret when the
    /// text, schema, or caret changed.
    autocomplete: Autocomplete,
    /// Bumped whenever the checked text or schema changes. Keys the
    /// autocomplete memo so candidates are not recomputed every repaint.
    assist_revision: u64,
    /// `ui.input(..).time` of the last text edit, for the diagnostic grace
    /// period on the chunk being typed in.
    last_edit_time: Option<f64>,
    /// Whether the editor had keyboard focus last frame.
    ///
    /// egui surrenders a widget's focus on Escape before that widget renders,
    /// so every Escape handler in this window reads last frame's state: live
    /// focus is already false on the frame the key arrives.
    editor_had_focus: bool,
    /// Editor-global byte span of the token whose hover doc is on display,
    /// `None` while no doc shows. Keeps the doc up while the pointer moves
    /// within the token.
    hover_doc_span: Option<Range<usize>>,
    /// The matches whose point rows are folded away in the results. Keyed by
    /// track and first point, so a rerun over unchanged data folds the same
    /// matches it folded before.
    folded_matches: FoldedMatches,
}

/// One completion offered in the popup: a language construct or a loaded
/// channel, reduced to what the popup needs. A construct and a channel render
/// and insert the same way, so the popup holds one type for both.
#[derive(Clone)]
struct Candidate {
    /// The name shown in the popup and inserted when accepted (`velocity`,
    /// `@accel`).
    insert: String,
    /// The dimmed one-line summary shown beside the name.
    summary: String,
    /// Appended after `insert` on acceptance: a trailing space where something
    /// always follows (stages, params, the source, connectives), `()` for a
    /// function.
    suffix: &'static str,
    /// Bytes the caret steps back into the suffix, so accepting `avg` lands
    /// the caret inside the inserted `()`.
    caret_back: usize,
    /// Whether a separating space is inserted when the replaced range starts
    /// directly after a digit: accepting a unit at the caret in `30` writes
    /// `30 km/h`, the way every example and doc spells it.
    pad_after_digit: bool,
}

impl Candidate {
    fn from_construct(construct: &Construct) -> Self {
        let (suffix, caret_back) = match construct.kind {
            ConstructKind::Function => ("()", 1),
            // Something always follows these, so land the caret past a space.
            ConstructKind::Source
            | ConstructKind::Stage
            | ConstructKind::Param
            | ConstructKind::Connective => (" ", 0),
            // A metric leads into an operator, a unit into `|` or a
            // connective, a mode into nothing - no suffix.
            ConstructKind::Mode | ConstructKind::Metric | ConstructKind::Unit => ("", 0),
        };
        Self {
            insert: construct.name.to_owned(),
            summary: construct.summary.to_owned(),
            suffix,
            caret_back,
            pad_after_digit: construct.kind == ConstructKind::Unit,
        }
    }

    fn from_channel(channel: gt_query::ChannelSuggestion) -> Self {
        Self {
            insert: format!("@{}", channel.name),
            summary: channel.summary,
            suffix: "",
            caret_back: 0,
            pad_after_digit: false,
        }
    }
}

/// What the editor hover shows under the pointer: a loaded channel or a
/// language construct.
enum HoverDoc {
    Channel(gt_query::ChannelSuggestion),
    Construct(&'static Construct),
}

/// The editor's autocomplete popup state.
///
/// Recomputed from the caret by [`QueryWindow::update_autocomplete`] and drawn
/// under the caret. Its key handling runs at the *start* of the next frame
/// ([`QueryWindow::apply_autocomplete_input`]) so it can claim keys before the
/// text editor consumes them.
///
/// The popup claims keys in two grades. *Active* - the user typed a prefix or
/// requested completion (Ctrl+Space) - claims Enter, Tab, and the arrow keys.
/// *Passive* - an eager empty-prefix offer, like the units after a number -
/// claims only Tab (accept) and Esc (dismiss), so Enter still breaks the line
/// and the arrows still move the caret through a multi-line query.
#[derive(Default)]
struct Autocomplete {
    /// Candidates for the caret as of the last frame, best first. Empty when
    /// the popup is not shown.
    items: Vec<Candidate>,
    /// Byte range of the partial word an accepted candidate replaces.
    range: Range<usize>,
    /// The text under `range` when the candidates were computed. Acceptance
    /// re-validates it, so a same-frame edit (key repeat, paste) can never
    /// splice a candidate over the wrong span.
    word: String,
    /// Whether the popup claims Enter and the arrow keys (see type docs).
    active: bool,
    /// The highlighted row.
    selected: usize,
    /// Screen position for the popup (just below the caret), cached so the
    /// popup can still be drawn on the frame a click steals the editor's focus.
    caret_pos: egui::Pos2,
    /// The caret's top edge, for flipping the popup above the caret when there
    /// is no room below.
    caret_top: egui::Pos2,
    /// Byte position (`range.start`) at which Esc dismissed the popup: it
    /// stays closed while completing the same word and re-arms when the caret
    /// moves on to another one.
    dismissed_at: Option<usize>,
    /// Set by Ctrl+Space. The next recompute runs with the manual trigger,
    /// which offers candidates even on an empty prefix.
    manual_request: bool,
    /// A non-interactive explanation row shown instead of candidates (typing
    /// `@` with no channels loaded).
    notice: Option<&'static str>,
    /// Memo key of the last candidate computation: the window's assist
    /// revision and the caret byte. Unchanged key, unchanged candidates.
    computed_for: Option<(u64, usize)>,
    /// Whether the popup was drawn last frame. Key handling reads this rather
    /// than live focus, for the reason on [`QueryWindow::editor_had_focus`].
    shown: bool,
}

impl QueryWindow {
    pub fn new() -> Self {
        Self {
            session: QuerySession::new(),
            worker: None,
            open: false,
            run_requested: false,
            cancel_requested: false,
            history: Vec::new(),
            history_revision: 0,
            autocomplete: Autocomplete::default(),
            assist_revision: 0,
            last_edit_time: None,
            editor_had_focus: false,
            hover_doc_span: None,
            folded_matches: FoldedMatches::default(),
        }
    }

    /// Matches of the last run, for the map. `None` when there was no run.
    pub fn matches(&self) -> Option<&QueryMatches> {
        self.session.matches()
    }

    /// Whether the queries are currently affecting the map (any hidden points
    /// or halos). Drives the toolbar indicator shown while the window is closed.
    pub fn filter_active(&self) -> bool {
        self.matches().is_some_and(|matches| !matches.is_empty())
    }

    /// The byte offset of the editor caret, from the text edit's stored state.
    /// Zero before the editor has ever been focused.
    fn caret_byte(&self, ctx: &egui::Context, editor_id: egui::Id) -> usize {
        let caret_char = TextEdit::load_state(ctx, editor_id)
            .and_then(|state| state.cursor.char_range())
            .map_or(egui::text::CharIndex::ZERO, |range| range.primary.index);
        char_to_byte(self.session.text(), caret_char)
    }

    /// Drop the last run's results so the map returns to normal, abandoning any
    /// run still in flight. Called by the toolbar's clear action and by the
    /// side panel's "Reset filters".
    pub fn clear_filter(&mut self) {
        // Dropping the receiver detaches the worker and discards its outcome.
        self.worker = None;
        self.session.clear_results();
    }

    /// Replace the editor text, e.g. when loading a history entry or an
    /// example. Never runs - running stays an explicit action.
    pub fn set_text(&mut self, text: String) {
        self.session.set_text(text);
    }

    /// The current editor text (used by tests to observe loads).
    #[cfg(test)]
    pub fn text(&self) -> &str {
        self.session.text()
    }

    /// The insertions currently offered by the autocomplete popup, best first
    /// (used by the UI test to assert the popup's contents).
    #[cfg(test)]
    pub fn autocomplete_names(&self) -> Vec<String> {
        self.autocomplete
            .items
            .iter()
            .map(|c| c.insert.clone())
            .collect()
    }

    /// Whether an editor hover doc is on display (used by the UI test to
    /// observe the popup across pointer motion).
    #[cfg(test)]
    pub fn hover_doc_shown(&self) -> bool {
        self.hover_doc_span.is_some()
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
        inputs: RunInputs<'_>,
        display_mask: DisplayMask,
        highlight: &mut MapHighlight,
        requests: &mut PointClickRequests<'_>,
        reveal_matches_request: &mut Option<MatchRevealTarget>,
    ) {
        let RunInputs { loaded_files, .. } = inputs;
        // Collect a finished worker even while the window is closed, so its
        // results are there on reopen.
        self.drain_completed();

        if !self.open {
            return;
        }

        // Results gray out when anything they depend on changed: loaded
        // files, track visibility, or the global filter.
        self.session.refresh_staleness(inputs);

        let files = loaded_files.files();
        // The channels the editor checks `@name` against, gathered across every
        // loaded track.
        let schema = schema_from_files(files);
        // Read before the window renders: `editor_ui` updates the field.
        let editor_was_focused = self.editor_had_focus;
        let mut open = self.open;
        Window::new("Query")
            .open(&mut open)
            .default_width(DEFAULT_WINDOW_WIDTH)
            .default_height(520.0)
            .resizable(true)
            .vscroll(true)
            .show(ctx, |ui| {
                self.editor_ui(ui, &schema);
                ui.separator();
                self.results_ui(
                    ui,
                    inputs,
                    display_mask,
                    highlight,
                    requests,
                    reveal_matches_request,
                );
                ui.separator();
                self.history_examples_ui(ui);
            });

        // Esc closes the window. With the editor focused, the first Esc only
        // unfocuses it (the completion popup, when open, consumes its own Esc
        // before this).
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
            && !editor_was_focused
        {
            open = false;
        }
        if !open {
            self.editor_had_focus = false;
        }
        self.open = open;

        // Ctrl+Enter (Cmd+Enter on macOS) runs, mirroring the Run button.
        // Consumed only while the window is open, so it never steals the
        // chord from other widgets.
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::Enter))
            && self.session.all_ok()
            && self.session.run_kind() != RunKind::MixedChannel
            && !self.session.run_in_flight()
        {
            self.run_requested = true;
        }

        if self.cancel_requested {
            self.cancel_requested = false;
            self.session.cancel_run();
        }
        // The run button lives inside `editor_ui` and sets this flag. One
        // run at a time: the button is disabled while one is in flight.
        if self.run_requested {
            self.run_requested = false;
            if !self.session.run_in_flight() {
                let text = self.session.text().to_owned();
                self.record_run(&text);
                self.spawn_run(ctx, inputs);
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

        CollapsingHeader::new("Query history")
            .default_open(false)
            .show(ui, |ui| {
                if self.history.is_empty() {
                    ui.label(RichText::new("No queries run yet").weak());
                    return;
                }
                if ui
                    .small_button(ICON_TRASH)
                    .on_hover_text("Clear the query history (pinned queries are kept)")
                    .clicked()
                {
                    clear_history = true;
                }
                // A table so the age and remove columns line up across rows.
                Grid::new(ui.id().with("query_history_grid"))
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
                                .selectable_label(entry.pinned, ICON_PUSH_PIN)
                                .on_hover_text(pin_hover)
                                .clicked()
                            {
                                toggle_pin = Some(index);
                            }
                            // The button flattens the query and drops
                            // comments. Its hover shows the full verbatim text
                            // (comments included). Loading restores that text
                            // unchanged.
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
                            ui.label(RichText::new(age).weak());
                            if ui
                                .small_button(ICON_X)
                                .on_hover_text("Remove from history")
                                .clicked()
                            {
                                delete = Some(index);
                            }
                            ui.end_row();
                        }
                    });
            });

        CollapsingHeader::new("Examples")
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

    /// Collect the worker's outcome, if any. A cancelled run keeps the previous
    /// results - partial output is never shown.
    fn drain_completed(&mut self) {
        let Some(rx) = &self.worker else {
            return;
        };
        match rx.try_recv() {
            Ok(outcome) => {
                self.worker = None;
                self.session.finish_run(outcome);
            }
            Err(mpsc::TryRecvError::Empty) => {}
            // The worker is gone without a message, so nothing to keep.
            Err(mpsc::TryRecvError::Disconnected) => {
                log::error!("query worker disappeared without completing");
                self.worker = None;
                self.session.abandon_run();
            }
        }
    }

    fn editor_ui(&mut self, ui: &mut egui::Ui, schema: &ChannelSchema) {
        let editor_id = egui::Id::new(EDITOR_ID_SALT);
        // Runs before the editor so the open popup can claim its keys, and may
        // edit the text (accepting a candidate) - so re-check after.
        self.apply_autocomplete_input(ui, editor_id);

        match self.session.sync_checks(schema) {
            CheckRefresh::Unchanged => {}
            CheckRefresh::SchemaChanged => self.assist_revision += 1,
            CheckRefresh::TextChanged => {
                self.assist_revision += 1;
                self.last_edit_time = Some(ui.input(|i| i.time));
            }
        }

        // Every failed chunk surfaces: each gets an underline and a message
        // line, so an error in query 3 is never hidden behind one in query 1.
        // The chunk being typed in gets a grace period instead of flashing
        // red under every keystroke.
        let now = ui.input(|i| i.time);
        let in_grace = self.editor_had_focus
            && self
                .last_edit_time
                .is_some_and(|at| now - at < DIAGNOSTIC_IDLE_SECS);
        let caret_byte = self.caret_byte(ui.ctx(), editor_id);
        let mut errors: Vec<Diagnostic> = Vec::new();
        let mut underlines: Vec<Span> = Vec::new();
        let mut suppressed = false;
        for chunk in self.session.chunks() {
            let Err(diagnostic) = &chunk.result else {
                continue;
            };
            if in_grace && chunk.range.start <= caret_byte && caret_byte <= chunk.range.end {
                suppressed = true;
                continue;
            }
            errors.push(diagnostic.clone());
            underlines.push(Span::new(
                diagnostic.span.start + chunk.range.start,
                diagnostic.span.end + chunk.range.start,
            ));
        }
        if suppressed {
            // Repaint when the grace period lapses so the error appears
            // without further input.
            ui.ctx().request_repaint_after(Duration::from_millis(100));
        }
        // A channel source mixed with other queries cannot run even though
        // every chunk checks green. Surface it like a check error: underline
        // the channel sources, message below.
        let mixed = self.session.all_ok() && self.session.run_kind() == RunKind::MixedChannel;
        if mixed {
            underlines.extend(self.session.channel_source_spans());
        }

        let mut layouter = |ui: &egui::Ui, buf: &dyn egui::TextBuffer, wrap_width: f32| {
            let mut job = highlight_layout(ui, buf.as_str(), &underlines);
            job.wrap.max_width = wrap_width;
            ui.fonts_mut(|f| f.layout_job(job))
        };
        let output = TextEdit::multiline(self.session.text_mut())
            .id(editor_id)
            .code_editor()
            .desired_rows(5)
            .desired_width(f32::INFINITY)
            .hint_text("points | where velocity > 30 km/h")
            .layouter(&mut layouter)
            .show(ui);
        self.editor_had_focus = output.response.has_focus();

        self.update_autocomplete(ui, editor_id, schema, &output);
        self.hover_docs(ui, schema, &output);

        for diagnostic in &errors {
            // The message shows in red with an error icon (the quoted token
            // lifts into the code font). The fix, held in the structured
            // `help`, is a plain "Hint:" line below.
            ui.label(error_message_layout(ui, &diagnostic.message));
            if let Some(hint) = &diagnostic.help {
                ui.label(format!("Hint: {hint}"));
            }
        }
        if mixed {
            ui.label(error_message_layout(
                ui,
                "a channel-source query (`@name | \u{2026}`) must be the only query in the editor",
            ));
        }

        ui.horizontal(|ui| {
            let in_flight = self.session.run_in_flight();
            let all_ok = self.session.all_ok();
            let mixed = all_ok && self.session.run_kind() == RunKind::MixedChannel;
            let runnable = all_ok && !in_flight && !mixed;
            let run = ui.add_enabled(runnable, Button::new("Run"));
            let run = match (all_ok, in_flight, mixed) {
                (false, _, _) if self.session.chunks().is_empty() => {
                    run.on_disabled_hover_text("Type a query to run")
                }
                (false, _, _) => run.on_disabled_hover_text("Fix the error above to run"),
                (_, _, true) => run.on_disabled_hover_text(
                    "A channel-source query must be the only query in the editor",
                ),
                (true, true, _) => run.on_disabled_hover_text("A run is in progress"),
                (true, false, false) => run,
            };
            if run.clicked() {
                self.run_requested = true;
            }

            let cancel = ui.add_enabled(in_flight, Button::new("Cancel"));
            let cancel = if in_flight {
                cancel
            } else {
                cancel.on_disabled_hover_text("No run in progress")
            };
            if cancel.clicked() {
                self.cancel_requested = true;
            }

            let clearable = !self.session.text().is_empty();
            let clear = ui.add_enabled(clearable, Button::new("Clear"));
            let clear = if clearable {
                clear
            } else {
                clear.on_disabled_hover_text("The editor is already empty")
            };
            if clear.clicked() {
                self.session.text_mut().clear();
                self.autocomplete = Autocomplete::default();
            }

            if let Some(progress) = self.session.progress() {
                ui.spinner();
                if progress.tracks_prepared < progress.track_total {
                    ui.label(format!(
                        "Preparing {}/{} tracks",
                        progress.tracks_prepared, progress.track_total
                    ));
                } else {
                    ui.label("Evaluating");
                }
                // Keep repainting so progress and completion show promptly.
                ui.ctx().request_repaint_after(Duration::from_millis(100));
            }
        });
    }

    /// Handle keyboard input for the autocomplete popup, using the candidates
    /// computed last frame. Called before the editor renders so the popup can
    /// consume keys the text field would otherwise take. (Mouse clicks are
    /// handled inline in `update_autocomplete`.)
    ///
    /// A passive popup (empty prefix, opened eagerly) claims only Tab and Esc:
    /// Enter must still break the line and the arrows must still move the
    /// caret, or the popup hijacks ordinary editing. Typing a prefix or
    /// pressing Ctrl+Space makes it active, claiming Enter and the arrows too.
    fn apply_autocomplete_input(&mut self, ui: &egui::Ui, editor_id: egui::Id) {
        // Ctrl+Space (Cmd+Space on macOS) requests completion at the caret,
        // the explicit counterpart to the automatic popup.
        if self.editor_had_focus
            && ui.input_mut(|input| input.consume_key(egui::Modifiers::COMMAND, egui::Key::Space))
        {
            self.autocomplete.manual_request = true;
            self.autocomplete.dismissed_at = None;
        }

        // Key state is read inside `input_mut`, and the follow-up (focus, text
        // edits) after it: re-entering the context lock inside would deadlock.
        let mut accept = None;
        let mut dismissed = false;
        if self.autocomplete.shown && !self.autocomplete.items.is_empty() {
            let len = self.autocomplete.items.len();
            let active = self.autocomplete.active;
            ui.input_mut(|input| {
                if active {
                    if input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown) {
                        self.autocomplete.selected = (self.autocomplete.selected + 1) % len;
                    }
                    if input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp) {
                        self.autocomplete.selected = (self.autocomplete.selected + len - 1) % len;
                    }
                }
                if input.consume_key(egui::Modifiers::NONE, egui::Key::Escape) {
                    dismissed = true;
                    return;
                }
                // Tab always accepts, Enter only on an active popup.
                // Ctrl+Enter has the COMMAND modifier, so it is left for the
                // window's run shortcut.
                if input.consume_key(egui::Modifiers::NONE, egui::Key::Tab)
                    || (active && input.consume_key(egui::Modifiers::NONE, egui::Key::Enter))
                {
                    accept = Some(self.autocomplete.selected);
                }
            });
        } else if self.autocomplete.shown && self.autocomplete.notice.is_some() {
            // A notice-only popup has nothing to accept, Esc just closes it.
            if ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
                dismissed = true;
            }
        }

        if dismissed {
            self.autocomplete.dismissed_at = Some(self.autocomplete.range.start);
            self.autocomplete.items.clear();
            self.autocomplete.notice = None;
            self.autocomplete.shown = false;
            // Restore the focus Escape dropped, so dismissing the popup keeps
            // the caret in the editor.
            ui.ctx().memory_mut(|m| m.request_focus(editor_id));
            return;
        }

        if let Some(index) = accept
            && let Some(candidate) = self.autocomplete.items.get(index).cloned()
        {
            self.accept_completion(ui, editor_id, &candidate);
        }
    }

    /// Replace the partial word under the caret with `candidate`, then move the
    /// caret past the insertion (or inside a function's inserted `()`) and
    /// close the popup for that word.
    fn accept_completion(&mut self, ui: &egui::Ui, editor_id: egui::Id, candidate: &Candidate) {
        let range = self.autocomplete.range.clone();
        // The candidates were computed for the word recorded alongside them.
        // A same-frame edit (key repeat, paste) may have moved or replaced it,
        // in which case accepting would splice the wrong span, so do nothing.
        if self.session.text().get(range.clone()) != Some(self.autocomplete.word.as_str()) {
            return;
        }
        // A unit accepted directly after a digit gets a separating space:
        // `30` + `km/h` reads `30 km/h`, the way the docs write it.
        let pad = candidate.pad_after_digit
            && range
                .start
                .checked_sub(1)
                .and_then(|i| self.session.text().as_bytes().get(i))
                .is_some_and(u8::is_ascii_digit);
        let space = if pad { " " } else { "" };
        let insertion = format!("{space}{}{}", candidate.insert, candidate.suffix);
        let caret_byte = range.start + insertion.len() - candidate.caret_back;
        self.session.text_mut().replace_range(range, &insertion);

        let caret_char = self
            .session
            .text()
            .get(..caret_byte)
            .map_or(0, |s| s.chars().count());
        let mut state = TextEdit::load_state(ui.ctx(), editor_id).unwrap_or_default();
        state
            .cursor
            .set_char_range(Some(CCursorRange::one(CCursor::new(caret_char))));
        TextEdit::store_state(ui.ctx(), editor_id, state);
        ui.ctx().memory_mut(|m| m.request_focus(editor_id));

        self.autocomplete.items.clear();
        self.autocomplete.selected = 0;
        self.autocomplete.dismissed_at = None;
        self.autocomplete.notice = None;
        self.autocomplete.shown = false;
    }

    /// Refresh the candidates for the current caret, draw the popup under it,
    /// and accept a clicked row. Candidates are recomputed only when the text,
    /// schema, or caret changed (or completion was requested manually). On the
    /// frame a click into the popup steals the editor's focus, the popup is
    /// redrawn from the cached candidates so the click still lands.
    fn update_autocomplete(
        &mut self,
        ui: &egui::Ui,
        editor_id: egui::Id,
        schema: &ChannelSchema,
        output: &egui::widgets::text_edit::TextEditOutput,
    ) {
        let focused = output.response.has_focus();
        let manual = std::mem::take(&mut self.autocomplete.manual_request);
        if let (true, Some(caret_char)) = (
            focused,
            output.cursor_range.map(|range| range.primary.index),
        ) {
            let caret_byte = char_to_byte(self.session.text(), caret_char);
            let memo_key = (self.assist_revision, caret_byte);
            if manual || self.autocomplete.computed_for != Some(memo_key) {
                self.recompute_candidates(caret_byte, schema, manual);
                self.autocomplete.computed_for = Some(memo_key);
            }
            // Esc keeps the popup closed while completing the same word. The
            // caret moving to another word re-arms it.
            if self.autocomplete.dismissed_at == Some(self.autocomplete.range.start) {
                self.autocomplete.items.clear();
                self.autocomplete.notice = None;
                self.autocomplete.shown = false;
                return;
            }
            self.autocomplete.dismissed_at = None;
            // The popup anchors track the caret every frame - the window may
            // move without the text changing.
            let caret_rect = output.galley.pos_from_cursor(CCursor::new(caret_char));
            self.autocomplete.caret_pos =
                output.galley_pos + caret_rect.left_bottom().to_vec2() + egui::vec2(0.0, 2.0);
            self.autocomplete.caret_top =
                output.galley_pos + caret_rect.left_top().to_vec2() - egui::vec2(0.0, 2.0);
        } else if !self.autocomplete.shown {
            // Not editing and nothing was open: keep it closed.
            self.autocomplete.items.clear();
            self.autocomplete.notice = None;
            return;
        }

        if self.autocomplete.items.is_empty() && self.autocomplete.notice.is_none() {
            self.autocomplete.shown = false;
            return;
        }
        let clicked = draw_autocomplete_popup(ui, output.response.id, &self.autocomplete);
        self.autocomplete.shown = true;

        if let Some(index) = clicked {
            if let Some(candidate) = self.autocomplete.items.get(index).cloned() {
                self.accept_completion(ui, editor_id, &candidate);
            }
        } else if !focused {
            // Focus left the editor without a click into the popup (e.g. a
            // click elsewhere) - close it.
            self.autocomplete.items.clear();
            self.autocomplete.notice = None;
            self.autocomplete.shown = false;
        }
    }

    /// Compute the completion candidates for the caret at `caret_byte` and
    /// store them in `self.autocomplete`.
    fn recompute_candidates(&mut self, caret_byte: usize, schema: &ChannelSchema, manual: bool) {
        // Analyze only the query the caret is in, then shift the byte range
        // back to editor coordinates. The chunk ranges are recomputed from the
        // current text: the checked chunks are one frame stale (the editor
        // applies this frame's keystrokes after the check ran), and a stale
        // range would misroute the caret to the between-queries fallback.
        let context = gt_query_run::analysis_context(self.session.text(), caret_byte);
        let offset = context.start;
        let src = self.session.text().get(context).unwrap_or("");
        let local = caret_byte - offset;
        let trigger = if manual {
            CompletionTrigger::Manual
        } else {
            CompletionTrigger::Automatic
        };
        // A `@name` being typed offers channels, anywhere else the language
        // constructs. The `@` sigil makes the two positions disjoint.
        let mut notice = None;
        let (range, items) =
            if let Some(channels) = gt_query::channel_completions_at(src, local, schema) {
                if channels.items.is_empty() && schema.is_empty() {
                    // The sigil path exists but has nothing to offer: say why,
                    // instead of silently not appearing.
                    notice = Some("No channels loaded");
                }
                let items = channels
                    .items
                    .into_iter()
                    .map(Candidate::from_channel)
                    .collect();
                (channels.range, items)
            } else {
                let completions = gt_query::completions_at(src, local, schema, trigger);
                let items = completions
                    .items
                    .iter()
                    .map(Candidate::from_construct)
                    .collect::<Vec<_>>();
                (completions.range, items)
            };
        let range = range.start + offset..range.end + offset;

        // Keep the highlighted row while the candidate set is unchanged,
        // otherwise start at the top.
        let unchanged = items.iter().map(|c| &c.insert).eq(self
            .autocomplete
            .items
            .iter()
            .map(|c| &c.insert));
        self.autocomplete.selected = if unchanged {
            self.autocomplete
                .selected
                .min(items.len().saturating_sub(1))
        } else {
            0
        };
        // The popup claims Enter and the arrows only once the user has shown
        // intent: a typed prefix or an explicit request.
        self.autocomplete.active = manual || caret_byte > range.start;
        self.autocomplete.word = self
            .session
            .text()
            .get(range.clone())
            .unwrap_or("")
            .to_owned();
        self.autocomplete.items = items;
        self.autocomplete.notice = notice;
        self.autocomplete.range = range;
    }

    /// Show a documentation tooltip for the token under the pointer, in the
    /// editor. Suppressed while the completion popup is up, so the two don't
    /// stack. Shown only after the pointer has rested a moment on the token
    /// (though the doc already on display stays up while the pointer moves
    /// within its token), and only when it actually sits on the token's own
    /// rectangle (the galley clamps a position in the blank space right of a
    /// line to the nearest character, which is not a hover).
    fn hover_docs(
        &mut self,
        ui: &egui::Ui,
        schema: &ChannelSchema,
        output: &egui::widgets::text_edit::TextEditOutput,
    ) {
        // Taken up front and put back only when a doc actually shows, so every
        // bail-out below (popup up, left the editor, off any token) drops the
        // remembered span and the next token starts with the delay armed.
        let shown = self.hover_doc_span.take();
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
        let byte = char_to_byte(self.session.text(), ccursor.index);
        // Look up the token within the query under the pointer. Fresh ranges:
        // the checked chunks can be one frame stale after an edit.
        let Some(chunk) = gt_query_run::split_queries(self.session.text())
            .into_iter()
            .find(|range| range.start <= byte && byte <= range.end)
        else {
            return;
        };
        let local = byte - chunk.start;
        let src = self.session.text().get(chunk.clone()).unwrap_or("");
        // Hit-test the token's actual span rectangle before documenting it.
        let Some(token_span) = lexer::tokenize(src)
            .into_iter()
            .map(|t| t.span)
            .find(|span| span.start <= local && local < span.end)
        else {
            return;
        };
        if !self
            .token_rect(
                output,
                chunk.start + token_span.start,
                chunk.start + token_span.end,
            )
            .contains(pointer)
        {
            return;
        }
        let span = chunk.start + token_span.start..chunk.start + token_span.end;
        let rested = ui.input(|i| i.pointer.time_since_last_movement());
        if !hover_doc_shows(shown.as_ref(), &span, rested) {
            // Repaint when the delay lapses so the tooltip appears without
            // further pointer movement.
            ui.ctx()
                .request_repaint_after(Duration::from_secs_f32(HOVER_DOC_DELAY_SECS - rested));
            return;
        }
        // A channel wins (its `@` is unambiguous), else a language construct,
        // else nothing under the pointer to document.
        let doc = match gt_query::channel_at(src, local, schema) {
            Some(channel) => HoverDoc::Channel(channel),
            None => match gt_query::construct_at(src, local) {
                Some(construct) => HoverDoc::Construct(construct),
                None => return,
            },
        };
        self.hover_doc_span = Some(span);
        // Drawn as an Area (rather than a hover tooltip) so it is anchored to
        // the token under the pointer.
        Area::new(egui::Id::new("query_hover_doc"))
            .order(egui::Order::Tooltip)
            .fixed_pos(pointer + egui::vec2(12.0, 18.0))
            .constrain(true)
            .interactable(false)
            .show(ui.ctx(), |ui| {
                Frame::popup(ui.style()).show(ui, |ui| match &doc {
                    HoverDoc::Channel(channel) => channel_tooltip_ui(ui, channel),
                    HoverDoc::Construct(construct) => construct_tooltip_ui(ui, construct),
                });
            });
    }

    /// The screen rectangle spanned by the text between byte `start` and
    /// `end`, from the editor galley. Grown a little so hovering the very edge
    /// of a glyph still counts.
    fn token_rect(
        &self,
        output: &egui::widgets::text_edit::TextEditOutput,
        start: usize,
        end: usize,
    ) -> egui::Rect {
        let char_at = |byte: usize| {
            self.session
                .text()
                .get(..byte)
                .map_or(0, |s| s.chars().count())
        };
        let first = output.galley.pos_from_cursor(CCursor::new(char_at(start)));
        let last = output.galley.pos_from_cursor(CCursor::new(char_at(end)));
        first
            .union(last)
            .translate(output.galley_pos.to_vec2())
            .expand(1.0)
    }

    fn results_ui(
        &mut self,
        ui: &mut egui::Ui,
        inputs: RunInputs<'_>,
        display_mask: DisplayMask,
        highlight: &mut MapHighlight,
        requests: &mut PointClickRequests<'_>,
        reveal_matches_request: &mut Option<MatchRevealTarget>,
    ) {
        // Split borrows: the results are read while the fold state they drive
        // is written.
        let Self {
            session,
            folded_matches,
            ..
        } = self;
        let files = inputs.loaded_files.files();
        // What the map draws right now: a match row can only pin a point that
        // is on it.
        let scope = MapScope {
            files,
            visibility: inputs.visibility,
            filter: inputs.filter,
            display_mask,
            query_matches: session.matches(),
        };
        let Some(results) = session.results() else {
            ui.label(RichText::new("No runs yet").weak());
            return;
        };
        show_on_map_ui(ui, results.matches(), reveal_matches_request);
        match results {
            RunResults::Points(points) => {
                let mut outputs = MatchTableOutputs {
                    highlight,
                    requests,
                    folds: folded_matches,
                    reveal: reveal_matches_request,
                };
                points_results_ui(ui, points, files, scope, &mut outputs);
            }
            RunResults::Channel(channel) => channel_results_ui(ui, channel, files),
        }
        if results.stale() {
            ui.label(
                RichText::new(format!("Data changed since this run {EM_DASH} run again"))
                    .weak()
                    .italics(),
            );
        }
    }

    /// The checks already ran. Prepare the run from the visible data and hand
    /// its evaluation to a worker thread.
    #[expect(
        clippy::expect_used,
        reason = "thread spawn can only fail under extreme system resource exhaustion"
    )]
    fn spawn_run(&mut self, ctx: &egui::Context, inputs: RunInputs<'_>) {
        let Some(prepared) = self.session.start_run(inputs) else {
            return;
        };
        let (tx, rx) = mpsc::channel();
        let worker_ctx = ctx.clone();
        thread::Builder::new()
            .name("query-run".to_owned())
            .spawn(move || {
                let outcome = prepared.execute();
                // A send failure means the window dropped the receiver, so
                // there is nothing left to notify.
                tx.send(outcome).ok();
                worker_ctx.request_repaint();
            })
            .expect("failed to spawn query worker thread");

        self.worker = Some(rx);
    }
}

/// The button that frames the map on this run's matches and plays their
/// reveal animation again. Disabled with the reason in its hover text when the
/// run drew no halos, or when the data changed after it.
fn show_on_map_ui(
    ui: &mut egui::Ui,
    matches: &QueryMatches,
    reveal_matches_request: &mut Option<MatchRevealTarget>,
) {
    let disabled_reason = if matches.stale {
        Some(format!(
            "Data changed since this run {EM_DASH} run again to show its matches"
        ))
    } else if !matches.has_halos() {
        Some("This run drew no matches on the map".to_owned())
    } else {
        None
    };
    let button = Button::new(format!("{ICON_CROSSHAIR} Show on map")).small();
    let response = ui.add_enabled(disabled_reason.is_none(), button);
    if let Some(reason) = disabled_reason {
        response.on_disabled_hover_text(reason);
    } else if response
        .on_hover_text("Zoom the map to the matches and highlight them")
        .clicked()
    {
        *reveal_matches_request = Some(MatchRevealTarget::WholeRun);
    }
}

/// Paints a small filled square in a draw query's halo `color`, tying its
/// results section to the matching halos on the map. Painted rather than a
/// text glyph, which the editor font does not carry.
fn query_swatch(ui: &mut egui::Ui, color: egui::Color32) {
    let side = egui::TextStyle::Body.resolve(ui.style()).size;
    let (rect, _) = ui.allocate_exact_size(egui::Vec2::splat(side), egui::Sense::hover());
    ui.painter().rect_filled(rect.shrink(1.0), 2.0, color);
    // Space between the swatch and the summary the caller appends next.
    ui.add_space(ui.spacing().item_spacing.x);
}

/// The line naming one match: the track it is on, the wall-clock span it
/// covers, how many points it holds and how long it ran.
///
/// The recording's filename leads only while several files are loaded, where
/// the track number alone would not say which recording is meant.
pub(super) fn match_name_text(
    files: &[LoadedFile],
    track_ref: TrackRef,
    range: &Range<usize>,
) -> String {
    let time_of = |index: usize| {
        points_of(files, track_ref)
            .and_then(|points| points.get(index))
            .map(|p| p.tpv.time().utc().format("%H:%M:%S").to_string())
    };
    let track = match files.len() {
        0 | 1 => format!("#{}", track_ref.index),
        _ => {
            let file = track_ref.fi.get(files).map_or_else(
                || format!("file {}", track_ref.fi),
                |f| f.metadata.filename.clone(),
            );
            format!("{file} #{}", track_ref.index)
        }
    };
    let mut fields = vec![track];
    let last = gt_fmt::last_index_of_span(range);
    if let Some(start) = time_of(range.start) {
        fields.push(match last.and_then(time_of) {
            Some(end) => format!("{start} {RIGHTWARDS_ARROW} {end}"),
            None => start,
        });
    }
    let count = range.len();
    fields.push(format!(
        "{count} {}",
        gt_fmt::pluralize(count, "point", "points")
    ));
    if let Some(seconds) = track_ref
        .resolve(files)
        .and_then(|track| gt_fmt::match_duration_seconds(track, range))
    {
        fields.push(gt_fmt::format_match_duration(seconds));
    }
    fields.join(&format!(" {MIDDLE_DOT} "))
}

/// Render a points pipeline's per-query sections with their point match tables.
fn points_results_ui(
    ui: &mut egui::Ui,
    points: &PointsResults,
    files: &[LoadedFile],
    scope: MapScope<'_>,
    out: &mut MatchTableOutputs<'_, '_>,
) {
    let stale = points.matches.stale;
    // One collapsible section per query, in editor order: its summary is the
    // header (with a color swatch for draw queries), its match tables the body.
    // Stable ids keep the open/closed state across reruns.
    let ctx = MatchTableContext {
        files,
        results: points,
        scope,
    };
    for (qi, query) in points.queries.iter().enumerate() {
        let id = ui.make_persistent_id(("query_result", qi));
        egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, true)
            .show_header(ui, |ui| {
                if let Some(color) = query.color {
                    query_swatch(ui, gt_ui_theme::query_halo_color(color, false));
                }
                // Ahead of the summary, whose length varies with the run: the
                // two buttons then sit in the same place in every section.
                fold_all_button_ui(ui, query, out.folds);
                copy_tsv_button_ui(ui, &ctx, query);
                // Truncated to the width that is left, and stated in full on
                // hover: a summary that extends would push the window over the
                // map beside it.
                ui.add(Label::new(query.summary.as_str()).truncate())
                    .on_hover_text(query.summary.as_str());
            })
            .body(|ui| {
                match_table::query_matches_ui(ui, &ctx, qi, query, stale, out);
            });
    }
}

/// Folds every match of one query away, or expands them all again. The icon
/// and its hover text state which of the two the press performs.
fn fold_all_button_ui(ui: &mut egui::Ui, query: &PanelQuery, folds: &mut FoldedMatches) {
    let all_folded = folds.all_folded(&query.matches);
    let (icon, tooltip) = if all_folded {
        (ICON_ARROWS_OUT, "Expand all matches")
    } else {
        (ICON_ARROWS_IN, "Collapse all matches")
    };
    let has_matches = !query.matches.is_empty();
    let response = ui.add_enabled(has_matches, Button::new(icon).small());
    if !has_matches {
        response.on_disabled_hover_text("This query matched nothing");
    } else if response.on_hover_text(tooltip).clicked() {
        if all_folded {
            folds.expand_all(&query.matches);
        } else {
            folds.fold_all(&query.matches);
        }
    }
}

/// Copies one query's whole result to the clipboard as tab-separated values,
/// for a spreadsheet.
fn copy_tsv_button_ui(ui: &mut egui::Ui, ctx: &MatchTableContext<'_>, query: &PanelQuery) {
    let has_matches = !query.matches.is_empty();
    let response = ui.add_enabled(has_matches, Button::new(ICON_COPY).small());
    if !has_matches {
        response.on_disabled_hover_text("This query matched nothing");
    } else if response
        .on_hover_text(
            "Copy as tab-separated values: one line per matched point, \
             starting with the number of the match it belongs to and its \
             index in the track",
        )
        .clicked()
    {
        ui.ctx().copy_text(match_table::matches_as_tsv(ctx, query));
    }
}

/// Render a channel-source run: a summary header, then one sample match table
/// per track. Each row is a matched sample's time and component values.
fn channel_results_ui(ui: &mut egui::Ui, channel: &ChannelResults, files: &[LoadedFile]) {
    let id = ui.make_persistent_id(("channel_result", channel.channel.as_str()));
    egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, true)
        .show_header(ui, |ui| {
            ui.label(channel.summary.as_str());
        })
        .body(|ui| {
            let total: usize = channel
                .tracks
                .iter()
                .map(|t| t.ranges.iter().map(Range::len).sum::<usize>())
                .sum();
            if total == 0 {
                ui.label(RichText::new("No matches").weak());
            }
            for track in &channel.tracks {
                channel_track_ui(ui, channel, track, files);
            }
        });
}

/// One track's matched channel samples as a table: `time` plus one column per
/// component (or the channel name for a scalar). Capped like the point tables.
/// Values are evaluated in base units, then converted back to each track's
/// declared unit for display.
fn channel_track_ui(
    ui: &mut egui::Ui,
    channel: &ChannelResults,
    track: &ChannelTrackResult,
    files: &[LoadedFile],
) {
    let matched: usize = track.ranges.iter().map(Range::len).sum();
    if matched == 0 {
        return;
    }
    let file = track.track.fi.get(files).map_or_else(
        || format!("file {}", track.track.fi),
        |f| f.metadata.filename.clone(),
    );
    ui.label(
        RichText::new(format!(
            "{file} #{} {EM_DASH} {matched} {}",
            track.track.index,
            gt_fmt::pluralize(matched, "sample", "samples"),
        ))
        .strong(),
    );

    // Column headers: time, then each component (the channel name when scalar).
    let value_headers: Vec<String> = if channel.components.is_empty() {
        vec![channel.channel.clone()]
    } else {
        channel.components.clone()
    };
    let value_format = ColumnFormat::of_channel_unit(track.unit.as_ref());
    // Samples are timed finer than points, so their times keep the
    // milliseconds the point tables leave out.
    let time_format = ColumnFormat::time_of_day_with_millis();
    Grid::new(ui.id().with(("channel_table", track.track)))
        .striped(true)
        .show(ui, |ui| {
            time_format.header_ui(ui, "time", None);
            for header in &value_headers {
                value_format.header_ui(ui, header, None);
            }
            ui.end_row();

            let columns = track.timeline.columns.max(1);
            for sample in track
                .ranges
                .iter()
                .flat_map(Clone::clone)
                .take(CHANNEL_SAMPLE_ROW_CAP)
            {
                time_format.value_ui(ui, track.timeline.times.get(sample).copied());
                for col in 0..columns {
                    let value = track.timeline.values.get(sample * columns + col).copied();
                    value_format.value_ui(ui, value);
                }
                ui.end_row();
            }
        });
    if matched > CHANNEL_SAMPLE_ROW_CAP {
        ui.label(
            RichText::new(format!(
                "{EM_DASH} {} more samples",
                matched - CHANNEL_SAMPLE_ROW_CAP
            ))
            .weak(),
        );
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
/// spans), so the two cannot drift.
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
    gt_fmt::truncate_with_ellipsis(&flat, HISTORY_LINE_MAX_CHARS).into_owned()
}

fn points_of(files: &[LoadedFile], track_ref: TrackRef) -> Option<&[NavPoint]> {
    track_ref.resolve(files).map(|t| t.points.as_slice())
}

/// Byte offset of the `char_index`-th character, or the text length when the
/// index is at or past the end. The egui caret is a char index, the query
/// position model works in bytes.
fn char_to_byte(text: &str, char_index: egui::text::CharIndex) -> usize {
    text.char_indices()
        .nth(char_index.0)
        .map_or(text.len(), |(byte, _)| byte)
}

/// Whether the editor hover doc for the token at `span` shows this frame:
/// instantly while it is the token whose doc is already on display (`shown`),
/// so the doc survives pointer motion within one word, else only once the
/// pointer has rested [`HOVER_DOC_DELAY_SECS`].
fn hover_doc_shows(shown: Option<&Range<usize>>, span: &Range<usize>, rested_secs: f32) -> bool {
    shown == Some(span) || rested_secs >= HOVER_DOC_DELAY_SECS
}

/// Draw the completion popup under (or, when the screen ends, above) the
/// caret, returning the index of a clicked row.
fn draw_autocomplete_popup(
    ui: &egui::Ui,
    editor_id: egui::Id,
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

    // Flip above the caret when there is no room below, so the popup never
    // covers the line being typed. The height estimate mirrors the layout
    // above. `constrain` still clamps any residual overshoot.
    let visible_rows = autocomplete.items.len().clamp(1, AUTOCOMPLETE_VISIBLE_ROWS);
    let footer = if overflow > 0 { row_height } else { 0.0 };
    let frame_padding = Frame::popup(ui.style()).total_margin().sum().y;
    let est_height = row_height * visible_rows as f32 + footer + frame_padding;
    let pos = if autocomplete.caret_pos.y + est_height > ui.ctx().content_rect().bottom() {
        autocomplete.caret_top - egui::vec2(0.0, est_height)
    } else {
        autocomplete.caret_pos
    };

    Area::new(editor_id.with("autocomplete"))
        .order(egui::Order::Foreground)
        .fixed_pos(pos)
        .constrain(true)
        .show(ui.ctx(), |ui| {
            Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_min_width(220.0);
                ui.set_max_width(380.0);
                if let Some(notice) = autocomplete.notice {
                    ui.label(RichText::new(notice).weak().italics());
                    return;
                }
                ScrollArea::vertical()
                    .max_height(max_height)
                    .show(ui, |ui| {
                        for (index, candidate) in autocomplete.items.iter().enumerate() {
                            let selected = index == autocomplete.selected;
                            let response =
                                ui.selectable_label(selected, autocomplete_row(ui, candidate));
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
                        RichText::new(format!("{ICON_CARET_DOWN} {} more below", overflow))
                            .weak()
                            .small(),
                    );
                }
            });
        });
    clicked
}

/// One popup row: the candidate's insertion in code font, its summary dimmed
/// beside it.
fn autocomplete_row(ui: &egui::Ui, candidate: &Candidate) -> LayoutJob {
    let mut job = LayoutJob::default();
    job.append(
        &candidate.insert,
        0.0,
        egui::TextFormat {
            font_id: egui::TextStyle::Monospace.resolve(ui.style()),
            color: ui.visuals().strong_text_color(),
            ..Default::default()
        },
    );
    job.append(
        &format!("  {}", candidate.summary),
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
    tooltip_header(ui, construct.name, construct.kind.label());
    let mono = egui::TextStyle::Monospace.resolve(ui.style());
    let body = egui::TextStyle::Body.resolve(ui.style());
    let default = ui.visuals().text_color();
    let dark = ui.visuals().dark_mode;
    ui.label(construct.summary);
    if !construct.doc.is_empty() || !construct.examples.is_empty() {
        ui.separator();
    }
    if !construct.doc.is_empty() {
        let mut doc = LayoutJob::default();
        // The doc is prose with `backticked` code spans, so color the code.
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
        ui.label(query_syntax_layout(ui, example));
    }
}

/// A hover tooltip for a channel: its `@name` colored like the editor, a
/// "channel" label, then its dimension summary. Channels carry no catalog doc,
/// so the tooltip is just the header and summary.
fn channel_tooltip_ui(ui: &mut egui::Ui, channel: &gt_query::ChannelSuggestion) {
    tooltip_header(ui, &format!("@{}", channel.name), "channel");
    ui.label(&channel.summary);
}

/// The shared head of an editor hover tooltip: the token colored the way the
/// editor colors it, then a dimmed kind label beside it.
fn tooltip_header(ui: &mut egui::Ui, name: &str, kind_label: &str) {
    ui.set_max_width(TOOLTIP_MAX_WIDTH);
    let mono = egui::TextStyle::Monospace.resolve(ui.style());
    let default = ui.visuals().text_color();
    let dark = ui.visuals().dark_mode;
    ui.horizontal(|ui| {
        let mut colored = LayoutJob::default();
        append_query_syntax(&mut colored, &mono, default, dark, name);
        ui.label(colored);
        ui.label(RichText::new(kind_label).weak().small());
    });
}

/// A single-color text section for a [`LayoutJob`].
fn text_format(font: &egui::FontId, color: egui::Color32) -> egui::TextFormat {
    egui::TextFormat {
        font_id: font.clone(),
        color,
        ..Default::default()
    }
}

/// The syntax-highlight color for a token class in the current theme.
/// `default` colors whitespace and punctuation. Shared by the editor's layouter and the
/// hover doc so they can't diverge.
fn syntax_color(class: TokenClass, default: egui::Color32, dark_mode: bool) -> egui::Color32 {
    match class {
        TokenClass::Keyword => gt_ui_theme::query_syntax_keyword(dark_mode),
        TokenClass::Number => gt_ui_theme::query_syntax_number(dark_mode),
        TokenClass::Ident => gt_ui_theme::query_syntax_ident(dark_mode),
        TokenClass::Comment => gt_ui_theme::query_syntax_comment(dark_mode),
        TokenClass::Punctuation => default,
        TokenClass::Error => gt_ui_theme::error_indicator(dark_mode),
    }
}

/// One query in the editor's colors and font, wrapped to the width available,
/// for the windows that show a query they do not edit.
pub(super) fn query_syntax_layout(ui: &egui::Ui, query: &str) -> LayoutJob {
    let mut job = LayoutJob::default();
    append_query_syntax(
        &mut job,
        &egui::TextStyle::Monospace.resolve(ui.style()),
        ui.visuals().text_color(),
        ui.visuals().dark_mode,
        query,
    );
    job.wrap.max_width = ui.available_width();
    job
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
    let error_color = gt_ui_theme::error_indicator(ui.visuals().dark_mode);
    let mut job = LayoutJob::default();
    job.append(
        &format!("{ICON_WARNING_OCTAGON} "),
        0.0,
        text_format(&body, error_color),
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
                text_format(&body, error_color)
            };
            job.append(part, 0.0, format);
        }
        in_code = !in_code;
    }
    job.wrap.max_width = ui.available_width();
    job
}

/// Token-driven syntax highlighting plus the diagnostic underlines (one per
/// failing chunk), built from the same lexer the parser uses.
fn highlight_layout(ui: &egui::Ui, text: &str, diagnostics: &[Span]) -> LayoutJob {
    let font = egui::TextStyle::Monospace.resolve(ui.style());
    let default_color = ui.visuals().text_color();
    let dark_mode = ui.visuals().dark_mode;
    let error_color = gt_ui_theme::error_indicator(dark_mode);
    let underlines: Vec<(usize, usize)> = diagnostics
        .iter()
        .map(|span| (span.start, span.end.max(span.start + 1)))
        .map(|(start, end)| (start, end.min(text.len().max(start))))
        .collect();

    let mut job = LayoutJob::default();
    let mut append = |range: Range<usize>, color: egui::Color32| {
        let Some(slice) = text.get(range.clone()) else {
            return;
        };
        if slice.is_empty() {
            return;
        }
        let underlined = underlines
            .iter()
            .any(|&(start, end)| range.start < end && start < range.end);
        let format = egui::TextFormat {
            font_id: font.clone(),
            color,
            underline: if underlined {
                egui::Stroke::new(2.0_f32, error_color)
            } else {
                egui::Stroke::NONE
            },
            ..Default::default()
        };
        job.append(slice, 0.0, format);
    };

    // Cut at token boundaries and at the diagnostic edges so each underline
    // starts and ends exactly on its reported span.
    let mut cursor = 0;
    for (span, class) in lexer::highlight_classes(text) {
        for range in segments(cursor..span.start, &underlines) {
            append(range, default_color);
        }
        let color = syntax_color(class, default_color, dark_mode);
        for range in segments(span.start..span.end, &underlines) {
            append(range, color);
        }
        cursor = span.end;
    }
    for range in segments(cursor..text.len(), &underlines) {
        append(range, default_color);
    }
    job
}

/// Split a byte range at the diagnostic edges so each piece is uniformly
/// underlined or not.
fn segments(range: Range<usize>, underlines: &[(usize, usize)]) -> Vec<Range<usize>> {
    if underlines.is_empty() {
        return vec![range];
    }
    let mut cuts = vec![range.start, range.end];
    for &(start, end) in underlines {
        for edge in [start, end] {
            if range.start < edge && edge < range.end {
                cuts.push(edge);
            }
        }
    }
    cuts.sort_unstable();
    cuts.dedup();
    cuts.windows(2)
        .filter_map(|pair| match pair {
            [a, b] if a < b => Some(*a..*b),
            _ => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use gt_query::TrackInput;
    use gt_query_run::TrackProvider;
    use gt_types::{FileIdx, TrackIdx};
    use gt_ui_theme::ELLIPSIS;
    use rstest::rstest;

    use super::*;

    /// A range built from arguments, so a single-element `vec![rng(0, 1)]` does
    /// not trip clippy's `single_range_in_vec_init`.
    fn rng(start: usize, end: usize) -> Range<usize> {
        start..end
    }

    /// Every built-in example parses, type-checks, and runs against a real
    /// track - the guard that keeps embedded queries valid as the language
    /// evolves.
    #[test]
    fn examples_parse_check_and_run() {
        let points = gt_test_utils::nav_test_data();
        let provider = TrackProvider::new(&points, &[], None);
        let inputs = [TrackInput {
            track: TrackRef::new(FileIdx::new(0), TrackIdx::new(0)),
            provider: &provider,
        }];
        for example in EXAMPLES {
            let parsed = gt_query::parse(example.text).unwrap_or_else(|e| {
                panic!("example {:?} failed to parse: {}", example.name, e.message)
            });
            let checked =
                gt_query::check(&parsed, &gt_query::ChannelSchema::new()).unwrap_or_else(|e| {
                    panic!("example {:?} failed to check: {}", example.name, e.message)
                });
            // Runs without panicking. Util/slip series are absent here, which
            // the poison rules handle, so we only assert it completes.
            let _ = gt_query::run(&checked, &inputs);
        }
    }

    #[rstest]
    // The doc on display sticks while the pointer moves within its token.
    #[case(Some(rng(0, 5)), rng(0, 5), 0.0, true)]
    // Entering another token re-arms the delay, however long the doc was up.
    #[case(Some(rng(0, 5)), rng(6, 8), HOVER_DOC_DELAY_SECS - 0.01, false)]
    // No doc up: the pointer must rest the full delay first.
    #[case(None, rng(0, 5), HOVER_DOC_DELAY_SECS - 0.01, false)]
    #[case(None, rng(0, 5), HOVER_DOC_DELAY_SECS, true)]
    // A rested pointer shows the doc of a newly entered token too.
    #[case(Some(rng(0, 5)), rng(6, 8), HOVER_DOC_DELAY_SECS, true)]
    fn hover_doc_sticks_to_its_token_and_delays_new_ones(
        #[case] shown: Option<Range<usize>>,
        #[case] span: Range<usize>,
        #[case] rested_secs: f32,
        #[case] expected: bool,
    ) {
        assert_eq!(
            hover_doc_shows(shown.as_ref(), &span, rested_secs),
            expected
        );
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
        assert_eq!(shown.chars().count(), HISTORY_LINE_MAX_CHARS.get() + 1);
    }

    #[test]
    fn segments_split_exactly_at_diagnostic_edges() {
        assert_eq!(segments(0..10, &[]), vec![0..10]);
        assert_eq!(segments(0..10, &[(3, 7)]), vec![0..3, 3..7, 7..10]);
        assert_eq!(segments(4..6, &[(3, 7)]), vec![4..6]);
        assert_eq!(segments(0..5, &[(3, 9)]), vec![0..3, 3..5]);
        assert_eq!(segments(5..5, &[(3, 9)]), Vec::<Range<usize>>::new());
        // Two diagnostics cut independently.
        assert_eq!(
            segments(0..10, &[(1, 3), (5, 7)]),
            vec![0..1, 1..3, 3..5, 5..7, 7..10]
        );
    }

    #[test]
    fn candidate_from_channel_inserts_the_at_sigil() {
        let channel = gt_query::ChannelSuggestion {
            name: "accel".to_owned(),
            summary: "in m/s2".to_owned(),
        };
        let candidate = Candidate::from_channel(channel);
        assert_eq!(candidate.insert, "@accel");
        assert_eq!(candidate.summary, "in m/s2");
        assert_eq!(candidate.suffix, "", "a channel takes no trailing space");
    }

    #[test]
    fn candidate_from_construct_maps_name_and_insertion() {
        // A stage keyword gets a trailing space so the next token can follow.
        // A metric does not: an operator, not a space, comes next. A function
        // brings its parentheses with the caret stepping inside them. A unit
        // pads itself off an adjacent digit.
        let find = |name| gt_query::catalog().iter().find(|c| c.name == name);
        let stage = Candidate::from_construct(find("where").expect("where is catalogued"));
        assert_eq!(stage.insert, "where");
        assert_eq!(stage.suffix, " ");
        let metric = Candidate::from_construct(find("velocity").expect("velocity is catalogued"));
        assert_eq!(metric.insert, "velocity");
        assert_eq!(metric.suffix, "");
        assert!(!metric.pad_after_digit);
        let func = Candidate::from_construct(find("avg").expect("avg is catalogued"));
        assert_eq!(func.suffix, "()");
        assert_eq!(func.caret_back, 1);
        let unit = Candidate::from_construct(find("km/h").expect("km/h is catalogued"));
        assert!(unit.pad_after_digit);
    }

    /// One loaded file named `filename`, holding one track of `points` fixes a
    /// second apart from noon.
    fn file_of(filename: &str, points: usize) -> LoadedFile {
        let noon = DateTime::<Utc>::from_timestamp(12 * 3600, 0).unwrap_or_default();
        LoadedFile {
            metadata: gt_types::FileMetadata {
                filename: filename.to_owned(),
                ..gt_test_utils::empty_file_metadata()
            },
            tracks: vec![gt_test_utils::loaded_track_with_points(
                gt_test_utils::nav_points_from(noon, points, 1),
            )],
            event_marker_styles: std::collections::HashMap::new(),
            orphaned_event_markers: Vec::new(),
            source: gt_types::FileSource::GtdBytes(std::sync::Arc::from(Vec::<u8>::new())),
            load_warnings: Vec::new(),
        }
    }

    /// A match's name states which track it is on, the span it covers, how many
    /// points it holds and how long it ran. The filename leads only where
    /// several recordings are loaded.
    #[rstest]
    #[case::one_file(1, rng(0, 5), "#0 · 12:00:00 → 12:00:04 · 5 points · 4 s")]
    #[case::several_files(2, rng(0, 5), "one.gtd #0 · 12:00:00 → 12:00:04 · 5 points · 4 s")]
    #[case::single_point(1, rng(3, 4), "#0 · 12:00:03 · 1 point")]
    #[case::over_a_minute(1, rng(0, 61), "#0 · 12:00:00 → 12:01:00 · 61 points · 1:00 min")]
    fn a_match_name_states_its_track_span_and_duration(
        #[case] file_count: usize,
        #[case] range: Range<usize>,
        #[case] expected: &str,
    ) {
        let files: Vec<LoadedFile> = ["one.gtd", "two.gtd"]
            .into_iter()
            .take(file_count)
            .map(|name| file_of(name, 61))
            .collect();
        let track = TrackRef::new(FileIdx::new(0), TrackIdx::new(0));
        assert_eq!(match_name_text(&files, track, &range), expected);
    }

    /// A track that is no longer loaded has no times to state: the name is
    /// left with the match's own point count.
    #[test]
    fn a_match_on_an_unloaded_track_states_only_its_size() {
        let track = TrackRef::new(FileIdx::new(3), TrackIdx::new(1));
        assert_eq!(match_name_text(&[], track, &rng(0, 3)), "#1 · 3 points");
    }
}

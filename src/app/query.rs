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
    ChannelInfo, ChannelSamples, ChannelSchema, ChannelTimeline, CheckedQuery, CompletionTrigger,
    Construct, ConstructKind, Diagnostic, MetricProvider, PipelineOutput, Quantity, QueryMetric,
    RunSummary, Span, TrackInput, TrackMatches, Unit,
};
use gt_side_panel::widgets::apply_point_click;
use gt_types::satellites::Constellation;
use gt_types::{
    Channel, DataCategory, DisplayMode, FileIdx, LoadedFile, NavPoint, PointIdx, TrackIdx, TrackRef,
};
use gt_ui_theme::{DEGREE_SIGN, EM_DASH};
use gt_ui_types::{
    DataPointRef, DrawLayer, DrawLayerMask, HighlightScope, MapHighlight, MatchHighlight,
    QueryMatches, TrackDataVisibility,
};

use crate::settings::QueryHistoryEntry;

/// Rows shown per match table before truncating with a "more points" note.
const MATCH_TABLE_ROW_CAP: usize = 100;

/// Unpinned history entries kept before the oldest is evicted. Pinned
/// entries never count against this cap.
const MAX_UNPINNED_HISTORY: usize = 50;

/// Characters of a history entry's first line shown before eliding.
const HISTORY_LINE_MAX_CHARS: usize = 48;

/// Microseconds per second, for converting a channel sample's `timestamp_micros`
/// to the evaluator's seconds.
const MICROS_PER_SEC: f64 = 1_000_000.0;

/// Max width of an editor hover tooltip, shared by the construct and channel
/// tooltips so they stay the same size.
const TOOLTIP_MAX_WIDTH: f32 = 360.0;

/// Ellipsis appended to an elided history line.
const ELLIPSIS: &str = "…";

/// Id salt for the query editor's text field. Fixed (not derived from the
/// enclosing `Ui`) so the autocomplete caret/focus plumbing - and the UI
/// snapshot test - can address the widget directly.
pub(crate) const EDITOR_ID_SALT: &str = "query_editor";

/// Candidate rows the autocomplete popup shows before it scrolls. A footer
/// notes how many more there are.
const AUTOCOMPLETE_VISIBLE_ROWS: usize = 5;

/// Seconds the pointer must rest before the editor hover doc appears, so the
/// tooltip does not flicker over every token the pointer crosses. Only entering
/// a token arms the delay; the doc already on display survives pointer motion
/// within its token.
const HOVER_DOC_DELAY_SECS: f32 = 0.15;

/// Seconds after the last keystroke before the caret chunk's diagnostic shows.
/// A query is structurally broken for most of the time it is being typed
/// (`points |` until the keyword lands); flashing red on every keystroke reads
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
    /// The blank-line-separated queries of `text`, each parsed and checked.
    /// Kept in sync by `editor_ui`.
    chunks: Vec<Chunk>,
    /// The text `chunks` was computed from.
    checked_text: String,
    /// The channel schema `chunks` was checked against. Kept so a file load or
    /// unload that changes the channels re-checks even when the text is
    /// unchanged (a `@name` error resolves once its channel appears).
    checked_schema: ChannelSchema,
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
    /// The editor's autocomplete popup, recomputed from the caret when the
    /// text, schema, or caret changed.
    autocomplete: Autocomplete,
    /// Bumped whenever the checked text or schema changes; keys the
    /// autocomplete memo so candidates are not recomputed every repaint.
    assist_revision: u64,
    /// `ui.input(..).time` of the last text edit, for the diagnostic grace
    /// period on the chunk being typed in.
    last_edit_time: Option<f64>,
    /// Whether the editor had keyboard focus last frame. Read (not the live
    /// value) by the window's Escape handling: egui surrenders focus on Escape
    /// before the editor renders, so the live value is already false on the
    /// very frame the Escape should only unfocus, not close.
    editor_had_focus: bool,
    /// Editor-global byte span of the token whose hover doc is on display,
    /// `None` while no doc shows. Keeps the doc up while the pointer moves
    /// within the token instead of re-arming the hover delay on every twitch.
    hover_doc_span: Option<Range<usize>>,
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
            // Something always follows these; land the caret past a space.
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
    /// Set by Ctrl+Space; the next recompute runs with the manual trigger,
    /// which offers candidates even on an empty prefix.
    manual_request: bool,
    /// A non-interactive explanation row shown instead of candidates (typing
    /// `@` with no channels loaded).
    notice: Option<&'static str>,
    /// Memo key of the last candidate computation: the window's assist
    /// revision and the caret byte. Unchanged key, unchanged candidates.
    computed_for: Option<(u64, usize)>,
    /// Whether the popup was drawn last frame. Key handling keys off this
    /// rather than live focus: egui surrenders a widget's focus on Escape (and
    /// on a click into the popup) *before* the editor renders, so live focus
    /// reads false on the very frame the popup must still act.
    shown: bool,
}

/// One query in the editor: its byte range in `text` and its check outcome.
struct Chunk {
    range: Range<usize>,
    result: Result<CheckedQuery, Diagnostic>,
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
}

/// What the worker sends back. `output: None` means the run was cancelled -
/// previous results stay untouched.
struct RunCompleted {
    output: Option<RunProduct>,
    /// Per-track derived series for the points path; empty for a channel run.
    track_data: HashMap<TrackRef, TrackQueryData>,
}

/// The worker's product, dispatched on the source of the run's queries.
enum RunProduct {
    /// A composed points pipeline.
    Points(PipelineOutput),
    /// A standalone channel-source run: matched sample ranges per track, and
    /// the source channel's timeline for the sample tables.
    Channel(ChannelRun),
}

/// A channel-source run's raw output, before the panel projection.
struct ChannelRun {
    channel: String,
    /// Component labels for a vector channel (`["x","y","z"]`), empty for a
    /// scalar. Column headers for the sample table.
    components: Vec<String>,
    summary: RunSummary,
    tracks: Vec<ChannelTrackResult>,
    /// The map effect: matched sample spans projected onto the track as
    /// enclosing nav-point ranges, honoring the query's draw/keep/hide mode.
    matches: QueryMatches,
}

/// How a run dispatches, decided from the checked queries' sources.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RunKind {
    /// Every query is points-source: run the composing pipeline.
    Points,
    /// A single channel-source query: run it standalone over its samples.
    Channel,
    /// A channel source mixed with other queries - not allowed, since a channel
    /// has its own timeline and cannot compose in one pipeline.
    MixedChannel,
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

/// Everything one run produced and the UI needs to show it: either a points
/// pipeline (map halos + point match tables) or a channel-source run (sample
/// match tables plus halos over the matched track segments).
struct QueryResults {
    body: ResultsBody,
    fingerprint: RunFingerprint,
}

/// The two kinds of run result, dispatched on the query source.
enum ResultsBody {
    Points(PointsResults),
    Channel(ChannelResults),
}

/// A points-pipeline run: its composed map effect and per-query panel rows.
struct PointsResults {
    /// The composed display effect for the map.
    matches: QueryMatches,
    /// Per query, in editor order, for the results panel.
    queries: Vec<PanelQuery>,
    /// Per-track derived series (only for metrics some query referenced),
    /// kept so match tables show the exact values the run used.
    track_data: HashMap<TrackRef, TrackQueryData>,
}

/// A channel-source run: matched sample ranges per track over the source
/// channel's own timeline. Renders as sample tables, and as halos over the
/// track segments the matched spans cover.
struct ChannelResults {
    /// The source channel's name, for the panel header.
    channel: String,
    /// Component labels for a vector channel, empty for a scalar.
    components: Vec<String>,
    summary: String,
    tracks: Vec<ChannelTrackResult>,
    /// The map effect: halos over the matched track segments, honoring the
    /// query mode. Carries its own `stale` flag for the map.
    matches: QueryMatches,
}

/// One track's channel-source matches and the timeline they index into.
struct ChannelTrackResult {
    track: TrackRef,
    /// Matched sample-index ranges into `timeline`.
    ranges: Vec<Range<usize>>,
    timeline: ChannelTimeline,
}

/// One query's result for the panel: its summary line, columns, and matches.
struct PanelQuery {
    /// Palette color index when this query draws, for the swatch; `None`
    /// otherwise.
    color: Option<usize>,
    summary: String,
    columns: Vec<QueryMetric>,
    /// Absolute point-index ranges this query matched.
    matches: Vec<TrackMatches>,
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
            chunks: check_all(&text, &ChannelSchema::new()),
            checked_text: text.clone(),
            checked_schema: ChannelSchema::new(),
            text,
            open: false,
            run_requested: false,
            cancel_requested: false,
            running: None,
            results: None,
            history: Vec::new(),
            history_revision: 0,
            autocomplete: Autocomplete::default(),
            assist_revision: 0,
            last_edit_time: None,
            editor_had_focus: false,
            hover_doc_span: None,
        }
    }

    /// Matches of the last run, for the map. `None` when there was no run.
    pub fn matches(&self) -> Option<&QueryMatches> {
        match &self.results.as_ref()?.body {
            ResultsBody::Points(p) => Some(&p.matches),
            ResultsBody::Channel(c) => Some(&c.matches),
        }
    }

    /// Whether the queries are currently affecting the map (any hidden points
    /// or halos). Drives the toolbar indicator shown while the window is closed.
    pub fn filter_active(&self) -> bool {
        self.matches().is_some_and(|matches| !matches.is_empty())
    }

    /// Whether every query in the editor checks and there is at least one, so
    /// the pipeline can run.
    fn all_ok(&self) -> bool {
        !self.chunks.is_empty() && self.chunks.iter().all(|c| c.result.is_ok())
    }

    /// How a run of the current (checked) queries would dispatch, or why it
    /// cannot run. Only meaningful when [`all_ok`](Self::all_ok).
    fn run_kind(&self) -> RunKind {
        let sources = self
            .chunks
            .iter()
            .filter_map(|c| c.result.as_ref().ok())
            .map(gt_query::CheckedQuery::is_channel_source);
        let (channel_count, total) = sources.fold((0, 0), |(ch, n), is_channel| {
            (ch + usize::from(is_channel), n + 1)
        });
        match channel_count {
            0 => RunKind::Points,
            // A channel source has its own timeline, so it cannot compose with
            // other queries in one pipeline: it must stand alone.
            _ if channel_count == total && total == 1 => RunKind::Channel,
            _ => RunKind::MixedChannel,
        }
    }

    /// The editor-coordinate span of each channel-source chunk's `@name`
    /// source token, for the mixed-channel underline.
    fn channel_source_spans(&self) -> Vec<Span> {
        self.chunks
            .iter()
            .filter(|c| c.result.as_ref().is_ok_and(CheckedQuery::is_channel_source))
            .filter_map(|c| {
                let src = self.text.get(c.range.clone())?;
                let first = lexer::tokenize(src).into_iter().next()?;
                Some(Span::new(
                    first.span.start + c.range.start,
                    first.span.end + c.range.start,
                ))
            })
            .collect()
    }

    /// The byte offset of the editor caret, from the text edit's stored state.
    /// Zero before the editor has ever been focused.
    fn caret_byte(&self, ctx: &egui::Context, editor_id: egui::Id) -> usize {
        let caret_char = egui::TextEdit::load_state(ctx, editor_id)
            .and_then(|state| state.cursor.char_range())
            .map_or(0, |range| range.primary.index);
        char_to_byte(&self.text, caret_char)
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
        loaded_files: LoadedFilesView<'_>,
        visibility: &TrackDataVisibility,
        filter: &GlobalFilter,
        highlight: &mut MapHighlight,
        requests: &mut MatchMapRequests<'_>,
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
            let stale =
                current_fingerprint(loaded_files, visibility, filter) != results.fingerprint;
            match &mut results.body {
                ResultsBody::Points(p) => p.matches.stale = stale,
                ResultsBody::Channel(c) => c.matches.stale = stale,
            }
        }

        let files = loaded_files.files();
        // The channels the editor checks `@name` against, gathered across every
        // loaded track.
        let schema = schema_from_files(files);
        // Whether the editor held focus at the start of this frame (egui drops
        // focus on Escape before any widget runs, so the field - updated
        // inside `editor_ui` - must be read before the window renders).
        let editor_was_focused = self.editor_had_focus;
        let mut open = self.open;
        egui::Window::new("Query")
            .open(&mut open)
            .default_width(460.0)
            .default_height(520.0)
            .resizable(true)
            .vscroll(true)
            .show(ctx, |ui| {
                self.editor_ui(ui, &schema);
                ui.separator();
                self.results_ui(ui, files, highlight, requests);
                ui.separator();
                self.history_examples_ui(ui);
            });

        // Esc closes the window - but not out from under someone typing: with
        // the editor focused, the first Esc only unfocuses it (the completion
        // popup, when open, consumes its own Esc before this).
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
            && self.all_ok()
            && self.run_kind() != RunKind::MixedChannel
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
        let body = match output {
            RunProduct::Points(pipeline) => {
                ResultsBody::Points(points_results(&pipeline, completed.track_data))
            }
            RunProduct::Channel(run) => ResultsBody::Channel(channel_results(run)),
        };
        self.results = Some(QueryResults {
            body,
            fingerprint: running.fingerprint,
        });
    }

    fn editor_ui(&mut self, ui: &mut egui::Ui, schema: &ChannelSchema) {
        let editor_id = egui::Id::new(EDITOR_ID_SALT);
        // Runs before the editor so the open popup can claim its keys, and may
        // edit the text (accepting a candidate) - so re-check after.
        self.apply_autocomplete_input(ui, editor_id);

        let text_changed = self.checked_text != self.text;
        if text_changed || self.checked_schema != *schema {
            self.chunks = check_all(&self.text, schema);
            self.checked_text = self.text.clone();
            self.checked_schema = schema.clone();
            self.assist_revision += 1;
            if text_changed {
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
        for chunk in &self.chunks {
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
        // every chunk checks green; surface it like a check error (underline
        // the channel sources, message below) instead of leaving a dead Run
        // button as the only clue.
        let mixed = self.all_ok() && self.run_kind() == RunKind::MixedChannel;
        if mixed {
            underlines.extend(self.channel_source_spans());
        }

        let mut layouter = |ui: &egui::Ui, buf: &dyn egui::TextBuffer, wrap_width: f32| {
            let mut job = highlight_layout(ui, buf.as_str(), &underlines);
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
        self.editor_had_focus = output.response.has_focus();

        self.update_autocomplete(ui, editor_id, schema, &output);
        self.hover_docs(ui, schema, &output);

        for diagnostic in &errors {
            // The message shows in red with an error icon (the quoted token
            // lifts into the code font); the fix, carried in the structured
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
            let in_flight = self.running.is_some();
            let all_ok = self.all_ok();
            let mixed = all_ok && self.run_kind() == RunKind::MixedChannel;
            let runnable = all_ok && !in_flight && !mixed;
            let run = ui.add_enabled(runnable, egui::Button::new("Run"));
            let run = match (all_ok, in_flight, mixed) {
                (false, _, _) if self.chunks.is_empty() => {
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

        // Keyed off `shown` (last frame), not live focus, because egui
        // surrenders the editor's focus on Escape before this runs. Key state
        // is read inside `input_mut`, but the follow-up (focus, text edits)
        // happens after - re-entering the context lock inside would deadlock.
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
                // Tab always accepts; Enter only on an active popup. Ctrl+Enter
                // carries the COMMAND modifier, so it is left for the window's
                // run shortcut.
                if input.consume_key(egui::Modifiers::NONE, egui::Key::Tab)
                    || (active && input.consume_key(egui::Modifiers::NONE, egui::Key::Enter))
                {
                    accept = Some(self.autocomplete.selected);
                }
            });
        } else if self.autocomplete.shown && self.autocomplete.notice.is_some() {
            // A notice-only popup has nothing to accept; Esc just closes it.
            if ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
                dismissed = true;
            }
        }

        if dismissed {
            self.autocomplete.dismissed_at = Some(self.autocomplete.range.start);
            self.autocomplete.items.clear();
            self.autocomplete.notice = None;
            self.autocomplete.shown = false;
            // egui already dropped the editor's focus for this Escape; put it
            // back so dismissing the popup keeps the caret in the editor.
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
        // The candidates were computed for the word recorded alongside them; a
        // same-frame edit (key repeat, paste) may have moved or replaced it,
        // in which case accepting would splice the wrong span - do nothing.
        if self.text.get(range.clone()) != Some(self.autocomplete.word.as_str()) {
            return;
        }
        // A unit accepted directly after a digit gets a separating space:
        // `30` + `km/h` reads `30 km/h`, the way the docs write it.
        let pad = candidate.pad_after_digit
            && range
                .start
                .checked_sub(1)
                .and_then(|i| self.text.as_bytes().get(i))
                .is_some_and(u8::is_ascii_digit);
        let space = if pad { " " } else { "" };
        let insertion = format!("{space}{}{}", candidate.insert, candidate.suffix);
        let caret_byte = range.start + insertion.len() - candidate.caret_back;
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
        self.autocomplete.dismissed_at = None;
        self.autocomplete.notice = None;
        self.autocomplete.shown = false;
    }

    /// Refresh the candidates for the current caret, draw the popup under it,
    /// and accept a clicked row. Candidates are recomputed only when the text,
    /// schema, or caret changed (or completion was requested manually); on the
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
            let caret_byte = char_to_byte(&self.text, caret_char);
            let memo_key = (self.assist_revision, caret_byte);
            if manual || self.autocomplete.computed_for != Some(memo_key) {
                self.recompute_candidates(caret_byte, schema, manual);
                self.autocomplete.computed_for = Some(memo_key);
            }
            // Esc keeps the popup closed while completing the same word; the
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
        let context = analysis_context(&self.text, caret_byte);
        let offset = context.start;
        let src = self.text.get(context).unwrap_or("");
        let local = caret_byte - offset;
        let trigger = if manual {
            CompletionTrigger::Manual
        } else {
            CompletionTrigger::Automatic
        };
        // A `@name` being typed offers channels; anywhere else, the language
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

        // Keep the highlighted row while the candidate set is unchanged;
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
        self.autocomplete.word = self.text.get(range.clone()).unwrap_or("").to_owned();
        self.autocomplete.items = items;
        self.autocomplete.notice = notice;
        self.autocomplete.range = range;
    }

    /// Show a documentation tooltip for the token under the pointer, in the
    /// editor. Suppressed while the completion popup is up, so the two don't
    /// stack; shown only after the pointer has rested a moment on the token
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
        let byte = char_to_byte(&self.text, ccursor.index);
        // Look up the token within the query under the pointer. Fresh ranges:
        // the checked chunks can be one frame stale after an edit.
        let Some(chunk) = split_queries(&self.text)
            .into_iter()
            .find(|range| range.start <= byte && byte <= range.end)
        else {
            return;
        };
        let local = byte - chunk.start;
        let src = self.text.get(chunk.clone()).unwrap_or("");
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
        egui::Area::new(egui::Id::new("query_hover_doc"))
            .order(egui::Order::Tooltip)
            .fixed_pos(pointer + egui::vec2(12.0, 18.0))
            .constrain(true)
            .interactable(false)
            .show(ui.ctx(), |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| match &doc {
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
        let char_at = |byte: usize| self.text.get(..byte).map_or(0, |s| s.chars().count());
        let first = output.galley.pos_from_cursor(CCursor::new(char_at(start)));
        let last = output.galley.pos_from_cursor(CCursor::new(char_at(end)));
        first
            .union(last)
            .translate(output.galley_pos.to_vec2())
            .expand(1.0)
    }

    fn results_ui(
        &self,
        ui: &mut egui::Ui,
        files: &[LoadedFile],
        highlight: &mut MapHighlight,
        requests: &mut MatchMapRequests<'_>,
    ) {
        let Some(results) = &self.results else {
            ui.label(egui::RichText::new("No runs yet").weak());
            return;
        };
        let stale = match &results.body {
            ResultsBody::Points(points) => {
                points_results_ui(ui, points, files, highlight, requests);
                points.matches.stale
            }
            ResultsBody::Channel(channel) => {
                channel_results_ui(ui, channel, files);
                channel.matches.stale
            }
        };
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
        if !self.all_ok() {
            return;
        }
        let queries: Vec<CheckedQuery> = self
            .chunks
            .iter()
            .filter_map(|c| c.result.as_ref().ok().cloned())
            .collect();
        let fingerprint = current_fingerprint(loaded_files, visibility, filter);

        // Owned snapshot for the worker: each evaluated track's full point
        // vector and its channels, plus the sub-range passing the time filter.
        // Cloning is the simple-and-correct baseline; an Arc-based snapshot is
        // the known follow-up if this shows up in profiling.
        let files = loaded_files.files();
        let tracks: Vec<TrackSnapshot> = fingerprint
            .tracks
            .iter()
            .filter_map(|&track_ref| {
                let track = track_ref.resolve(files)?;
                let slice = gt_filter::time_filtered_range(&track.points, filter);
                Some(TrackSnapshot {
                    track_ref,
                    points: track.points.clone(),
                    channels: track.channels.clone(),
                    slice,
                })
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
                let completed = run_worker(&queries, &tracks, &worker_cancel, &worker_prepared);
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
        });
    }
}

/// One track's owned data for the worker: the full point vector and channels,
/// plus the sub-range of points passing the time filter.
struct TrackSnapshot {
    track_ref: TrackRef,
    points: Vec<NavPoint>,
    channels: Vec<Channel>,
    slice: Range<usize>,
}

/// The worker body: derived series per track, then the sequential pipeline
/// evaluation, with cancellation checks between tracks (gt-query checks within
/// them).
fn run_worker(
    queries: &[CheckedQuery],
    tracks: &[TrackSnapshot],
    cancel: &AtomicBool,
    prepared: &AtomicUsize,
) -> RunCompleted {
    let cancelled = || cancel.load(Ordering::Relaxed);
    // A channel-source query runs standalone (the UI gates a mix), on its own
    // sample timeline rather than the composing points pipeline.
    if let Some(query) = queries.first().filter(|q| q.is_channel_source()) {
        return run_channel_worker(query, tracks, &cancelled, prepared);
    }
    let uses_util = queries
        .iter()
        .any(|q| q.referenced_metrics().iter().any(|m| m.is_util()));
    let uses_slip = queries
        .iter()
        .any(|q| q.referenced_metrics().iter().any(|m| m.is_slip()));
    // One derived-series set per track, so `with` parameters merge: the first
    // query to set mask/snr_drop/slip_window wins (queries rarely disagree).
    let params = merge_params(queries);

    let mut track_data: HashMap<TrackRef, TrackQueryData> = HashMap::new();
    for snapshot in tracks {
        if cancelled() {
            return RunCompleted {
                output: None,
                track_data,
            };
        }
        track_data.insert(
            snapshot.track_ref,
            compute_track_data(
                &snapshot.points,
                params,
                uses_util,
                uses_slip,
                snapshot.slice.start,
            ),
        );
        prepared.fetch_add(1, Ordering::Relaxed);
    }

    let providers: Vec<(TrackRef, SliceProvider<'_>)> = tracks
        .iter()
        .map(|snapshot| {
            let provider = provider_for(
                &snapshot.points,
                &snapshot.channels,
                track_data.get(&snapshot.track_ref),
            );
            (
                snapshot.track_ref,
                SliceProvider {
                    inner: provider,
                    start: snapshot.slice.start,
                    len: snapshot.slice.len(),
                },
            )
        })
        .collect();
    let inputs: Vec<TrackInput<'_, SliceProvider<'_>>> = providers
        .iter()
        .map(|(track_ref, provider)| TrackInput {
            track: *track_ref,
            provider,
        })
        .collect();

    RunCompleted {
        output: gt_query::run_pipeline(queries, &inputs, &cancelled).map(RunProduct::Points),
        track_data,
    }
}

/// Evaluate one channel-source `query` over each track's sample timeline. Nav
/// metrics are rejected on a channel source, so no derived series are needed.
/// Returns `None` output on cancellation, like the points path.
fn run_channel_worker(
    query: &CheckedQuery,
    tracks: &[TrackSnapshot],
    cancelled: &impl Fn() -> bool,
    prepared: &AtomicUsize,
) -> RunCompleted {
    let providers: Vec<(TrackRef, SliceProvider<'_>)> = tracks
        .iter()
        .map(|snapshot| {
            let provider = provider_for(&snapshot.points, &snapshot.channels, None);
            prepared.fetch_add(1, Ordering::Relaxed);
            (
                snapshot.track_ref,
                SliceProvider {
                    inner: provider,
                    start: snapshot.slice.start,
                    len: snapshot.slice.len(),
                },
            )
        })
        .collect();
    let inputs: Vec<TrackInput<'_, SliceProvider<'_>>> = providers
        .iter()
        .map(|(track_ref, provider)| TrackInput {
            track: *track_ref,
            provider,
        })
        .collect();

    let Some(output) = gt_query::run_cancellable(query, &inputs, cancelled) else {
        return RunCompleted {
            output: None,
            track_data: HashMap::new(),
        };
    };
    // The source channel is present (the query checked as a channel source).
    let name = query.source_channel().unwrap_or_default();
    // Component labels for the table headers, from any track carrying the
    // channel. Components are structural (the channel's shape), not per-track
    // content, so first-vs-last does not matter here unlike the unit collision
    // schema_from_files resolves last-wins.
    let components = tracks
        .iter()
        .flat_map(|s| &s.channels)
        .find(|c| c.name == name)
        .map(|c| c.components.clone())
        .unwrap_or_default();
    // Pair each track's matched sample ranges with its timeline, for the table.
    let track_results: Vec<ChannelTrackResult> = output
        .matches
        .into_iter()
        .filter_map(|tm| {
            let provider = providers.iter().find(|(tr, _)| *tr == tm.track)?;
            Some(ChannelTrackResult {
                track: tm.track,
                ranges: tm.ranges,
                timeline: provider.1.channel_timeline(name),
            })
        })
        .collect();

    // Project each track's matched sample spans onto its nav points, for the
    // map halos: a matched span bands the track segments it covers.
    let per_track: HashMap<TrackRef, (Vec<Range<usize>>, usize)> = track_results
        .iter()
        .filter_map(|result| {
            let snapshot = tracks.iter().find(|s| s.track_ref == result.track)?;
            let point_ranges =
                matched_point_ranges(&snapshot.points, &result.timeline, &result.ranges);
            let len = snapshot.points.len();
            (!point_ranges.is_empty()).then_some((result.track, (point_ranges, len)))
        })
        .collect();
    let matches = channel_query_matches(query.mode(), &per_track);

    RunCompleted {
        output: Some(RunProduct::Channel(ChannelRun {
            channel: name.to_owned(),
            components,
            summary: output.summary,
            tracks: track_results,
            matches,
        })),
        track_data: HashMap::new(),
    }
}

/// Map one track's matched sample ranges to enclosing nav-point index ranges,
/// so the point-halo renderer bands the track over each matched span. A span
/// `[t0, t1]` extends to the nav point at or before `t0` through the one at or
/// after `t1`, so even a sub-interval match bands the segment it sits on.
/// Returned sorted and merged (disjoint), as [`QueryMatches`] requires.
fn matched_point_ranges(
    points: &[NavPoint],
    timeline: &ChannelTimeline,
    ranges: &[Range<usize>],
) -> Vec<Range<usize>> {
    let Some(last) = points.len().checked_sub(1) else {
        return Vec::new();
    };
    let point_secs: Vec<f64> = points.iter().map(|p| p.tpv.time().as_secs_f64()).collect();
    let mut spans: Vec<Range<usize>> = ranges
        .iter()
        .filter_map(|r| {
            let t0 = *timeline.times.get(r.start)?;
            let t1 = *timeline.times.get(r.end.checked_sub(1)?)?;
            // Last point at or before t0 (or the first point); first point at or
            // after t1 (or the last).
            let lo = point_secs.partition_point(|&t| t <= t0).saturating_sub(1);
            let hi = point_secs.partition_point(|&t| t < t1).min(last);
            Some(lo..hi + 1)
        })
        .collect();
    spans.sort_by_key(|r| r.start);
    merge_ranges(spans)
}

/// Merge overlapping or touching ranges (assumes `ranges` sorted by start),
/// yielding sorted, disjoint, non-empty ranges.
fn merge_ranges(ranges: Vec<Range<usize>>) -> Vec<Range<usize>> {
    let mut merged: Vec<Range<usize>> = Vec::with_capacity(ranges.len());
    for range in ranges.into_iter().filter(|r| !r.is_empty()) {
        match merged.last_mut() {
            Some(last) if range.start <= last.end => last.end = last.end.max(range.end),
            _ => merged.push(range),
        }
    }
    merged
}

/// The complement of sorted, disjoint `ranges` within `0..len`: the gaps not
/// covered by any range.
fn complement_ranges(ranges: &[Range<usize>], len: usize) -> Vec<Range<usize>> {
    let mut gaps = Vec::new();
    let mut cursor = 0;
    for range in ranges {
        if range.start > cursor {
            gaps.push(cursor..range.start);
        }
        cursor = cursor.max(range.end);
    }
    if cursor < len {
        gaps.push(cursor..len);
    }
    gaps
}

/// Build the map effect for a channel-source run from each track's matched
/// nav-point ranges and point count, honoring the query's display mode:
/// `draw` halos the matched segments, `hide` breaks the polyline there, and
/// `keep` breaks it everywhere else.
fn channel_query_matches(
    mode: DisplayMode,
    per_track: &HashMap<TrackRef, (Vec<Range<usize>>, usize)>,
) -> QueryMatches {
    let matched: HashMap<TrackRef, Vec<Range<usize>>> = per_track
        .iter()
        .map(|(track, (ranges, _))| (*track, ranges.clone()))
        .collect();
    match mode {
        DisplayMode::Draw => QueryMatches {
            draws: vec![DrawLayer {
                color: 0,
                ranges: matched,
            }],
            ..QueryMatches::default()
        },
        DisplayMode::Hide => QueryMatches {
            hidden: matched,
            ..QueryMatches::default()
        },
        DisplayMode::Keep => QueryMatches {
            hidden: per_track
                .iter()
                .map(|(track, (ranges, len))| (*track, complement_ranges(ranges, *len)))
                .filter(|(_, gaps)| !gaps.is_empty())
                .collect(),
            ..QueryMatches::default()
        },
    }
}

/// Merge the `with` parameters of every query, taking the first value set for
/// each. The derived util/slip series are computed once per track, so a later
/// query that declares a different mask reuses the first (a rare conflict).
fn merge_params(queries: &[CheckedQuery]) -> gt_query::Params {
    let mut merged = gt_query::Params::default();
    for query in queries {
        let params = query.params();
        merged.mask_deg = merged.mask_deg.or(params.mask_deg);
        merged.snr_drop_db_hz = merged.snr_drop_db_hz.or(params.snr_drop_db_hz);
        merged.slip_window_s = merged.slip_window_s.or(params.slip_window_s);
    }
    merged
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

/// The shared inputs a query's match tables read: the files, the run's derived
/// series, and the query's columns.
struct MatchCtx<'a> {
    files: &'a [LoadedFile],
    track_data: &'a HashMap<TrackRef, TrackQueryData>,
    columns: &'a [QueryMetric],
}

/// Map requests a match-table interaction can raise, mirroring the side
/// panel's `PanelContext` fields: clicking a row pins its point popup,
/// double-clicking centers the map on the point.
pub struct MatchMapRequests<'a> {
    pub map_center: &'a mut Option<(f64, f64)>,
    pub popup_pos: &'a mut Option<egui::Pos2>,
}

/// One match: a collapsing header with the point table inside. Header hover
/// echoes the whole match on the map (a halo band plus track focus) and on
/// the plot (a shaded time band, via the app layer); row hover echoes the
/// single point through the plot cross-highlight ring.
fn match_ui(
    ui: &mut egui::Ui,
    ctx: &MatchCtx<'_>,
    track_ref: TrackRef,
    range: &Range<usize>,
    stale: bool,
    highlight: &mut MapHighlight,
    requests: &mut MatchMapRequests<'_>,
) {
    let header = match_header_text(ctx.files, track_ref, range);
    let id = ui.id().with(("query_match", track_ref, range.start));
    if stale {
        // Grayed out, not hidden: the rows reference point indices that may
        // no longer address the same data.
        ui.add_enabled(false, egui::Label::new(header))
            .on_disabled_hover_text(format!("Data changed since this run {EM_DASH} run again"));
        return;
    }
    let response = egui::CollapsingHeader::new(header)
        .id_salt(id)
        .show(ui, |ui| {
            match_table_ui(ui, ctx, track_ref, range, highlight, requests);
        });
    if response.header_response.hovered() {
        highlight.hover_match = Some(MatchHighlight::new(track_ref, range));
        // Track focus alongside the band: the map fades the other tracks and
        // the plot dims their series, like hovering the track in the side
        // panel.
        highlight.hover = Some(HighlightScope::Track(track_ref));
    }
}

fn match_table_ui(
    ui: &mut egui::Ui,
    ctx: &MatchCtx<'_>,
    track_ref: TrackRef,
    range: &Range<usize>,
    highlight: &mut MapHighlight,
    requests: &mut MatchMapRequests<'_>,
) {
    let columns = ctx.columns;
    let Some(points) = points_of(ctx.files, track_ref) else {
        return;
    };
    let data = ctx.track_data.get(&track_ref);
    // The match table reads only metric columns (channels cannot be columns
    // yet), so it needs no channel data.
    let provider = provider_for(points, &[], data);
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
            for column in columns {
                ui.strong(column.to_string());
            }
            ui.end_row();

            for pi in range.clone().take(MATCH_TABLE_ROW_CAP) {
                // The cell responses union into one row response, so the row
                // reacts as a whole wherever it is hovered or clicked.
                let mut row_response: Option<egui::Response> = None;
                for column in columns {
                    let value = if *column == QueryMetric::Accel {
                        pi.checked_sub(slice_start)
                            .and_then(|rel| gt_query::derived_accel(&slice, rel))
                    } else {
                        provider.value(*column, pi)
                    };
                    let response = ui.add(
                        egui::Label::new(format_value(*column, value)).sense(egui::Sense::click()),
                    );
                    row_response = Some(match row_response {
                        Some(row) => row.union(response),
                        None => response,
                    });
                }
                if let Some(response) = row_response {
                    if response.hovered() {
                        // Echo the hovered row on the map, same ring as the
                        // plot cursor cross-highlight.
                        highlight.plot_hover_point =
                            Some((track_ref.fi, track_ref.index, PointIdx::new(pi)));
                    }
                    if let Some(p) = points.get(pi) {
                        // The side panel's point-row semantics: click pins the
                        // point's map popup, double-click centers the map.
                        apply_point_click(
                            ui,
                            &response,
                            DataPointRef {
                                track: track_ref,
                                category: DataCategory::Tpv,
                                point_index: PointIdx::new(pi),
                            },
                            (p.tpv.lat().as_degrees(), p.tpv.lon().as_degrees()),
                            highlight,
                            requests.map_center,
                            requests.popup_pos,
                        );
                    }
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

/// Render a points pipeline's per-query sections with their point match tables.
fn points_results_ui(
    ui: &mut egui::Ui,
    points: &PointsResults,
    files: &[LoadedFile],
    highlight: &mut MapHighlight,
    requests: &mut MatchMapRequests<'_>,
) {
    let stale = points.matches.stale;
    // One collapsible section per query, in editor order: its summary is the
    // header (with a color swatch for draw queries), its match tables the body.
    // Stable ids keep the open/closed state across reruns.
    for (qi, query) in points.queries.iter().enumerate() {
        let matches = query
            .matches
            .iter()
            .map(|tm| tm.ranges.len())
            .sum::<usize>();
        let match_ctx = MatchCtx {
            files,
            track_data: &points.track_data,
            columns: &query.columns,
        };
        let id = ui.make_persistent_id(("query_result", qi));
        egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, true)
            .show_header(ui, |ui| {
                if let Some(color) = query.color {
                    query_swatch(ui, gt_ui_theme::query_halo_color(color, false));
                }
                ui.label(query.summary.as_str());
            })
            .body(|ui| {
                if matches == 0 {
                    ui.label(egui::RichText::new("No matches").weak());
                }
                for tm in &query.matches {
                    for range in &tm.ranges {
                        match_ui(ui, &match_ctx, tm.track, range, stale, highlight, requests);
                    }
                }
            });
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
                ui.label(egui::RichText::new("No matches").weak());
            }
            for track in &channel.tracks {
                channel_track_ui(ui, channel, track, files);
            }
        });
}

/// One track's matched channel samples as a table: `time` plus one column per
/// component (or the channel name for a scalar). Capped like the point tables.
/// Values print in the evaluator's base units (a `g` channel shows m/s2), the
/// same convention the query language and the provider timeline use.
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
        egui::RichText::new(format!(
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
    egui::Grid::new(ui.id().with(("channel_table", track.track)))
        .striped(true)
        .show(ui, |ui| {
            ui.strong("time");
            for header in &value_headers {
                ui.strong(header.as_str());
            }
            ui.end_row();

            let columns = track.timeline.columns.max(1);
            for sample in track
                .ranges
                .iter()
                .flat_map(Clone::clone)
                .take(MATCH_TABLE_ROW_CAP)
            {
                let time = track
                    .timeline
                    .times
                    .get(sample)
                    .and_then(|t| {
                        DateTime::<Utc>::from_timestamp_micros((t * MICROS_PER_SEC) as i64)
                    })
                    .map(|dt| dt.format("%H:%M:%S%.3f").to_string())
                    .unwrap_or_default();
                ui.label(time);
                for col in 0..columns {
                    let value = track.timeline.values.get(sample * columns + col);
                    ui.label(value.map_or_else(String::new, |v| format!("{v:.3}")));
                }
                ui.end_row();
            }
        });
    if matched > MATCH_TABLE_ROW_CAP {
        ui.label(
            egui::RichText::new(format!(
                "{EM_DASH} {} more samples",
                matched - MATCH_TABLE_ROW_CAP
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

fn points_of(files: &[LoadedFile], track_ref: TrackRef) -> Option<&[NavPoint]> {
    track_ref.resolve(files).map(|t| t.points.as_slice())
}

/// The provider both the run and the match tables read through - one code
/// path, so tables always show the values the evaluator saw.
fn provider_for<'a>(
    points: &'a [NavPoint],
    channels: &'a [Channel],
    data: Option<&'a TrackQueryData>,
) -> TrackProvider<'a> {
    TrackProvider {
        points,
        channels,
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

    fn channel_span(&self, name: &str, t_lo: f64, t_hi: f64) -> ChannelSamples {
        // Channel samples are keyed by absolute time, so the slice's time span
        // selects them directly from the inner provider - no index offset.
        self.inner.channel_span(name, t_lo, t_hi)
    }

    fn channel_timeline(&self, name: &str) -> ChannelTimeline {
        // A channel's own sample clock is independent of the point slice, so it
        // forwards whole.
        self.inner.channel_timeline(name)
    }
}

/// Project a points [`PipelineOutput`] into the panel/map result. Evaluation
/// ran on each track's time-filtered slice, so point indices shift back to
/// absolute positions here.
fn points_results(
    output: &PipelineOutput,
    track_data: HashMap<TrackRef, TrackQueryData>,
) -> PointsResults {
    let absolute = |tms: &[TrackMatches]| -> HashMap<TrackRef, Vec<Range<usize>>> {
        tms.iter()
            .filter(|tm| !tm.ranges.is_empty())
            .map(|tm| {
                let start = track_data.get(&tm.track).map_or(0, |d| d.slice_start);
                let ranges = tm
                    .ranges
                    .iter()
                    .map(|r| r.start + start..r.end + start)
                    .collect();
                (tm.track, ranges)
            })
            .collect()
    };

    // The point-key mask distinguishes only so many halo layers; extra draw
    // queries beyond that cannot be rendered distinctly.
    if output.draws.len() > DrawLayerMask::MAX_LAYERS {
        log::warn!(
            "query has {} draw stages; only the first {} render as halos",
            output.draws.len(),
            DrawLayerMask::MAX_LAYERS
        );
    }

    // The i-th draw query gets palette color i; the map keys its halo layer to
    // the same order.
    let draw_color: HashMap<usize, usize> = output
        .draws
        .iter()
        .take(DrawLayerMask::MAX_LAYERS)
        .enumerate()
        .map(|(order, layer)| (layer.query_index, order))
        .collect();

    let matches = QueryMatches {
        hidden: absolute(&output.hidden),
        draws: output
            .draws
            .iter()
            .take(DrawLayerMask::MAX_LAYERS)
            .enumerate()
            .map(|(color, layer)| DrawLayer {
                color,
                ranges: absolute(&layer.matches),
            })
            .collect(),
        stale: false,
    };
    let queries = output
        .queries
        .iter()
        .enumerate()
        .map(|(qi, q)| PanelQuery {
            color: draw_color.get(&qi).copied(),
            summary: summary_line(&q.summary, q.mode),
            columns: q.columns.clone(),
            matches: q
                .matches
                .iter()
                .map(|tm| TrackMatches {
                    track: tm.track,
                    ranges: absolute(std::slice::from_ref(tm))
                        .remove(&tm.track)
                        .unwrap_or_default(),
                })
                .collect(),
        })
        .collect();

    PointsResults {
        matches,
        queries,
        track_data,
    }
}

/// Project a channel-source [`ChannelRun`] into its panel result. The matched
/// ranges are sample indices into each track's timeline, kept as-is (no slice
/// offset: a channel source is not sliced by the point time filter).
fn channel_results(run: ChannelRun) -> ChannelResults {
    ChannelResults {
        channel: run.channel,
        components: run.components,
        summary: channel_summary_line(&run.summary),
        tracks: run.tracks,
        matches: run.matches,
    }
}

/// A channel-source run's summary line: match count over tracks, plus any
/// skipped-window reporting. Matches are samples, so it never mentions the
/// points/keep/hide accounting the [`summary_line`] points path uses.
fn channel_summary_line(summary: &RunSummary) -> String {
    let mut parts = vec![format!(
        "{} {} on {} {}",
        summary.match_count,
        gt_fmt::pluralize(summary.match_count, "match", "matches"),
        summary.tracks_with_matches,
        gt_fmt::pluralize(summary.tracks_with_matches, "track", "tracks"),
    )];
    for (channel, count) in &summary.skipped_channels {
        parts.push(format!("{count} skipped (missing @{channel})"));
    }
    if summary.skipped_non_finite > 0 {
        parts.push(format!(
            "{} skipped (undefined arithmetic)",
            summary.skipped_non_finite
        ));
    }
    parts.join(&format!(" {EM_DASH} "))
}

fn summary_line(summary: &RunSummary, mode: DisplayMode) -> String {
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
    channels: &'a [Channel],
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
        Some(percent * Unit::PERCENT.to_base())
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

    /// Locate channel `name` and the two things both channel readers need: its
    /// column count and the factor converting its stored unit to base units. An
    /// unknown or absent unit leaves values a bare number (factor 1.0), matching
    /// how the checker types such a channel; components share the channel unit.
    fn resolve_channel(&self, name: &str) -> Option<(&Channel, usize, f64)> {
        let channel = self.channels.iter().find(|c| c.name == name)?;
        let to_base = channel
            .unit
            .as_deref()
            .and_then(Unit::from_label)
            .map_or(1.0, Unit::to_base);
        Some((channel, channel.component_count(), to_base))
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

    /// A channel's samples whose timestamp lands in `[t_lo, t_hi]`, as row-major
    /// rows (one column per component, one for a scalar channel), converted from
    /// the channel's stored unit to the evaluator's base units.
    ///
    /// `t_lo`/`t_hi` arrive floored to whole seconds (the query engine's time
    /// resolution, since nav-point time floors to whole seconds); the sub-second
    /// precision of a sample's own timestamp only refines placement within that
    /// grid.
    fn channel_span(&self, name: &str, t_lo: f64, t_hi: f64) -> ChannelSamples {
        let Some((channel, columns, to_base)) = self.resolve_channel(name) else {
            return ChannelSamples::default();
        };
        // `times` is sorted ascending, so the samples in the closed span are a
        // contiguous row range found by binary search. An inverted span (`t_lo >
        // t_hi`, possible when the track's time is non-monotonic) makes the range
        // empty rather than panicking. Values are row-major `[rows, columns]`, so
        // row `r`'s columns are `r*columns .. (r+1)*columns`.
        let secs = |time: &DateTime<Utc>| time.timestamp_micros() as f64 / MICROS_PER_SEC;
        let lo = channel.times.partition_point(|time| secs(time) < t_lo);
        let hi = channel.times.partition_point(|time| secs(time) <= t_hi);
        let values = channel
            .values
            .get(lo * columns..hi * columns)
            .unwrap_or_default()
            .iter()
            .map(|value| value * to_base)
            .collect();
        ChannelSamples { values, columns }
    }

    /// The whole sample timeline of `name` in base units, for a query whose
    /// source is that channel. Each sample's time is its own (sub-second)
    /// clock, since the channel is the timeline here rather than being bucketed
    /// onto nav-point seconds.
    fn channel_timeline(&self, name: &str) -> ChannelTimeline {
        let Some((channel, columns, to_base)) = self.resolve_channel(name) else {
            return ChannelTimeline::default();
        };
        ChannelTimeline {
            times: channel
                .times
                .iter()
                .map(|t| t.timestamp_micros() as f64 / MICROS_PER_SEC)
                .collect(),
            values: channel.values.iter().map(|value| value * to_base).collect(),
            columns,
        }
    }
}

/// The schema the editor checks against: every scalar or vector channel across
/// the loaded files, keyed by name. A channel is queryable if any loaded track
/// carries it; a run over a track lacking it reports the window as skipped. On
/// a name collision the last track's metadata wins (channels rarely collide,
/// and a debug tool need not choose between conflicting units).
fn schema_from_files(files: &[LoadedFile]) -> ChannelSchema {
    use uom::si::angle::degree;

    let mut schema = ChannelSchema::new();
    for file in files {
        for channel in file.tracks.iter().flat_map(|t| &t.channels) {
            schema.insert(
                &channel.name,
                ChannelInfo {
                    unit: channel.unit.clone(),
                    period_deg: channel.period.map(|p| p.get::<degree>()),
                    components: channel.components.clone(),
                },
            );
        }
    }
    schema
}

fn check_text(text: &str, schema: &ChannelSchema) -> Result<CheckedQuery, Diagnostic> {
    gt_query::check(&gt_query::parse(text)?, schema)
}

/// Parse and check every query in the editor against the loaded channels.
/// Queries are separated by a blank line; each chunk keeps its byte range so
/// diagnostics and the caret map back to editor coordinates.
///
/// A chunk holding no code - a standalone comment paragraph - is not a query:
/// it is skipped rather than checked, so a block comment between queries
/// neither errors nor blocks Run.
fn check_all(text: &str, schema: &ChannelSchema) -> Vec<Chunk> {
    split_queries(text)
        .into_iter()
        .filter_map(|range| {
            let src = text.get(range.clone()).unwrap_or("");
            // Comment-only via the highlighter's classes rather than the
            // parsing tokenizer: the latter also drops rejected characters,
            // which must still be checked (and reported), not skipped.
            let comment_only = lexer::highlight_classes(src)
                .iter()
                .all(|(_, class)| *class == TokenClass::Comment);
            if comment_only {
                return None;
            }
            Some(Chunk {
                result: check_text(src, schema),
                range,
            })
        })
        .collect()
}

/// The byte range of the text the caret's completions analyze: the query
/// chunk containing the caret; or, when the caret sits on the line directly
/// after a chunk (an Enter pressed to continue it with `| …`), that chunk
/// extended to the caret so continuation typing is analyzed in context;
/// otherwise an empty context at the caret (a fresh query).
fn analysis_context(text: &str, caret: usize) -> Range<usize> {
    let mut preceding: Option<Range<usize>> = None;
    for range in split_queries(text) {
        if range.start <= caret && caret <= range.end {
            return range;
        }
        if range.end < caret {
            preceding = Some(range);
        }
    }
    // Directly after a chunk means only whitespace with at most one newline in
    // between: a second newline is the blank-line separator, so the caret
    // starts a fresh query.
    if let Some(range) = preceding
        && let Some(gap) = text.get(range.end..caret)
        && gap.chars().all(char::is_whitespace)
        && gap.matches('\n').count() <= 1
    {
        return range.start..caret;
    }
    caret..caret
}

/// Byte ranges of the blank-line-separated queries in `text`. Each range spans
/// from a query's first non-blank line to the end of its last non-blank line.
fn split_queries(text: &str) -> Vec<Range<usize>> {
    let mut chunks = Vec::new();
    let mut current: Option<Range<usize>> = None;
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        let start = offset;
        offset += line.len();
        if line.trim().is_empty() {
            if let Some(range) = current.take() {
                chunks.push(range);
            }
        } else {
            let content_end = start + line.trim_end().len();
            match &mut current {
                Some(range) => range.end = content_end,
                None => current = Some(start..content_end),
            }
        }
    }
    if let Some(range) = current {
        chunks.push(range);
    }
    chunks
}

/// Byte offset of the `char_index`-th character, or the text length when the
/// index is at or past the end. The egui caret is a char index; the query
/// position model works in bytes.
fn char_to_byte(text: &str, char_index: usize) -> usize {
    text.char_indices()
        .nth(char_index)
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
    // above; `constrain` still clamps any residual overshoot.
    let visible_rows = autocomplete.items.len().clamp(1, AUTOCOMPLETE_VISIBLE_ROWS);
    let footer = if overflow > 0 { row_height } else { 0.0 };
    let frame_padding = egui::Frame::popup(ui.style()).total_margin().sum().y;
    let est_height = row_height * visible_rows as f32 + footer + frame_padding;
    let pos = if autocomplete.caret_pos.y + est_height > ui.ctx().content_rect().bottom() {
        autocomplete.caret_top - egui::vec2(0.0, est_height)
    } else {
        autocomplete.caret_pos
    };

    egui::Area::new(editor_id.with("autocomplete"))
        .order(egui::Order::Foreground)
        .fixed_pos(pos)
        .constrain(true)
        .show(ui.ctx(), |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_min_width(220.0);
                ui.set_max_width(380.0);
                if let Some(notice) = autocomplete.notice {
                    ui.label(egui::RichText::new(notice).weak().italics());
                    return;
                }
                egui::ScrollArea::vertical()
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
        ui.label(egui::RichText::new(kind_label).weak().small());
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
        TokenClass::Error => gt_ui_theme::error_indicator(dark_mode),
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
    let error_color = gt_ui_theme::error_indicator(ui.visuals().dark_mode);
    let mut job = LayoutJob::default();
    job.append(
        &format!("{} ", egui_phosphor::regular::WARNING_OCTAGON),
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
        Quantity::Ratio => format!("{:.0} %", v / Unit::PERCENT.to_base()),
        Quantity::Rate => format!("{v:.2}/min"),
        Quantity::Condition => EM_DASH.to_owned(),
    }
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
                egui::Stroke::new(2.0, error_color)
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
        let provider = provider_for(&points, &[], None);
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
            // Runs without panicking; util/slip series are absent here, which
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
        assert_eq!(shown.chars().count(), HISTORY_LINE_MAX_CHARS + 1);
    }

    #[test]
    fn split_queries_handles_blank_line_edge_cases() {
        // Built from arguments so single-element `vec![r(0, 1)]` does not trip
        // clippy's `single_range_in_vec_init`.
        let r = |start: usize, end: usize| start..end;
        let cases: &[(&str, Vec<Range<usize>>)] = &[
            // A blank line separates two queries.
            ("a\n\nb", vec![r(0, 1), r(3, 4)]),
            // Leading and trailing blank lines are dropped.
            ("\n\na\n\n", vec![r(2, 3)]),
            // A whitespace-only line still separates.
            ("a\n   \nb", vec![r(0, 1), r(6, 7)]),
            // A single chunk without a trailing newline.
            ("a", vec![r(0, 1)]),
            // Adjacent non-blank lines are one multi-line query, and the range
            // ends at the last line's trimmed content.
            ("points\n| draw", vec![r(0, 13)]),
        ];
        for (text, want) in cases {
            assert_eq!(&split_queries(text), want, "input: {text:?}");
        }
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
        let provider = provider_for(&points, &[], Some(&data));

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
            inner: provider_for(&points, &[], None),
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
            &gt_query::ChannelSchema::new(),
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
        let line = summary_line(&output.summary, DisplayMode::Draw);
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
        let query = gt_query::check(
            &gt_query::parse("points | where velocity > 30 km/h").unwrap(),
            &gt_query::ChannelSchema::new(),
        )
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
        assert!(summary_line(&output.summary, DisplayMode::Keep).contains("3 of 5 points hidden"));
        assert!(summary_line(&output.summary, DisplayMode::Hide).contains("2 of 5 points hidden"));
        assert!(!summary_line(&output.summary, DisplayMode::Draw).contains("hidden"));
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

    /// The Unix epoch the test fixtures place their first sample at, matching
    /// `test_points`.
    const TEST_EPOCH: i64 = 1_700_000_000;

    /// A scalar channel named `name` with `unit`, sampled at `TEST_EPOCH + secs`
    /// for each `(secs, value)` pair.
    fn scalar_channel(name: &str, unit: Option<&str>, samples: &[(i64, f64)]) -> Channel {
        Channel {
            name: name.to_owned(),
            unit: unit.map(str::to_owned),
            period: None,
            description: None,
            components: vec![],
            times: samples
                .iter()
                .map(|&(secs, _)| {
                    DateTime::from_timestamp(TEST_EPOCH + secs, 0).expect("valid timestamp")
                })
                .collect(),
            values: samples.iter().map(|&(_, value)| value).collect(),
        }
    }

    /// A 3-component vector channel, each row `[x, y, z]` at `TEST_EPOCH + secs`.
    fn vector_channel(
        name: &str,
        unit: Option<&str>,
        components: &[&str],
        samples: &[(i64, [f64; 3])],
    ) -> Channel {
        Channel {
            name: name.to_owned(),
            unit: unit.map(str::to_owned),
            period: None,
            description: None,
            components: components.iter().map(|c| (*c).to_owned()).collect(),
            times: samples
                .iter()
                .map(|&(secs, _)| {
                    DateTime::from_timestamp(TEST_EPOCH + secs, 0).expect("valid timestamp")
                })
                .collect(),
            values: samples.iter().flat_map(|&(_, row)| row).collect(),
        }
    }

    #[test]
    fn channel_span_converts_units_and_filters_time() {
        // A g-valued scalar accel channel: channel_span converts each sample to
        // base m/s2 and keeps only those whose absolute time lands in the span.
        let base = TEST_EPOCH as f64;
        let accel = scalar_channel(
            "accel",
            Some("g"),
            &[(0, 1.0), (1, 1.5), (2, 2.0), (3, 0.5)],
        );
        let channels = [accel];
        let points = test_points();
        let provider = provider_for(&points, &channels, None);

        let got = provider.channel_span("accel", base, base + 2.0);
        // The first three samples (the fourth is past t_hi), each g -> m/s2, one
        // column (scalar).
        let g = Unit::G.to_base();
        let want = [1.0 * g, 1.5 * g, 2.0 * g];
        assert_eq!(got.columns, 1);
        assert_eq!(got.values.len(), want.len());
        for (a, b) in got.values.iter().zip(want) {
            assert!((a - b).abs() < 1e-9, "{a} != {b}");
        }
    }

    #[test]
    fn channel_span_reads_vector_rows() {
        // A vector channel returns row-major values, all columns per row, each
        // converted to base; an unknown channel yields nothing.
        let base = TEST_EPOCH as f64;
        let accel = vector_channel(
            "accel",
            Some("g"),
            &["x", "y", "z"],
            &[(0, [1.0, 2.0, 3.0]), (1, [1.1, 2.2, 3.3])],
        );
        let channels = [accel];
        let points = test_points();
        let provider = provider_for(&points, &channels, None);

        let g = Unit::G.to_base();
        let got = provider.channel_span("accel", base, base + 1.0);
        assert_eq!(got.columns, 3);
        let want = [1.0, 2.0, 3.0, 1.1, 2.2, 3.3];
        assert_eq!(got.values.len(), want.len());
        for (a, raw) in got.values.iter().zip(want) {
            assert!((a - raw * g).abs() < 1e-9, "{a}");
        }
        assert!(
            provider
                .channel_span("missing", 0.0, f64::MAX)
                .values
                .is_empty()
        );
    }

    #[test]
    fn slice_provider_channel_span_ignores_the_index_offset() {
        // Channels are absolute-time-keyed, so a SliceProvider selects the same
        // samples as its inner provider regardless of the point-index start.
        let base = TEST_EPOCH as f64;
        let accel = scalar_channel("accel", Some("g"), &[(0, 1.0), (1, 1.5), (2, 2.0)]);
        let channels = [accel];
        let points = test_points();
        let inner = provider_for(&points, &channels, None);
        let slice = SliceProvider {
            inner,
            start: 1,
            len: 1,
        };

        // The span [base, base+1] holds the first two samples through either
        // provider; the slice's start must not shift the time window.
        assert_eq!(
            slice.channel_span("accel", base, base + 1.0),
            inner.channel_span("accel", base, base + 1.0),
        );
        let g = Unit::G.to_base();
        let want = [1.0 * g, 1.5 * g];
        let got = slice.channel_span("accel", base, base + 1.0);
        assert_eq!(got.values.len(), want.len());
        for (a, b) in got.values.iter().zip(want) {
            assert!((a - b).abs() < 1e-9, "{a} != {b}");
        }
    }

    /// A single-track file carrying `channels` over `test_points`.
    fn file_with_channels(channels: Vec<Channel>) -> LoadedFile {
        use gt_types::{FileMetadata, FileSource, LoadedTrack, TrackLod, TrackMetadata};

        LoadedFile {
            metadata: FileMetadata::default(),
            tracks: vec![LoadedTrack {
                metadata: TrackMetadata::default(),
                points: test_points(),
                lod: TrackLod::default(),
                sat_label_anchors: Vec::new(),
                custom_markers: vec![],
                generated_markers: vec![],
                event_markers: vec![],
                channels,
            }],
            event_marker_styles: HashMap::new(),
            orphaned_event_markers: vec![],
            source: FileSource::GtdBytes(Arc::from(Vec::<u8>::new())),
            load_warnings: vec![],
        }
    }

    #[test]
    fn schema_from_files_types_a_channel_for_the_editor() {
        // A loaded g-unit accel channel resolves to an acceleration in the
        // editor: it compares to an acceleration literal and rejects a speed.
        let files = [file_with_channels(vec![scalar_channel(
            "accel",
            Some("g"),
            &[(0, 1.0)],
        )])];
        let schema = schema_from_files(&files);

        check_text("points | window 2 | where max(@accel) > 1 g", &schema)
            .expect("a g channel checks against an acceleration literal");
        let err = check_text("points | window 2 | where max(@accel) > 30 km/h", &schema)
            .expect_err("an acceleration cannot compare to a speed");
        assert!(err.message.contains("acceleration"), "{}", err.message);
    }

    #[test]
    fn channel_timeline_serves_the_whole_channel_in_base_units() {
        // The channel-source timeline carries every sample's time and value,
        // converted to base units (g -> m/s2), independent of the point slice.
        let base = TEST_EPOCH as f64;
        let accel = scalar_channel("accel", Some("g"), &[(0, 1.0), (1, 2.0)]);
        let channels = [accel];
        let points = test_points();
        let provider = provider_for(&points, &channels, None);

        let timeline = provider.channel_timeline("accel");
        assert_eq!(timeline.columns, 1);
        assert_eq!(timeline.times.len(), 2);
        assert!((timeline.times[0] - base).abs() < 1e-6);
        let g = Unit::G.to_base();
        assert!((timeline.values[0] - g).abs() < 1e-9);
        assert!((timeline.values[1] - 2.0 * g).abs() < 1e-9);
        // Unknown channel yields an empty timeline.
        assert!(provider.channel_timeline("missing").times.is_empty());
    }

    #[test]
    fn a_channel_source_runs_over_its_own_samples() {
        // The whole channel-source app path: run `@accel | where ...` over the
        // channel's timeline. Two of three samples clear 1 g, matching per
        // sample (indices 0 and 2), with the sample count as the total.
        let channel = scalar_channel("accel", Some("g"), &[(0, 1.5), (1, 0.2), (2, 2.0)]);
        let files = [file_with_channels(vec![channel.clone()])];
        let schema = schema_from_files(&files);
        let query = check_text("@accel | where @accel > 1 g", &schema)
            .expect("a channel source checks against the loaded schema");
        assert!(query.is_channel_source());

        let points = test_points();
        let channels = [channel];
        let provider = provider_for(&points, &channels, None);
        let output = gt_query::run(
            &query,
            &[TrackInput {
                track: TrackRef::new(FileIdx::new(0), TrackIdx::new(0)),
                provider: &provider,
            }],
        );
        assert_eq!(output.matches[0].ranges, vec![0..1, 2..3]);
        assert_eq!(output.summary.match_count, 2);
        // Three channel samples were the timeline, not the two nav points.
        assert_eq!(output.summary.total_points, 3);
    }

    /// A `QueryWindow` whose editor holds `text`, checked against `schema`, for
    /// exercising `run_kind` without stepping the egui harness.
    fn window_with(text: &str, schema: &ChannelSchema) -> QueryWindow {
        let mut window = QueryWindow::new();
        window.chunks = check_all(text, schema);
        window
    }

    #[rstest]
    // Every query points-source: the composing pipeline.
    #[case("points | where velocity > 1 km/h", RunKind::Points)]
    // A lone channel source: standalone run.
    #[case("@accel | where norm(@accel) > 1 g", RunKind::Channel)]
    // A channel source mixed with a points query: disallowed.
    #[case(
        "points | where velocity > 1 km/h\n\n@accel | where norm(@accel) > 1 g",
        RunKind::MixedChannel
    )]
    fn run_kind_classifies_the_editor_queries(#[case] text: &str, #[case] expected: RunKind) {
        let files = [file_with_channels(vec![vector_channel(
            "accel",
            Some("g"),
            &["x", "y", "z"],
            &[(0, [1.0, 0.0, 0.0])],
        )])];
        let schema = schema_from_files(&files);
        let window = window_with(text, &schema);
        assert!(window.all_ok(), "fixture queries must check");
        assert_eq!(window.run_kind(), expected);
    }

    #[test]
    fn run_channel_worker_pairs_matches_with_the_timeline() {
        // The worker branch for a channel source: run it standalone and package
        // per-track matched sample ranges with the channel's timeline and
        // component labels. Sample 0 (1.5 g -> 14.7 m/s2) clears 1 g; sample 1
        // (0.2 g -> 1.96) does not.
        let channel = vector_channel(
            "accel",
            Some("g"),
            &["x", "y", "z"],
            &[(0, [1.5, 0.0, 0.0]), (1, [0.2, 0.0, 0.0])],
        );
        let files = [file_with_channels(vec![channel.clone()])];
        let schema = schema_from_files(&files);
        let query = check_text("@accel | where norm(@accel) > 1 g", &schema).unwrap();

        let points = test_points();
        let snapshot = TrackSnapshot {
            track_ref: TrackRef::new(FileIdx::new(0), TrackIdx::new(0)),
            slice: 0..points.len(),
            points,
            channels: vec![channel],
        };
        let cancel = AtomicBool::new(false);
        let prepared = AtomicUsize::new(0);
        let completed = run_worker(&[query], &[snapshot], &cancel, &prepared);

        let Some(RunProduct::Channel(run)) = completed.output else {
            panic!("a channel source produces a channel run");
        };
        assert_eq!(run.channel, "accel");
        assert_eq!(run.components, vec!["x", "y", "z"]);
        assert_eq!(run.tracks.len(), 1);
        assert_eq!(run.tracks[0].ranges, vec![0..1]);
        assert_eq!(run.tracks[0].timeline.times.len(), 2);
        // The matched sample (at t=0 s) bands the track: a draw halo over the
        // enclosing nav-point range.
        let track = TrackRef::new(FileIdx::new(0), TrackIdx::new(0));
        assert_eq!(run.matches.draws.len(), 1);
        assert_eq!(run.matches.draws[0].ranges_for(track), [rng(0, 1)]);
    }

    #[test]
    fn matched_point_ranges_band_the_covering_segment() {
        // Points at 0 and 1 s; a channel sample matched at 0.5 s brackets to the
        // segment between them, banding both nav points.
        let base = TEST_EPOCH as f64;
        let points = test_points();
        let timeline = ChannelTimeline {
            times: vec![base + 0.5],
            values: vec![9.8],
            columns: 1,
        };
        assert_eq!(
            matched_point_ranges(&points, &timeline, &[rng(0, 1)]),
            vec![rng(0, 2)]
        );
        // No matched samples yields no bands.
        assert!(matched_point_ranges(&points, &timeline, &[]).is_empty());
    }

    #[test]
    fn merge_ranges_merges_touching_and_overlapping() {
        assert_eq!(merge_ranges(vec![0..2, 2..4, 5..6]), vec![0..4, 5..6]);
        assert_eq!(merge_ranges(vec![0..3, 1..2]), vec![rng(0, 3)]);
        assert!(merge_ranges(vec![]).is_empty());
    }

    #[test]
    fn complement_ranges_returns_the_gaps() {
        assert_eq!(complement_ranges(&[1..3, 5..6], 8), vec![0..1, 3..5, 6..8]);
        assert!(complement_ranges(&[rng(0, 4)], 4).is_empty());
        assert_eq!(complement_ranges(&[], 3), vec![rng(0, 3)]);
    }

    #[rstest]
    #[case(DisplayMode::Draw)]
    #[case(DisplayMode::Hide)]
    #[case(DisplayMode::Keep)]
    fn channel_query_matches_honors_the_mode(#[case] mode: DisplayMode) {
        let track = TrackRef::new(FileIdx::new(0), TrackIdx::new(0));
        let per_track = HashMap::from([(track, (vec![rng(1, 3)], 5usize))]);
        let matches = channel_query_matches(mode, &per_track);
        match mode {
            // Draw halos the matched segments.
            DisplayMode::Draw => {
                assert_eq!(matches.draws[0].ranges_for(track), [rng(1, 3)]);
                assert!(matches.hidden.is_empty());
            }
            // Hide breaks the polyline at the matched segments.
            DisplayMode::Hide => {
                assert_eq!(matches.hidden_ranges(track), [rng(1, 3)]);
                assert!(matches.draws.is_empty());
            }
            // Keep breaks the polyline everywhere else (the complement).
            DisplayMode::Keep => {
                assert_eq!(matches.hidden_ranges(track), &[0..1, 3..5]);
                assert!(matches.draws.is_empty());
            }
        }
    }

    #[test]
    fn a_loaded_channel_checks_and_runs_end_to_end() {
        // The whole app path: build the editor schema from the file, check a
        // channel query against it, then run it over a provider carrying the
        // same channel. The peak sample (1.5 g) clears the 1 g threshold.
        let channel = scalar_channel("accel", Some("g"), &[(0, 0.9), (1, 1.5)]);
        let files = [file_with_channels(vec![channel.clone()])];
        let schema = schema_from_files(&files);
        let query = check_text("points | window 2 | where max(@accel) > 1 g", &schema)
            .expect("checks against the loaded schema");

        let points = test_points();
        let channels = [channel];
        let provider = provider_for(&points, &channels, None);
        let output = gt_query::run(
            &query,
            &[TrackInput {
                track: TrackRef::new(FileIdx::new(0), TrackIdx::new(0)),
                provider: &provider,
            }],
        );
        assert_eq!(output.matches.len(), 1, "the window matches");
        assert_eq!(output.summary.match_count, 1);
    }

    #[test]
    fn a_vector_component_checks_and_runs_end_to_end() {
        // The whole app path for a vector component: build the schema, check
        // @accel.y, then run over a provider carrying the vector. Only the y
        // column (peak 1.5 g) clears the threshold; x (0.9) would not.
        let channel = vector_channel(
            "accel",
            Some("g"),
            &["x", "y", "z"],
            &[(0, [0.9, 0.9, 0.9]), (1, [0.9, 1.5, 0.9])],
        );
        let files = [file_with_channels(vec![channel.clone()])];
        let schema = schema_from_files(&files);
        let query = check_text("points | window 2 | where max(@accel.y) > 1 g", &schema)
            .expect("a component checks against the loaded schema");

        let points = test_points();
        let channels = [channel];
        let provider = provider_for(&points, &channels, None);
        let output = gt_query::run(
            &query,
            &[TrackInput {
                track: TrackRef::new(FileIdx::new(0), TrackIdx::new(0)),
                provider: &provider,
            }],
        );
        assert_eq!(output.matches.len(), 1, "the y column clears the threshold");
    }

    #[test]
    fn an_si_prefixed_channel_unit_checks_and_runs_end_to_end() {
        // The whole app path for SI prefixes on both sides: a channel spec'd
        // in mg (the usual IMU datasheet unit) against an mg literal. Sample
        // 1 (80 mg) clears the 50 mg threshold; sample 0 (20 mg) does not,
        // pinning that the channel values scale by the prefixed label too.
        let channel = vector_channel(
            "accel",
            Some("mg"),
            &["x", "y", "z"],
            &[(0, [20.0, 0.0, 0.0]), (1, [80.0, 0.0, 0.0])],
        );
        let files = [file_with_channels(vec![channel.clone()])];
        let schema = schema_from_files(&files);
        let query = check_text("points | window 2 | where max(@accel.x) > 50 mg", &schema)
            .expect("an mg channel compares to an mg literal");
        // The same channel against a g literal: the units share the quantity.
        check_text("points | window 2 | where max(@accel.x) > 0.05 g", &schema)
            .expect("an mg channel compares to a g literal");

        let points = test_points();
        let channels = [channel];
        let provider = provider_for(&points, &channels, None);
        let output = gt_query::run(
            &query,
            &[TrackInput {
                track: TrackRef::new(FileIdx::new(0), TrackIdx::new(0)),
                provider: &provider,
            }],
        );
        assert_eq!(output.summary.match_count, 1, "only the 80 mg sample");
    }

    #[test]
    fn norm_of_a_loaded_vector_checks_and_runs_end_to_end() {
        // norm(@accel) over a loaded vector: row 0 is (3,4,0) -> 5 m/s2, well
        // over 0.1 g (0.981 m/s2), so the window matches.
        let channel = vector_channel(
            "accel",
            Some("g"),
            &["x", "y", "z"],
            &[(0, [3.0, 4.0, 0.0]), (1, [0.1, 0.0, 0.0])],
        );
        let files = [file_with_channels(vec![channel.clone()])];
        let schema = schema_from_files(&files);
        let query = check_text(
            "points | window 2 | where max(norm(@accel)) > 0.1 g",
            &schema,
        )
        .expect("norm checks against the loaded schema");

        let points = test_points();
        let channels = [channel];
        let provider = provider_for(&points, &channels, None);
        let output = gt_query::run(
            &query,
            &[TrackInput {
                track: TrackRef::new(FileIdx::new(0), TrackIdx::new(0)),
                provider: &provider,
            }],
        );
        assert_eq!(
            output.matches.len(),
            1,
            "the magnitude clears the threshold"
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
        // A stage keyword gets a trailing space so the next token can follow;
        // a metric does not (an operator, not a space, comes next); a function
        // brings its parentheses with the caret stepping inside them; a unit
        // pads itself off a glued digit.
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

    #[test]
    fn analysis_context_ties_the_next_line_to_its_chunk() {
        // Caret inside a chunk: the chunk itself.
        assert_eq!(analysis_context("points | draw", 6), 0..13);
        // Caret on the line directly after a chunk (Enter pressed to continue
        // it): the chunk extends to the caret, so `| where` is analyzed in
        // context rather than as a fresh query.
        let text = "points\n";
        assert_eq!(analysis_context(text, text.len()), 0..text.len());
        // A blank line in between is the query separator: fresh context.
        let separated = "points\n\n";
        assert_eq!(
            analysis_context(separated, separated.len()),
            separated.len()..separated.len()
        );
        // On the (would-be separator) line right after a chunk, typing still
        // continues that chunk - a character typed there joins the two lines
        // into one query, so the analysis matches what an edit would produce.
        let two = "points | draw\n\npoints | hide";
        assert_eq!(analysis_context(two, 14), 0..14);
    }

    #[test]
    fn comment_only_chunks_are_skipped_not_checked() {
        // A standalone comment paragraph between queries is documentation,
        // not a query: it must not error (or block Run).
        let text = "# block comment\n\npoints | draw";
        let chunks = check_all(text, &ChannelSchema::new());
        assert_eq!(chunks.len(), 1, "only the real query is a chunk");
        assert!(chunks[0].result.is_ok(), "the real query checks");
        // A chunk with a lexer-rejected character is code, not comment - it
        // still surfaces its error.
        let bad = check_all("Points", &ChannelSchema::new());
        assert_eq!(bad.len(), 1);
        assert!(bad[0].result.is_err(), "the rejected character errors");
    }
}

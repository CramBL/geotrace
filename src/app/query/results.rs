//! The results tab: what each query of the run counted, the run's matches as a
//! table to pick from, and the rows of the picked match under their columns.

use std::collections::BTreeSet;
use std::fmt::Write as _;

use egui::{
    Align, Button, CursorIcon, Label, Layout, RichText, ScrollArea, Sense, TextStyle, TextWrapMode,
    Window,
};
use egui_extras::{Column, TableBuilder, TableRow};
use egui_phosphor::regular::ARROW_SQUARE_IN as ICON_ARROW_SQUARE_IN;
use egui_phosphor::regular::ARROW_SQUARE_OUT as ICON_ARROW_SQUARE_OUT;
use egui_phosphor::regular::CARET_DOWN as ICON_CARET_DOWN;
use egui_phosphor::regular::COPY as ICON_COPY;
use egui_phosphor::regular::CROSSHAIR as ICON_CROSSHAIR;
use egui_phosphor::regular::INFO as ICON_INFO;
use gt_query::{ChannelTimeline, Construct, MetricProvider as _, QueryMetric};
use gt_query_run::{
    ChannelResults, ChannelTrackResult, PointsResults, QuerySummary, RunResults, SliceProvider,
    TrackProvider, TrackQueryData,
};
use gt_side_panel::widgets::{PointClickRequests, apply_point_click};
use gt_types::{DataCategory, LoadedFile, NavPoint, PointIdx, TrackRef};
use gt_ui_theme::buttons::SortHeaderButton;
use gt_ui_theme::labels::{CountLine, LabelWithHover};
use gt_ui_types::{
    DataPointRef, HighlightScope, MapHighlight, MapScope, MatchHighlight, MatchRevealTarget,
    StaleRunNote,
};
use strum::IntoEnumIterator as _;

use super::column_format::{self, ColumnFormat};
use super::match_row::{MatchColumn, MatchKey, MatchRow, MatchRows, MatchSort, RowNoun};
use super::results_split::{MIN_SPLIT_ROWS, ResultsSplit, SplitGeometry};
use super::value_bar::{ColumnValueRange, RunColumnRanges, ValueBar};

/// Vertical padding a row adds around its text.
pub(super) const ROW_PADDING: f32 = 2.0;

/// Matches listed before the matches table scrolls, so the points table below
/// it keeps most of the window. The splitter under the table moves the
/// boundary from there.
const VISIBLE_MATCH_ROWS: usize = 5;

/// Height of the splitter band between the two tables: thin, and still wide
/// enough to grab.
const SPLITTER_HEIGHT: f32 = 8.0;

/// The grip painted at the middle of that band, so it reads as draggable.
const SPLITTER_GRIP_WIDTH: f32 = 40.0;
const SPLITTER_GRIP_HEIGHT: f32 = 2.0;
const SPLITTER_GRIP_CORNER_RADIUS: f32 = 1.0;

/// What a screen reader announces the splitter as.
pub(crate) const SPLITTER_LABEL: &str = "Resize the matches list";

/// The window the matches list moves into when it is popped out of the results
/// tab.
pub(crate) const MATCH_LIST_WINDOW_TITLE: &str = "Query matches";

/// Size that window opens at. Like the query window it grows only when the
/// user drags it: the table inside scrolls.
pub(crate) const MATCH_LIST_WINDOW_WIDTH: f32 = 460.0;
pub(crate) const MATCH_LIST_WINDOW_HEIGHT: f32 = 320.0;

/// Width the track column never falls below, however narrow the window is.
const MIN_TRACK_COLUMN_WIDTH: f32 = 30.0;

/// How far the painted swatch sits inside the square allocated for it, and how
/// round its corners are.
const SWATCH_INSET_PX: f32 = 1.0;
const SWATCH_CORNER_RADIUS_PX: f32 = 2.0;

/// The second line of a sample row's hover, under its index.
const SAMPLE_WITHOUT_POSITION: &str = "Samples have no position: nothing to pin";

/// What the results tab writes back to the app: the cross-highlight its rows
/// drive, the point a click pins, and what to frame the map on.
pub(super) struct ResultsOutputs<'a, 'b> {
    pub(super) highlight: &'a mut MapHighlight,
    pub(super) requests: &'a mut PointClickRequests<'b>,
    pub(super) reveal: &'a mut Option<MatchRevealTarget>,
}

/// What the results tab keeps between frames: the order of the matches table,
/// the match the points table follows, the ranges its bars scale to, and the
/// share of the tab the matches table takes.
#[derive(Debug, Default)]
pub(super) struct ResultsState {
    sort: MatchSort,
    /// The picked match, kept across reruns. `None` until one is picked, which
    /// lists the first match.
    selected: Option<MatchKey>,
    column_ranges: RunColumnRanges,
    split: ResultsSplit,
}

/// One query of the run: what it counted, the colour it draws in, and the
/// columns its matches' rows read under.
struct QuerySection<'a> {
    summary: &'a QuerySummary,
    color: Option<egui::Color32>,
    columns: Vec<TableColumn<'a>>,
}

/// One column of the points table: what its header names, the documentation
/// that header explains on hover, how its cells print, and where they read
/// their value.
struct TableColumn<'a> {
    name: String,
    doc: Option<&'static Construct>,
    format: ColumnFormat<'a>,
    source: ColumnSource,
}

/// Where one points-table column reads its value.
#[derive(Debug, Clone, Copy)]
#[expect(
    variant_size_differences,
    reason = "one value per column of one table, never one per row"
)]
enum ColumnSource {
    Metric(QueryMetric),
    /// When a channel sample was recorded.
    SampleTime,
    /// One component of a channel, by its index into the sample's values.
    ChannelComponent(usize),
}

/// One track's values, as the run computed them. A row reads its value without
/// rebuilding a provider: this is built once per track the run matched.
struct TrackValues<'a> {
    points: &'a [NavPoint],
    provider: TrackProvider<'a>,
    /// The evaluator's view of the track, for the metrics derived across
    /// neighbouring points.
    slice: SliceProvider<'a>,
    slice_start: usize,
}

impl TrackValues<'_> {
    /// The value a column shows for one point, derived through the same slice
    /// the run evaluated so a filtered first point shows what the predicate
    /// used.
    fn value(&self, metric: QueryMetric, point_index: usize) -> Option<f64> {
        if metric == QueryMetric::Accel {
            return point_index
                .checked_sub(self.slice_start)
                .and_then(|relative| gt_query::derived_accel(&self.slice, relative));
        }
        self.provider.value(metric, point_index)
    }

    fn lat_lon(&self, point_index: usize) -> Option<(f64, f64)> {
        let (latitude, longitude) = self.points.get(point_index)?.resolved_position();
        Some((latitude.as_degrees(), longitude.as_degrees()))
    }
}

/// Where the rows of one run come from: the nav points a points query matched,
/// or the samples a channel-source query matched on the channel's own timeline.
enum RowSource<'a> {
    NavPoints {
        /// One entry per track the run matched, in track order.
        track_values: Vec<(TrackRef, TrackValues<'a>)>,
    },
    ChannelSamples {
        tracks: &'a [ChannelTrackResult],
    },
}

impl RowSource<'_> {
    /// The value under `column` of the row `source_index` addresses on `track`.
    fn value(&self, track: TrackRef, source_index: usize, column: ColumnSource) -> Option<f64> {
        match (self, column) {
            (Self::NavPoints { track_values }, ColumnSource::Metric(metric)) => track_values
                .iter()
                .find(|(candidate, _)| *candidate == track)
                .and_then(|(_, values)| values.value(metric, source_index)),
            (Self::ChannelSamples { tracks }, ColumnSource::SampleTime) => {
                timeline_of(tracks, track)?.times.get(source_index).copied()
            }
            (Self::ChannelSamples { tracks }, ColumnSource::ChannelComponent(component)) => {
                timeline_of(tracks, track)?.value(source_index, component)
            }
            (Self::NavPoints { .. } | Self::ChannelSamples { .. }, _) => None,
        }
    }

    /// The point on the map a row addresses, absent for a channel sample: a
    /// sample has no position of its own.
    fn map_point(&self, track: TrackRef, source_index: usize) -> Option<DataPointRef> {
        match self {
            Self::NavPoints { .. } => Some(DataPointRef {
                track,
                category: DataCategory::Tpv,
                point_index: PointIdx::new(source_index),
            }),
            Self::ChannelSamples { .. } => None,
        }
    }

    fn lat_lon(&self, track: TrackRef, source_index: usize) -> Option<(f64, f64)> {
        match self {
            Self::NavPoints { track_values } => track_values
                .iter()
                .find(|(candidate, _)| *candidate == track)
                .and_then(|(_, values)| values.lat_lon(source_index)),
            Self::ChannelSamples { .. } => None,
        }
    }

    /// What one row's hover states: which row of its source it holds, and where
    /// that row was recorded.
    fn row_hover_text(&self, track: TrackRef, source_index: usize) -> String {
        match self {
            Self::NavPoints { .. } => {
                point_hover_text(source_index, self.lat_lon(track, source_index))
            }
            Self::ChannelSamples { .. } => {
                format!("#{source_index}\n{SAMPLE_WITHOUT_POSITION}")
            }
        }
    }
}

fn timeline_of(tracks: &[ChannelTrackResult], track: TrackRef) -> Option<&ChannelTimeline> {
    tracks
        .iter()
        .find(|result| result.track == track)
        .map(|result| &result.timeline)
}

/// What a click in the points table requests from the app, applied once the
/// table is laid out and the enclosing panel's `Ui` is available again.
struct PointClick {
    point: DataPointRef,
    lat_lon: (f64, f64),
    response: egui::Response,
}

/// What a click in the matches table selects.
enum MatchAction {
    Select(MatchKey),
    FrameOnMap(MatchRevealTarget),
}

/// Where the matches list is drawn, which is what gives its table a height.
#[derive(Clone, Copy)]
enum MatchListPlacement {
    /// In the results tab, over the splitter that divides the tab between the
    /// list and the picked match's rows below it.
    ResultsTab,
    /// In a window of its own, which it fills.
    OwnWindow,
}

/// The heights the matches table lays its header and rows out at.
#[derive(Clone, Copy)]
struct MatchTableHeights {
    row: f32,
    /// Rows are separated by the item spacing, so a row on display takes that
    /// much more than its own height.
    stride: f32,
    /// The header, which stays put while the rows scroll.
    header: f32,
    /// The header and the gap under it: what the table takes before its first
    /// row.
    above_rows: f32,
}

impl MatchTableHeights {
    fn of(ui: &egui::Ui) -> Self {
        let row = ui.text_style_height(&TextStyle::Body) + 2.0 * ROW_PADDING;
        let spacing = ui.spacing().item_spacing.y;
        Self {
            row,
            stride: row + spacing,
            header: row,
            above_rows: row + spacing,
        }
    }

    /// What the table takes to list `rows`.
    fn listing(self, rows: usize) -> f32 {
        self.above_rows + self.stride * rows as f32
    }

    /// The height the rows scroll within when the whole table is `total` tall.
    fn body(self, total: f32) -> f32 {
        (total - self.above_rows).max(0.0)
    }
}

/// One run's results as the two tables of the results tab: every match of the
/// run to pick from, and the rows of the picked match under the columns its
/// query tables.
pub(super) struct ResultsTables<'a> {
    files: &'a [LoadedFile],
    /// The run these tables list, as the session numbered it. The value ranges
    /// the bars scale to are computed once per number.
    run: u64,
    /// One per query of the run, in editor order.
    queries: Vec<QuerySection<'a>>,
    matches: MatchRows,
    source: RowSource<'a>,
    row_noun: RowNoun,
    /// Whether the run drew any halo on the map, which is what the run-wide
    /// button frames.
    draws_halos: bool,
    /// Whether the data changed after the run that produced these rows.
    stale: bool,
}

impl<'a> ResultsTables<'a> {
    pub(super) fn of_run(files: &'a [LoadedFile], results: &'a RunResults) -> Self {
        match results {
            RunResults::Points(points) => Self::of_points(files, points),
            RunResults::Channel(channel) => Self::of_channel_samples(files, channel),
        }
    }

    /// The tables of a points run: every query's matches, listing nav points
    /// under the metric columns that query tables.
    fn of_points(files: &'a [LoadedFile], results: &'a PointsResults) -> Self {
        let stale = results.matches.stale;
        let matches = MatchRows::of_points(files, results);
        let tracks: BTreeSet<TrackRef> = matches.rows().iter().map(|row| row.track).collect();
        let queries = results
            .queries
            .iter()
            .map(|query| QuerySection {
                summary: &query.summary,
                color: query
                    .color
                    .map(|color| gt_ui_theme::query_halo_color(color, stale)),
                columns: query
                    .columns
                    .iter()
                    .map(|metric| TableColumn {
                        name: metric.to_string(),
                        doc: gt_query::metric_documentation(*metric),
                        format: ColumnFormat::of_metric(*metric),
                        source: ColumnSource::Metric(*metric),
                    })
                    .collect(),
            })
            .collect();
        Self {
            files,
            run: results.matches.run,
            queries,
            matches,
            source: RowSource::NavPoints {
                track_values: tracks
                    .into_iter()
                    .filter_map(|track| Some((track, track_values(files, results, track)?)))
                    .collect(),
            },
            row_noun: RowNoun::Point,
            draws_halos: results.matches.has_halos(),
            stale,
        }
    }

    /// The tables of a channel-source run: every track's matched stretch of
    /// samples, listing them under the sample time and one column per channel
    /// component.
    ///
    /// The whole table reads in the unit the first track declared: the
    /// evaluator timed and valued every track's samples in base units, and a
    /// run whose tracks declare incompatible units is rejected by the check
    /// before it gets here.
    fn of_channel_samples(files: &'a [LoadedFile], channel: &'a ChannelResults) -> Self {
        let stale = channel.matches.stale;
        // Samples are timed finer than points, so their times keep the
        // milliseconds the point tables leave out.
        let mut columns = vec![TableColumn {
            name: "time".to_owned(),
            doc: None,
            format: ColumnFormat::time_of_day_with_millis(),
            source: ColumnSource::SampleTime,
        }];
        let unit = channel
            .tracks
            .iter()
            .find_map(|result| result.unit.as_ref());
        let component_names: &[String] = if channel.components.is_empty() {
            std::slice::from_ref(&channel.channel)
        } else {
            &channel.components
        };
        columns.extend(
            component_names
                .iter()
                .enumerate()
                .map(|(component, name)| TableColumn {
                    name: name.clone(),
                    doc: None,
                    format: ColumnFormat::of_channel_unit(unit),
                    source: ColumnSource::ChannelComponent(component),
                }),
        );
        Self {
            files,
            run: channel.matches.run,
            queries: vec![QuerySection {
                summary: &channel.summary,
                color: channel
                    .matches
                    .draws
                    .first()
                    .map(|layer| gt_ui_theme::query_halo_color(layer.color, stale)),
                columns,
            }],
            matches: MatchRows::of_channel_samples(channel),
            source: RowSource::ChannelSamples {
                tracks: &channel.tracks,
            },
            row_noun: RowNoun::Sample,
            draws_halos: channel.matches.has_halos(),
            stale,
        }
    }

    /// The matches list, the caption naming the picked match, and that match's
    /// rows. While the list is popped out the tab holds only the last two, and
    /// the list fills a window of its own.
    pub(super) fn ui(
        &mut self,
        ui: &mut egui::Ui,
        state: &mut ResultsState,
        popped_out: &mut bool,
        scope: MapScope<'_>,
        out: &mut ResultsOutputs<'_, '_>,
    ) {
        self.matches.sort(state.sort);
        state
            .column_ranges
            .refresh(self.run, || self.column_ranges());
        let selected = self.matches.selected(state.selected).cloned();
        if let Some(selected) = &selected {
            state.selected = Some(selected.key());
        }
        ui.scope(|ui| {
            // A stale run's rows reference indices that may no longer address
            // the same data: they are shown, but answer nothing.
            if self.stale {
                ui.disable();
            }
            if !*popped_out {
                self.match_list_ui(
                    ui,
                    state,
                    selected.as_ref(),
                    popped_out,
                    out,
                    MatchListPlacement::ResultsTab,
                );
            }
            let Some(selected) = &selected else {
                return;
            };
            self.caption_ui(ui, selected);
            self.points_table_ui(ui, selected, &state.column_ranges, scope, out);
        });
        if *popped_out {
            self.match_list_window_ui(ui.ctx(), state, selected.as_ref(), popped_out, out);
        }
    }

    /// The matches list wherever it is drawn: the summary strip over every
    /// match of the run, and in the results tab the splitter that divides the
    /// tab between that table and the rows below it.
    fn match_list_ui(
        &self,
        ui: &mut egui::Ui,
        state: &mut ResultsState,
        selected: Option<&MatchRow>,
        popped_out: &mut bool,
        out: &mut ResultsOutputs<'_, '_>,
        placement: MatchListPlacement,
    ) {
        self.summary_strip_ui(ui, out.reveal, popped_out);
        let Some(selected) = selected else {
            ui.label(RichText::new("No matches").weak());
            return;
        };
        ui.add_space(ui.spacing().item_spacing.y);
        let heights = MatchTableHeights::of(ui);
        match placement {
            MatchListPlacement::OwnWindow => {
                // The table takes what is left of the window under the summary
                // strip and stops one item spacing short of its bottom edge.
                // The window keeps that height whether or not the run fills
                // it, so it does not resize itself around every run.
                let available = ui.available_height();
                ui.set_min_height(available);
                let body_height = heights.body(available - ui.spacing().item_spacing.y);
                self.matches_table_ui(ui, state, selected, out, body_height);
            }
            MatchListPlacement::ResultsTab => {
                let geometry = self.split_geometry(ui);
                let listed = state.split.matches_height(geometry);
                let table = self.matches_table_ui(ui, state, selected, out, heights.body(listed));
                let splitter = splitter_ui(ui);
                if splitter.double_clicked() {
                    state.split.reset();
                } else if splitter.dragged() {
                    // The table is as tall as it was laid out, which is where
                    // the splitter sits: the drag moves on from there.
                    state.split.set_matches_height(
                        geometry,
                        table.rect.height() + splitter.drag_delta().y,
                    );
                }
            }
        }
    }

    /// The matches list in a window of its own, leaving the results tab to the
    /// picked match's rows. Closing that window puts the list back in the tab.
    fn match_list_window_ui(
        &self,
        ctx: &egui::Context,
        state: &mut ResultsState,
        selected: Option<&MatchRow>,
        popped_out: &mut bool,
        out: &mut ResultsOutputs<'_, '_>,
    ) {
        let mut open = true;
        Window::new(MATCH_LIST_WINDOW_TITLE)
            .open(&mut open)
            .default_width(MATCH_LIST_WINDOW_WIDTH)
            .default_height(MATCH_LIST_WINDOW_HEIGHT)
            .resizable(true)
            .show(ctx, |ui| {
                // The summary strip and the table's columns have a width they
                // cannot go below.
                ScrollArea::horizontal().show(ui, |ui| {
                    ui.scope(|ui| {
                        if self.stale {
                            ui.disable();
                        }
                        self.match_list_ui(
                            ui,
                            state,
                            selected,
                            popped_out,
                            out,
                            MatchListPlacement::OwnWindow,
                        );
                    });
                });
            });
        if !open {
            *popped_out = false;
        }
    }

    /// What the results tab has to divide between its two tables, measured
    /// before either one is laid out. Each line of text counts as the whole
    /// pixels egui lays it out in, so a minimum really holds the rows it
    /// counts.
    fn split_geometry(&self, ui: &egui::Ui) -> SplitGeometry {
        let heights = MatchTableHeights::of(ui);
        let spacing = ui.spacing().item_spacing.y;
        let caption = ui.text_style_height(&TextStyle::Body).ceil() + spacing;
        let point_row = ui.text_style_height(&TextStyle::Monospace).ceil() + ROW_PADDING + spacing;
        SplitGeometry {
            available: ui.available_height(),
            matches_minimum: heights.listing(MIN_SPLIT_ROWS),
            matches_content: heights.listing(self.matches.rows().len()),
            matches_default: heights.listing(VISIBLE_MATCH_ROWS),
            points_minimum: caption
                + column_format::header_height(ui)
                + spacing
                + point_row * MIN_SPLIT_ROWS as f32,
            splitter: SPLITTER_HEIGHT + 2.0 * spacing,
        }
    }

    /// One line per query: what it matched, what it hides, and what it skipped,
    /// with the run-wide buttons on the first line.
    fn summary_strip_ui(
        &self,
        ui: &mut egui::Ui,
        reveal: &mut Option<MatchRevealTarget>,
        popped_out: &mut bool,
    ) {
        for (index, query) in self.queries.iter().enumerate() {
            ui.horizontal(|ui| {
                if index == 0 {
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        self.pop_out_button_ui(ui, popped_out);
                        self.copy_tsv_button_ui(ui);
                        self.frame_run_on_map_button_ui(ui, reveal);
                        ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                            self.summary_counts_ui(ui, query);
                        });
                    });
                } else {
                    self.summary_counts_ui(ui, query);
                }
            });
        }
    }

    /// The button moving the matches list between the results tab and a window
    /// of its own. A run that matched nothing has no list to move out, but one
    /// already moved out always comes back.
    fn pop_out_button_ui(&self, ui: &mut egui::Ui, popped_out: &mut bool) {
        if *popped_out {
            if ui
                .add(Button::new(ICON_ARROW_SQUARE_IN).small())
                .on_hover_text("Put the matches back in the query window")
                .clicked()
            {
                *popped_out = false;
            }
            return;
        }
        let has_matches = !self.matches.is_empty();
        let response = ui.add_enabled(has_matches, Button::new(ICON_ARROW_SQUARE_OUT).small());
        if !has_matches {
            response.on_disabled_hover_text("This run matched nothing");
        } else if response
            .on_hover_text("Show the matches in a window of their own")
            .clicked()
        {
            *popped_out = true;
        }
    }

    /// One query's counts as one line: the numbers in the text colour, what
    /// they count dimmed, with everything the run left out on hover.
    fn summary_counts_ui(&self, ui: &mut egui::Ui, query: &QuerySection<'_>) {
        let summary = query.summary;
        let summary_with_notes = summary.to_string();
        query_swatch_ui(ui, query.color, &summary_with_notes);
        let mut line = CountLine::new(ui)
            .count(
                summary.match_count,
                gt_fmt::pluralize(summary.match_count, "match", "matches"),
            )
            .dot()
            .count(
                summary.tracks_with_matches,
                gt_fmt::pluralize(summary.tracks_with_matches, "track", "tracks"),
            );
        if let Some(hidden) = summary.hidden_points {
            line = line
                .dot()
                .number(hidden.hidden)
                .words(" of ")
                .count(hidden.total, "points hidden");
        }
        if summary.skipped > 0 {
            line = line.dot().count(summary.skipped, "skipped");
        }
        if !summary.notes.is_empty() {
            line = line.words(&format!(" {ICON_INFO}"));
        }
        // Truncated to the width that is left: a line that extends would push
        // the window over the map beside it. The hover explains the line, so
        // it takes the Help cursor: it spells out the notes the line reduces
        // to an icon.
        LabelWithHover::plain(line.into_job())
            .truncate()
            .explanation_ui(ui, &summary_with_notes);
    }

    /// The button framing the map on every match of this run and playing their
    /// reveal animation again. Disabled with the reason in its hover text when
    /// the run drew no halos, or when the data changed after it.
    fn frame_run_on_map_button_ui(
        &self,
        ui: &mut egui::Ui,
        reveal: &mut Option<MatchRevealTarget>,
    ) {
        let disabled_reason = if self.stale {
            Some(StaleRunNote::RunAgainToFrameItsMatches.to_string())
        } else if !self.draws_halos {
            Some("This run drew no matches on the map".to_owned())
        } else {
            None
        };
        let button = Button::new(ICON_CROSSHAIR).small();
        let response = ui.add_enabled(disabled_reason.is_none(), button);
        if let Some(reason) = disabled_reason {
            response.on_disabled_hover_text(reason);
        } else if response
            .on_hover_text("Frame the map on every match of this run and highlight them")
            .clicked()
        {
            *reveal = Some(MatchRevealTarget::WholeRun);
        }
    }

    /// Copies every row of every match to the clipboard as tab-separated
    /// values, for a spreadsheet.
    fn copy_tsv_button_ui(&self, ui: &mut egui::Ui) {
        let has_matches = !self.matches.is_empty();
        let response = ui.add_enabled(has_matches, Button::new(ICON_COPY).small());
        if !has_matches {
            response.on_disabled_hover_text("This run matched nothing");
        } else if response.on_hover_text(self.copy_tooltip()).clicked() {
            ui.ctx().copy_text(self.as_tsv());
        }
    }

    fn copy_tooltip(&self) -> String {
        format!(
            "Copy as tab-separated values: one line per matched {}, starting \
             with the number of the match it belongs to and its index in the {}",
            self.row_noun.singular(),
            self.row_noun.index_source()
        )
    }

    /// The line naming the match the points table lists below it.
    fn caption_ui(&self, ui: &mut egui::Ui, selected: &MatchRow) {
        let rows = selected.rows.len();
        ui.label(
            CountLine::new(ui)
                .words("Match ")
                .number(selected.number)
                .dot()
                .count(
                    rows,
                    gt_fmt::pluralize(rows, self.row_noun.singular(), self.row_noun.plural()),
                )
                .into_job(),
        );
    }

    /// Every match of the run, one row each, ordered by the column the user
    /// clicked. Clicking a row picks the match the points table follows. The
    /// rows scroll within `body_height`, under a header that stays put.
    fn matches_table_ui(
        &self,
        ui: &mut egui::Ui,
        state: &mut ResultsState,
        selected: &MatchRow,
        out: &mut ResultsOutputs<'_, '_>,
        body_height: f32,
    ) -> egui::Response {
        let heights = MatchTableHeights::of(ui);
        let widths = self.match_column_widths(ui);
        let swatch_width = swatch_side(ui);
        let mut action: Option<MatchAction> = None;

        let table = ui.scope(|ui| {
            let mut table = TableBuilder::new(ui)
                .id_salt("query_match_list")
                .striped(true)
                .sense(Sense::click())
                .auto_shrink([false, true])
                .min_scrolled_height(0.0)
                .max_scroll_height(body_height)
                .cell_layout(Layout::left_to_right(Align::Center))
                .column(Column::exact(swatch_width));
            for width in &widths {
                table = table.column(Column::exact(*width).clip(true));
            }
            // The trailing column holds the button framing the map on one
            // match, and stretches the striping across the whole table.
            table = table.column(Column::remainder());

            table
                .header(heights.header, |mut header| {
                    header.col(|_| {});
                    for column in MatchColumn::iter() {
                        header.col(|ui| sort_header_ui(ui, column, state, self.row_noun));
                    }
                    header.col(|_| {});
                })
                .body(|body| {
                    body.rows(heights.row, self.matches.rows().len(), |mut row| {
                        let Some(match_row) = self.matches.rows().get(row.index()) else {
                            return;
                        };
                        let picked = match_row.key() == selected.key();
                        if let Some(taken) =
                            self.match_row_ui(&mut row, match_row, picked, out.highlight)
                        {
                            action = Some(taken);
                        }
                    });
                });
        });

        match action {
            Some(MatchAction::Select(key)) => state.selected = Some(key),
            Some(MatchAction::FrameOnMap(target)) => *out.reveal = Some(target),
            None => {}
        }
        table.response
    }

    /// One match's row: its query's colour, what it covers, and the button
    /// framing the map on it.
    fn match_row_ui(
        &self,
        row: &mut TableRow<'_, '_>,
        match_row: &MatchRow,
        selected: bool,
        highlight: &mut MapHighlight,
    ) -> Option<MatchAction> {
        row.set_selected(selected);
        let query = self.queries.get(match_row.query_index);
        row.col(|ui| {
            let Some(query) = query else {
                return;
            };
            query_swatch_ui(ui, query.color, &query.summary.to_string());
        });
        for column in MatchColumn::iter() {
            let text =
                match_row.cell_text(column, self.files, self.matches.duration_clock_format());
            row.col(|ui| {
                ui.with_layout(column.cell_layout(), |ui| {
                    ui.add(
                        Label::new(RichText::new(&text).monospace())
                            .wrap_mode(TextWrapMode::Extend),
                    );
                });
            });
        }
        let target = self.reveal_target(match_row);
        let mut reveal = None;
        row.col(|ui| {
            if frame_match_on_map_button_ui(ui, target.as_ref().err().map(String::as_str)).clicked()
            {
                reveal = target.as_ref().ok().cloned();
            }
        });

        let response = row.response();
        if response.hovered() {
            self.hover_match(match_row, highlight);
        }
        if let Some(target) = reveal {
            return Some(MatchAction::FrameOnMap(target));
        }
        response
            .clicked()
            .then(|| MatchAction::Select(match_row.key()))
    }

    /// What the button at the end of a match's row frames the map on, or the
    /// reason it cannot.
    fn reveal_target(&self, match_row: &MatchRow) -> Result<MatchRevealTarget, String> {
        if self.stale {
            return Err(StaleRunNote::RunAgainToFrameThisMatch.to_string());
        }
        match self.source {
            RowSource::NavPoints { .. } => Ok(MatchRevealTarget::OneMatch {
                track: match_row.track,
                points: match_row.rows.clone(),
            }),
            RowSource::ChannelSamples { .. } => Err("Channel samples have no position of their \
                 own: the button above frames the whole run"
                .to_owned()),
        }
    }

    /// What hovering a match's row echoes elsewhere. A channel sample range
    /// indexes the channel's own timeline, so it bands nothing on the map: its
    /// track still takes focus.
    fn hover_match(&self, match_row: &MatchRow, highlight: &mut MapHighlight) {
        if let RowSource::NavPoints { .. } = self.source {
            highlight.hover_match = Some(MatchHighlight::new(match_row.track, &match_row.rows));
        }
        // Track focus alongside the band: the map fades the other tracks and
        // the plot dims their series, like hovering the track in the side
        // panel.
        highlight.hover = Some(HighlightScope::Track(match_row.track));
    }

    /// Every column of the matches table between its swatch and its map button,
    /// sized once for its header and the widest value it prints. The track
    /// column takes what the others leave, so the table is exactly as wide as
    /// the window.
    fn match_column_widths(&self, ui: &egui::Ui) -> Vec<f32> {
        let caret = column_format::text_width(ui, ICON_CARET_DOWN, &TextStyle::Small)
            + ui.spacing().item_spacing.x;
        let natural = |column: MatchColumn| {
            let header =
                column_format::text_width(ui, column.title(self.row_noun), &TextStyle::Body)
                    + caret;
            let cells = column_format::text_width(
                ui,
                self.matches.widest_cell_text(column),
                &TextStyle::Monospace,
            );
            header.max(cells)
        };
        // What the track column has to share the row with: every other column,
        // the swatch, the map button, the gap between each pair of columns, and
        // the scroll bar.
        let taken: f32 = MatchColumn::iter()
            .filter(|column| *column != MatchColumn::Track)
            .map(natural)
            .sum::<f32>()
            + swatch_side(ui)
            + column_format::text_width(ui, ICON_CROSSHAIR, &TextStyle::Body)
            + 2.0 * ui.spacing().button_padding.x
            + ui.spacing().item_spacing.x * (MatchColumn::iter().count() + 1) as f32
            + ui.spacing().scroll.allocated_width();
        let left = (ui.available_width() - taken).max(MIN_TRACK_COLUMN_WIDTH);
        MatchColumn::iter()
            .map(|column| match column {
                MatchColumn::Track => self.widest_track_label_width(ui).min(left),
                other => natural(other),
            })
            .collect()
    }

    /// How wide the track column has to be for the longest label it prints. A
    /// label is the track's number, led by its recording's filename only while
    /// several files are loaded.
    fn widest_track_label_width(&self, ui: &egui::Ui) -> f32 {
        let number = column_format::text_width(
            ui,
            self.matches.widest_cell_text(MatchColumn::Track),
            &TextStyle::Monospace,
        );
        if self.files.len() < 2 {
            return number;
        }
        self.files
            .iter()
            .map(|file| {
                column_format::text_width(ui, &file.metadata.filename, &TextStyle::Monospace)
                    + number
            })
            .fold(number, f32::max)
    }

    /// The picked match's rows, under the columns its query tables.
    fn points_table_ui(
        &self,
        ui: &mut egui::Ui,
        selected: &MatchRow,
        ranges: &RunColumnRanges,
        scope: MapScope<'_>,
        out: &mut ResultsOutputs<'_, '_>,
    ) {
        let Some(query) = self.queries.get(selected.query_index) else {
            return;
        };
        let columns = query.columns.as_slice();
        let cells = PointColumns {
            columns,
            // A stale run's rows read data that changed after it: their bars
            // come back with the next run.
            ranges: if self.stale {
                &[]
            } else {
                ranges.of_query(selected.query_index)
            },
            bar_color: gt_ui_theme::query_value_bar_color(
                query.color.unwrap_or_else(|| ui.visuals().text_color()),
            ),
        };
        let widths: Vec<f32> = columns
            .iter()
            .map(|column| column.format.column_width(ui, &column.name))
            .collect();
        let row_height = ui.text_style_height(&TextStyle::Monospace) + ROW_PADDING;
        let header_height = column_format::header_height(ui);
        let mut click: Option<PointClick> = None;

        ui.scope(|ui| {
            // What is left of the window under the sticky header, which is
            // drawn above the scrolling body. The item spacing comes off as
            // well: egui places a widget after that spacing, so a table sized
            // to the whole remaining height would end one spacing past the
            // window and grow it by that much every frame.
            let body_height =
                (ui.available_height() - header_height - ui.spacing().item_spacing.y).max(0.0);

            let mut table = TableBuilder::new(ui)
                .id_salt("query_point_table")
                .striped(true)
                .sense(Sense::click())
                .auto_shrink([false, false])
                .min_scrolled_height(body_height)
                .max_scroll_height(body_height)
                .cell_layout(Layout::left_to_right(Align::Center));
            for width in &widths {
                // Exact columns clip: a value wider than the width its column
                // was sized for is cut off there, and the table stays as wide as
                // the window it is in.
                table = table.column(Column::exact(*width));
            }
            // The trailing column holds no value: it stretches the striping and
            // the hover fill across the full width of the table.
            table = table.column(Column::remainder());

            table
                .header(header_height, |mut header| {
                    for column in columns {
                        header.col(|ui| {
                            column.format.header_ui(ui, &column.name, column.doc);
                        });
                    }
                    header.col(|_| {});
                })
                .body(|body| {
                    body.rows(row_height, selected.rows.len(), |mut row| {
                        let Some(source_index) = selected.rows.start.checked_add(row.index())
                        else {
                            return;
                        };
                        if let Some(taken) = self.point_row_ui(
                            &mut row,
                            &cells,
                            selected.track,
                            source_index,
                            out.highlight,
                        ) {
                            click = Some(taken);
                        }
                    });
                });
        });

        // Applied out here, where the panel's `Ui` places the pinned window
        // beside it.
        if let Some(click) = click {
            apply_point_click(
                ui,
                &click.response,
                click.point,
                click.lat_lon,
                scope,
                out.highlight,
                out.requests,
            );
        }
    }

    /// One matched row: its value under each column the table shows, the bar
    /// behind it, and the click it took.
    fn point_row_ui(
        &self,
        row: &mut TableRow<'_, '_>,
        cells: &PointColumns<'_>,
        track: TrackRef,
        source_index: usize,
        highlight: &mut MapHighlight,
    ) -> Option<PointClick> {
        let point = self.source.map_point(track, source_index);
        row.set_selected(
            point.is_some_and(|point| highlight.sticky.is_some_and(|sticky| sticky == point)),
        );
        for (index, column) in cells.columns.iter().enumerate() {
            let value = self.source.value(track, source_index, column.source);
            let bar = cells.bar(index, value);
            row.col(|ui| column.format.value_ui(ui, value, bar));
        }
        row.col(|_| {});

        let response = row
            .response()
            .on_hover_text(self.source.row_hover_text(track, source_index));
        let point = point?;
        if response.hovered() {
            // The ring the plot cursor draws: the row and the map then agree
            // on which point is meant.
            highlight.plot_hover_point = Some((track.fi, track.index, point.point_index));
        }
        if !response.clicked() && !response.double_clicked() {
            return None;
        }
        Some(PointClick {
            point,
            lat_lon: self.source.lat_lon(track, source_index)?,
            response,
        })
    }

    /// The value range every column of every query spans over all of the run's
    /// matched rows, in editor order. A column of times or blanks holds none:
    /// it states no magnitude to compare.
    fn column_ranges(&self) -> Vec<Vec<Option<ColumnValueRange>>> {
        self.queries
            .iter()
            .enumerate()
            .map(|(query_index, query)| {
                query
                    .columns
                    .iter()
                    .map(|column| {
                        if !column.format.holds_magnitudes() {
                            return None;
                        }
                        self.matches.column_range(query_index, |track, index| {
                            self.source.value(track, index, column.source)
                        })
                    })
                    .collect()
            })
            .collect()
    }

    /// Every match of the run as tab-separated values, in the order the table
    /// lists them: one block per query, each with a header line naming its
    /// columns in the unit their values are in.
    fn as_tsv(&self) -> String {
        let mut tsv = String::new();
        for (query_index, query) in self.queries.iter().enumerate() {
            if !tsv.is_empty() {
                tsv.push('\n');
            }
            // Writing to a String cannot fail.
            write!(tsv, "match\t{}", self.row_noun.singular()).ok();
            for column in &query.columns {
                tsv.push('\t');
                tsv.push_str(&column.format.header_with_unit(&column.name));
            }
            tsv.push('\n');

            let rows = self
                .matches
                .rows()
                .iter()
                .filter(|row| row.query_index == query_index);
            for match_row in rows {
                for source_index in match_row.rows.clone() {
                    // Writing to a String cannot fail.
                    write!(tsv, "{}\t{source_index}", match_row.number).ok();
                    for column in &query.columns {
                        tsv.push('\t');
                        let value = self
                            .source
                            .value(match_row.track, source_index, column.source);
                        if let Some(value) = value {
                            tsv.push_str(&column.format.cell_text(Some(value)));
                        }
                    }
                    tsv.push('\n');
                }
            }
        }
        tsv
    }
}

/// What the points table lists one row under: the columns of its query, the
/// range each of them spans over the run, and the colour of the bars behind
/// their cells.
struct PointColumns<'a> {
    columns: &'a [TableColumn<'a>],
    /// One entry per column of `columns`, in the same order. Empty where the
    /// table paints no bars.
    ranges: &'a [Option<ColumnValueRange>],
    bar_color: egui::Color32,
}

impl PointColumns<'_> {
    /// The bar behind the cell the column at `index` prints `value` in.
    fn bar(&self, index: usize, value: Option<f64>) -> Option<ValueBar> {
        let fraction = self
            .ranges
            .get(index)
            .copied()
            .flatten()?
            .bar_fraction(value)?;
        Some(ValueBar::new(fraction, self.bar_color))
    }
}

/// A clickable matches-table header that orders the list by `column`.
///
/// The active column shows a caret pointing the way its values run. Clicking it
/// reverses that, clicking any other column switches to it.
fn sort_header_ui(
    ui: &mut egui::Ui,
    column: MatchColumn,
    state: &mut ResultsState,
    row_noun: RowNoun,
) {
    let mut header = SortHeaderButton::new(column.title(row_noun)).wrap_mode(TextWrapMode::Extend);
    if state.sort.column == column {
        header = header.active_direction_caret(state.sort.direction.caret());
    }

    let clicked = header
        .show(
            ui,
            column.cell_layout(),
            column.order_hint(state.sort.next_direction(column)),
        )
        .clicked();

    if clicked {
        state.sort.clicked(column);
    }
}

/// The band between the matches table and the points table below it. Dragging
/// it gives one table the height of the other, double-clicking it puts the
/// boundary back where it opened.
fn splitter_ui(ui: &mut egui::Ui) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), SPLITTER_HEIGHT),
        Sense::click_and_drag(),
    );
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::ResizeHandle,
            ui.is_enabled(),
            SPLITTER_LABEL,
        )
    });
    ui.painter().hline(
        rect.x_range(),
        rect.center().y,
        ui.visuals().widgets.noninteractive.bg_stroke,
    );
    ui.painter().rect_filled(
        egui::Rect::from_center_size(
            rect.center(),
            egui::vec2(SPLITTER_GRIP_WIDTH, SPLITTER_GRIP_HEIGHT),
        ),
        SPLITTER_GRIP_CORNER_RADIUS,
        ui.style().interact(&response).fg_stroke.color,
    );
    response
        .on_hover_cursor(CursorIcon::ResizeVertical)
        .on_hover_text("Drag to resize the matches list, double-click to reset")
}

/// The button framing the map on one match, at the right end of its row.
/// Disabled with the reason on hover where it cannot answer.
fn frame_match_on_map_button_ui(
    ui: &mut egui::Ui,
    disabled_reason: Option<&str>,
) -> egui::Response {
    // A right-to-left scope: the button is drawn at its own size at the end of
    // the row.
    ui.scope_builder(
        egui::UiBuilder::new().layout(Layout::right_to_left(Align::Center)),
        |ui| {
            let enabled = disabled_reason.is_none();
            let response = ui.add_enabled(enabled, Button::new(ICON_CROSSHAIR).small());
            match disabled_reason {
                Some(reason) => response.on_disabled_hover_text(reason),
                None => response.on_hover_text("Frame the map on this match alone"),
            }
        },
    )
    .inner
}

/// The side of the square painted in a draw query's halo colour.
fn swatch_side(ui: &egui::Ui) -> f32 {
    TextStyle::Body.resolve(ui.style()).size
}

/// A small square in a draw query's halo `color`, tying a summary line and its
/// matches to the halos on the map. A query that draws no halos leaves the
/// space blank, so the lines and rows still line up. Painted rather than a text
/// glyph, which the editor font does not carry.
fn query_swatch_ui(ui: &mut egui::Ui, color: Option<egui::Color32>, hover: &str) {
    let (rect, response) =
        ui.allocate_exact_size(egui::Vec2::splat(swatch_side(ui)), Sense::hover());
    if let Some(color) = color {
        ui.painter()
            .rect_filled(rect.shrink(SWATCH_INSET_PX), SWATCH_CORNER_RADIUS_PX, color);
    }
    response.on_hover_text(hover);
}

/// The hover text of a nav-point row: which point of its track the row holds,
/// and where that point was recorded.
fn point_hover_text(point_index: usize, lat_lon: Option<(f64, f64)>) -> String {
    match lat_lon {
        Some((lat, lon)) => format!("#{point_index}\n{lat:.5}, {lon:.5}"),
        None => format!("#{point_index}"),
    }
}

/// One track's providers, absent for a track that is no longer loaded.
fn track_values<'a>(
    files: &'a [LoadedFile],
    results: &'a PointsResults,
    track: TrackRef,
) -> Option<TrackValues<'a>> {
    let points = track.resolve(files)?.points.as_slice();
    let data = results.track_data(track);
    // Match tables need no channel data: they read only metric columns.
    let provider = TrackProvider::new(points, &[], data);
    let slice_start = data.map_or(0, TrackQueryData::slice_start);
    Some(TrackValues {
        points,
        provider,
        slice: SliceProvider::new(
            provider,
            slice_start,
            points.len().saturating_sub(slice_start),
        ),
        slice_start,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_point_row_states_its_index_and_position() {
        assert_eq!(
            point_hover_text(150, Some((55.676_23, 12.568_9))),
            "#150\n55.67623, 12.56890"
        );
        assert_eq!(point_hover_text(150, None), "#150");
    }
}

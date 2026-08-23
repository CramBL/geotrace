//! The results tab: what each query of the run counted, the run's matches as a
//! table to pick from, and the rows of the picked match under their columns.

use std::collections::BTreeSet;
use std::fmt::Write as _;

use egui::text::LayoutJob;
use egui::{
    Align, Button, CursorIcon, Label, Layout, RichText, Sense, TextFormat, TextStyle, TextWrapMode,
};
use egui_extras::{Column, TableBuilder, TableRow};
use egui_phosphor::regular::CARET_DOWN as ICON_CARET_DOWN;
use egui_phosphor::regular::COPY as ICON_COPY;
use egui_phosphor::regular::CROSSHAIR as ICON_CROSSHAIR;
use egui_phosphor::regular::FRAME_CORNERS as ICON_FRAME_CORNERS;
use egui_phosphor::regular::INFO as ICON_INFO;
use gt_query::{ChannelTimeline, Construct, MetricProvider as _, QueryMetric};
use gt_query_run::{
    ChannelResults, ChannelTrackResult, PointsResults, QuerySummary, RunResults, SliceProvider,
    TrackProvider, TrackQueryData,
};
use gt_side_panel::widgets::{PointClickRequests, apply_point_click};
use gt_types::{DataCategory, LoadedFile, NavPoint, PointIdx, TrackRef};
use gt_ui_theme::{EM_DASH, MIDDLE_DOT};
use gt_ui_types::{
    DataPointRef, HighlightScope, MapHighlight, MapScope, MatchHighlight, MatchRevealTarget,
};
use strum::IntoEnumIterator as _;

use super::column_format::{self, ColumnFormat};
use super::match_row::{MatchColumn, MatchKey, MatchRow, MatchRows, MatchSort, RowNoun};

/// Vertical padding a row adds around its text.
pub(super) const ROW_PADDING: f32 = 2.0;

/// Matches listed before the matches table scrolls, so the points table below
/// it keeps most of the window.
const VISIBLE_MATCH_ROWS: usize = 5;

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

/// How the results tab lists a run's matches: the order of the matches table,
/// and the match the points table follows.
#[derive(Debug, Default)]
pub(super) struct MatchListState {
    sort: MatchSort,
    /// The picked match, kept across reruns. `None` until one is picked, which
    /// lists the first match.
    selected: Option<MatchKey>,
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
        let point = self.points.get(point_index)?;
        Some((point.tpv.lat().as_degrees(), point.tpv.lon().as_degrees()))
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
    ShowOnMap(MatchRevealTarget),
}

/// One run's results as the two tables of the results tab: every match of the
/// run to pick from, and the rows of the picked match under the columns its
/// query tables.
pub(super) struct ResultsTables<'a> {
    files: &'a [LoadedFile],
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

    /// The summary strip, the matches table, and the rows of the picked match
    /// under it.
    pub(super) fn ui(
        &mut self,
        ui: &mut egui::Ui,
        state: &mut MatchListState,
        scope: MapScope<'_>,
        out: &mut ResultsOutputs<'_, '_>,
    ) {
        self.matches.sort(state.sort);
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
            self.summary_strip_ui(ui, out.reveal);
            let Some(selected) = selected else {
                ui.label(RichText::new("No matches").weak());
                return;
            };
            ui.add_space(ui.spacing().item_spacing.y);
            self.matches_table_ui(ui, state, &selected, out);
            ui.add_space(ui.spacing().item_spacing.y);
            self.caption_ui(ui, &selected);
            self.points_table_ui(ui, &selected, scope, out);
        });
    }

    /// One line per query: what it matched, what it hides, and what it skipped,
    /// with the run-wide buttons on the first line.
    fn summary_strip_ui(&self, ui: &mut egui::Ui, reveal: &mut Option<MatchRevealTarget>) {
        for (index, query) in self.queries.iter().enumerate() {
            ui.horizontal(|ui| {
                if index == 0 {
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        self.copy_tsv_button_ui(ui);
                        self.show_run_on_map_button_ui(ui, reveal);
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

    /// One query's counts as one line: the numbers in the text colour, what
    /// they count dimmed, with everything the run left out on hover.
    fn summary_counts_ui(&self, ui: &mut egui::Ui, query: &QuerySection<'_>) {
        let summary = query.summary;
        let stated_in_full = summary.to_string();
        query_swatch_ui(ui, query.color, &stated_in_full);
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
        // Truncated to the width that is left, and stated in full on hover: a
        // line that extends would push the window over the map beside it.
        ui.add(Label::new(line.into_job()).truncate())
            .on_hover_text(stated_in_full);
    }

    /// The button framing the map on every match of this run and playing their
    /// reveal animation again. Disabled with the reason in its hover text when
    /// the run drew no halos, or when the data changed after it.
    fn show_run_on_map_button_ui(&self, ui: &mut egui::Ui, reveal: &mut Option<MatchRevealTarget>) {
        let disabled_reason = if self.stale {
            Some(format!(
                "Data changed since this run {EM_DASH} run again to show its matches"
            ))
        } else if !self.draws_halos {
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
    /// clicked. Clicking a row picks the match the points table follows.
    fn matches_table_ui(
        &self,
        ui: &mut egui::Ui,
        state: &mut MatchListState,
        selected: &MatchRow,
        out: &mut ResultsOutputs<'_, '_>,
    ) {
        let row_height = ui.text_style_height(&TextStyle::Body) + 2.0 * ROW_PADDING;
        let header_height = row_height;
        // Rows are separated by the item spacing, so the rows on display take
        // that much more than their own height.
        let visible_height = (row_height + ui.spacing().item_spacing.y) * VISIBLE_MATCH_ROWS as f32;
        let widths = self.match_column_widths(ui);
        let swatch_width = swatch_side(ui);
        let mut action: Option<MatchAction> = None;

        ui.scope(|ui| {
            // A selectable label senses clicks and drags of its own, which would
            // take the pointer from the row it sits in.
            ui.style_mut().interaction.selectable_labels = false;
            let mut table = TableBuilder::new(ui)
                .id_salt("query_match_list")
                .striped(true)
                .sense(Sense::click())
                .auto_shrink([false, true])
                .min_scrolled_height(0.0)
                .max_scroll_height(visible_height)
                .cell_layout(Layout::left_to_right(Align::Center))
                .column(Column::exact(swatch_width));
            for width in &widths {
                table = table.column(Column::exact(*width).clip(true));
            }
            // The trailing column holds the button framing the map on one
            // match, and stretches the striping across the whole table.
            table = table.column(Column::remainder());

            table
                .header(header_height, |mut header| {
                    header.col(|_| {});
                    for column in MatchColumn::iter() {
                        header.col(|ui| sort_header_ui(ui, column, state, self.row_noun));
                    }
                    header.col(|_| {});
                })
                .body(|body| {
                    body.rows(row_height, self.matches.rows().len(), |mut row| {
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
            Some(MatchAction::ShowOnMap(target)) => *out.reveal = Some(target),
            None => {}
        }
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
            let text = match_row.cell_text(column, self.files);
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
            if show_match_on_map_button_ui(ui, target.as_ref().err().map(String::as_str)).clicked()
            {
                reveal = target.as_ref().ok().cloned();
            }
        });

        let response = row.response();
        if response.hovered() {
            self.hover_match(match_row, highlight);
        }
        if let Some(target) = reveal {
            return Some(MatchAction::ShowOnMap(target));
        }
        response
            .clicked()
            .then(|| MatchAction::Select(match_row.key()))
    }

    /// What the button at the end of a match's row frames the map on, or the
    /// reason it cannot.
    fn reveal_target(&self, match_row: &MatchRow) -> Result<MatchRevealTarget, String> {
        if self.stale {
            return Err(format!(
                "Data changed since this run {EM_DASH} run again to show this match"
            ));
        }
        match self.source {
            RowSource::NavPoints { .. } => Ok(MatchRevealTarget::OneMatch {
                track: match_row.track,
                points: match_row.rows.clone(),
            }),
            RowSource::ChannelSamples { .. } => Err("Channel samples have no position of their \
                 own: \"Show on map\" above frames the whole run"
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
            let cells =
                column_format::text_width(ui, column.widest_cell_text(), &TextStyle::Monospace);
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
            + column_format::text_width(ui, ICON_FRAME_CORNERS, &TextStyle::Body)
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
            MatchColumn::Track.widest_cell_text(),
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
        scope: MapScope<'_>,
        out: &mut ResultsOutputs<'_, '_>,
    ) {
        let Some(columns) = self
            .queries
            .get(selected.query_index)
            .map(|query| query.columns.as_slice())
        else {
            return;
        };
        let widths: Vec<f32> = columns
            .iter()
            .map(|column| column.format.column_width(ui, &column.name))
            .collect();
        let row_height = ui.text_style_height(&TextStyle::Monospace) + ROW_PADDING;
        let header_height = column_format::header_height(ui);
        let mut click: Option<PointClick> = None;

        ui.scope(|ui| {
            // A selectable label senses clicks and drags of its own, which would
            // take the pointer from the row it sits in.
            ui.style_mut().interaction.selectable_labels = false;
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
                            columns,
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

    /// One matched row: its value under each column the table shows, and the
    /// click it took.
    fn point_row_ui(
        &self,
        row: &mut TableRow<'_, '_>,
        columns: &[TableColumn<'_>],
        track: TrackRef,
        source_index: usize,
        highlight: &mut MapHighlight,
    ) -> Option<PointClick> {
        let point = self.source.map_point(track, source_index);
        row.set_selected(
            point.is_some_and(|point| highlight.sticky.is_some_and(|sticky| sticky == point)),
        );
        for column in columns {
            let value = self.source.value(track, source_index, column.source);
            row.col(|ui| column.format.value_ui(ui, value));
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

/// A clickable matches-table header that orders the list by `column`.
///
/// The active column shows a caret pointing the way its values run. Clicking it
/// reverses that, clicking any other column switches to it.
fn sort_header_ui(
    ui: &mut egui::Ui,
    column: MatchColumn,
    state: &mut MatchListState,
    row_noun: RowNoun,
) {
    let active = state.sort.column == column;
    let clicked = ui
        .with_layout(column.cell_layout(), |ui| {
            let title = ui.add(
                Label::new(RichText::new(column.title(row_noun)).strong())
                    .selectable(false)
                    .wrap_mode(TextWrapMode::Extend)
                    .sense(Sense::click()),
            );
            if active {
                ui.label(RichText::new(state.sort.direction.caret()).small().weak());
            }
            title
        })
        .inner
        // Pointer cursor, not the text I-beam.
        .on_hover_cursor(CursorIcon::PointingHand)
        .on_hover_text(format!(
            "Click to sort {}",
            column.order_hint(state.sort.next_direction(column))
        ))
        .clicked();
    if clicked {
        state.sort.clicked(column);
    }
}

/// The button framing the map on one match, at the right end of its row.
/// Disabled with the reason on hover where it cannot answer.
fn show_match_on_map_button_ui(ui: &mut egui::Ui, disabled_reason: Option<&str>) -> egui::Response {
    // A right-to-left scope: the button is drawn at its own size at the end of
    // the row.
    ui.scope_builder(
        egui::UiBuilder::new().layout(Layout::right_to_left(Align::Center)),
        |ui| {
            let enabled = disabled_reason.is_none();
            let response = ui.add_enabled(enabled, Button::new(ICON_FRAME_CORNERS).small());
            match disabled_reason {
                Some(reason) => response.on_disabled_hover_text(reason),
                None => response.on_hover_text("Show on map"),
            }
        },
    )
    .inner
}

/// A line stating what a run counted, as the summary strip and the caption
/// below the matches table state it: the numbers in the text colour, the words
/// around them dimmed. One line is one label, so it reads as one line rather
/// than as the widgets it is built from.
struct CountLine {
    job: LayoutJob,
    number_format: TextFormat,
    words_format: TextFormat,
}

impl CountLine {
    fn new(ui: &egui::Ui) -> Self {
        let font = TextStyle::Body.resolve(ui.style());
        Self {
            job: LayoutJob::default(),
            number_format: TextFormat {
                font_id: font.clone(),
                color: ui.visuals().text_color(),
                ..Default::default()
            },
            words_format: TextFormat {
                font_id: font,
                color: ui.visuals().weak_text_color(),
                ..Default::default()
            },
        }
    }

    fn number(mut self, number: usize) -> Self {
        self.job
            .append(&number.to_string(), 0.0, self.number_format.clone());
        self
    }

    fn words(mut self, words: &str) -> Self {
        self.job.append(words, 0.0, self.words_format.clone());
        self
    }

    /// A number and what it counts.
    fn count(self, count: usize, noun: &str) -> Self {
        self.number(count).words(&format!(" {noun}"))
    }

    /// The dot between two counts.
    fn dot(self) -> Self {
        self.words(&format!(" {MIDDLE_DOT} "))
    }

    fn into_job(self) -> LayoutJob {
        self.job
    }
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

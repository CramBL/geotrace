//! One query's matches as a single table: a row naming each match, a row per
//! matched nav point or channel sample, and one column header above them all.

use std::collections::{BTreeSet, HashSet};
use std::fmt::Write as _;
use std::ops::Range;

use egui::{Button, RichText, TextStyle, TextWrapMode, WidgetInfo, WidgetText, WidgetType};
use egui_extras::{Column, TableBuilder, TableRow};
use egui_phosphor::regular::CARET_DOWN as ICON_CARET_DOWN;
use egui_phosphor::regular::CARET_RIGHT as ICON_CARET_RIGHT;
use egui_phosphor::regular::FRAME_CORNERS as ICON_FRAME_CORNERS;
use gt_fmt::EN_DASH;
use gt_query::{Construct, MetricProvider as _, QueryMetric, TrackMatches};
use gt_query_run::{
    ChannelResults, ChannelTrackResult, PanelQuery, PointsResults, SliceProvider, TrackProvider,
    TrackQueryData,
};
use gt_side_panel::widgets::{PointClickRequests, apply_point_click};
use gt_types::{DataCategory, LoadedFile, NavPoint, PointIdx, TrackRef};
use gt_ui_theme::EM_DASH;
use gt_ui_types::{
    DataPointRef, HighlightScope, MapHighlight, MapScope, MatchHighlight, MatchRevealTarget,
};

use super::column_format::{self, ColumnFormat};
use super::{match_name_text, sample_span_name_text};

/// Width of the rule down the left of a group's rows, in the halo colour its
/// query paints on the map.
const MATCH_RULE_WIDTH: f32 = 3.0;

/// Vertical padding a row adds around its text.
pub(super) const ROW_PADDING: f32 = 2.0;

/// The second line of a sample row's hover, under its index.
const SAMPLE_WITHOUT_POSITION: &str = "Samples have no position: nothing to pin";

/// What one table writes back to the app: the cross-highlight its rows drive,
/// the point a click pins, the groups it folds, and the match to frame on the
/// map.
pub(super) struct MatchTableOutputs<'a, 'b> {
    pub(super) highlight: &'a mut MapHighlight,
    pub(super) requests: &'a mut PointClickRequests<'b>,
    pub(super) folds: &'a mut FoldedMatches,
    pub(super) reveal: &'a mut Option<MatchRevealTarget>,
}

/// A group's identity across reruns: the track it is on and the row it starts
/// at. A rerun of the same query over unchanged data produces the same key, so
/// the group keeps its fold state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) struct MatchKey {
    track: TrackRef,
    first_row: usize,
}

/// The groups whose value rows are folded away. An empty set is every group
/// expanded, which is what a first run shows.
#[derive(Debug, Default)]
pub(super) struct FoldedMatches(HashSet<MatchKey>);

impl FoldedMatches {
    fn contains(&self, key: MatchKey) -> bool {
        self.0.contains(&key)
    }

    fn toggle(&mut self, key: MatchKey) {
        if !self.0.remove(&key) {
            self.0.insert(key);
        }
    }

    /// Whether every group of `groups` is folded. Vacuously true for a query
    /// that matched nothing.
    pub(super) fn all_folded(&self, groups: &[RowGroup]) -> bool {
        groups.iter().all(|group| self.contains(group.key()))
    }

    pub(super) fn fold_all(&mut self, groups: &[RowGroup]) {
        self.0.extend(groups.iter().map(RowGroup::key));
    }

    /// Expands every group of `groups`, leaving the fold state of the groups
    /// other queries list untouched.
    pub(super) fn expand_all(&mut self, groups: &[RowGroup]) {
        for group in groups {
            self.0.remove(&group.key());
        }
    }
}

/// One stretch of rows the table lists under a name row and folds as a unit:
/// one match of a points query, or one track's matched channel samples.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RowGroup {
    track: TrackRef,
    /// Indices into the track's row source: its nav points, or the samples of
    /// the channel timeline.
    rows: Range<usize>,
}

impl RowGroup {
    /// Every match of a points query, in the order the run listed them.
    fn of_query_matches(matches: &[TrackMatches]) -> Vec<Self> {
        matches
            .iter()
            .flat_map(|track_matches| {
                track_matches.ranges.iter().map(|rows| Self {
                    track: track_matches.track,
                    rows: rows.clone(),
                })
            })
            .collect()
    }

    /// Every matched sample range of a channel-source run, in the order the run
    /// listed the tracks.
    fn of_channel_samples(tracks: &[ChannelTrackResult]) -> Vec<Self> {
        tracks
            .iter()
            .flat_map(|result| {
                result.ranges.iter().map(|rows| Self {
                    track: result.track,
                    rows: rows.clone(),
                })
            })
            .collect()
    }

    fn key(&self) -> MatchKey {
        MatchKey {
            track: self.track,
            first_row: self.rows.start,
        }
    }

    /// The index span the group covers, as the hover states it: `#150–#300`,
    /// or the bare index of a single-row group.
    fn index_span_text(&self) -> String {
        match gt_fmt::last_index_of_span(&self.rows) {
            Some(last) => format!("#{}{EN_DASH}#{last}", self.rows.start),
            None => format!("#{}", self.rows.start),
        }
    }
}

/// One group as the table lays it out: the rows it covers and whether they are
/// folded away.
#[derive(Debug, PartialEq, Eq)]
struct GroupEntry {
    group: RowGroup,
    folded: bool,
}

/// Where one row of the table comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MatchTableRow {
    /// The row naming a group and its extent.
    Name { group_index: usize },
    /// One row of the group's source, by its index into that source.
    Value {
        group_index: usize,
        source_index: usize,
    },
}

/// The flat row sequence of one table's groups: each group contributes a name
/// row, followed by one row per source row it covers unless it is folded.
///
/// Resolves a row index to its group without walking the groups before it:
/// the table renders only the rows on screen.
struct MatchTableRows {
    groups: Vec<GroupEntry>,
    /// Row index of each group's name row, ascending.
    name_rows: Vec<usize>,
    total_rows: usize,
}

impl MatchTableRows {
    fn of_groups(groups: &[RowGroup], folds: &FoldedMatches) -> Self {
        let mut entries = Vec::with_capacity(groups.len());
        let mut name_rows = Vec::with_capacity(groups.len());
        let mut total_rows = 0;
        for group in groups {
            let folded = folds.contains(group.key());
            name_rows.push(total_rows);
            total_rows += 1 + if folded { 0 } else { group.rows.len() };
            entries.push(GroupEntry {
                group: group.clone(),
                folded,
            });
        }
        Self {
            groups: entries,
            name_rows,
            total_rows,
        }
    }

    fn total_rows(&self) -> usize {
        self.total_rows
    }

    fn entry(&self, group_index: usize) -> Option<&GroupEntry> {
        self.groups.get(group_index)
    }

    /// What the table draws at `row`.
    fn row_at(&self, row: usize) -> Option<MatchTableRow> {
        let group_index = self.name_rows.partition_point(|&start| start <= row);
        let group_index = group_index.checked_sub(1)?;
        let start = self.name_rows.get(group_index)?;
        let entry = self.groups.get(group_index)?;
        let offset = row.checked_sub(*start)?;
        match offset.checked_sub(1) {
            None => Some(MatchTableRow::Name { group_index }),
            Some(_) if entry.folded => None,
            Some(row_offset) => {
                let source_index = entry.group.rows.start.checked_add(row_offset)?;
                (source_index < entry.group.rows.end).then_some(MatchTableRow::Value {
                    group_index,
                    source_index,
                })
            }
        }
    }
}

/// One column of a match table: what its header names, the documentation that
/// header explains on hover, and how its cells print.
struct TableColumn<'a> {
    name: String,
    doc: Option<&'static Construct>,
    format: ColumnFormat<'a>,
}

/// One track's values, as the run computed them. A row reads its value
/// without rebuilding a provider: this is built once per track the query
/// matched.
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
    fn value(&self, column: QueryMetric, point_index: usize) -> Option<f64> {
        if column == QueryMetric::Accel {
            return point_index
                .checked_sub(self.slice_start)
                .and_then(|relative| gt_query::derived_accel(&self.slice, relative));
        }
        self.provider.value(column, point_index)
    }

    fn lat_lon(&self, point_index: usize) -> Option<(f64, f64)> {
        let point = self.points.get(point_index)?;
        Some((point.tpv.lat().as_degrees(), point.tpv.lon().as_degrees()))
    }
}

/// Where one table's rows come from: the nav points a points query matched, or
/// the samples a channel-source query matched on the channel's own timeline.
enum RowSource<'a> {
    NavPoints {
        /// The metrics the query tables, one per value column.
        metrics: &'a [QueryMetric],
        /// One entry per track the query matched, in track order.
        track_values: Vec<(TrackRef, TrackValues<'a>)>,
    },
    ChannelSamples {
        tracks: &'a [ChannelTrackResult],
    },
}

impl RowSource<'_> {
    /// The value under column `column` of the row `source_index` addresses on
    /// `track`.
    fn value(&self, track: TrackRef, source_index: usize, column: usize) -> Option<f64> {
        match self {
            Self::NavPoints {
                metrics,
                track_values,
            } => {
                let values = track_values
                    .iter()
                    .find(|(candidate, _)| *candidate == track)
                    .map(|(_, values)| values)?;
                values.value(*metrics.get(column)?, source_index)
            }
            Self::ChannelSamples { tracks } => {
                let timeline = &tracks.iter().find(|result| result.track == track)?.timeline;
                // Column 0 holds the sample time, the columns after it the
                // channel's components.
                match column.checked_sub(1) {
                    None => timeline.times.get(source_index).copied(),
                    Some(component) => timeline.value(source_index, component),
                }
            }
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
            Self::NavPoints { track_values, .. } => track_values
                .iter()
                .find(|(candidate, _)| *candidate == track)
                .and_then(|(_, values)| values.lat_lon(source_index)),
            Self::ChannelSamples { .. } => None,
        }
    }

    /// The line naming one group, as its name row reads.
    fn group_name(&self, files: &[LoadedFile], group: &RowGroup) -> String {
        match self {
            Self::NavPoints { .. } => match_name_text(files, group.track, &group.rows),
            Self::ChannelSamples { tracks } => {
                let times = tracks
                    .iter()
                    .find(|result| result.track == group.track)
                    .map_or(&[][..], |result| result.timeline.times.as_slice());
                sample_span_name_text(files, group.track, times, &group.rows)
            }
        }
    }

    /// What one row's hover states: which row of its source it holds, and
    /// where that row was recorded.
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

    /// What one row is called in the copied table's second column.
    fn row_noun(&self) -> &'static str {
        match self {
            Self::NavPoints { .. } => "point",
            Self::ChannelSamples { .. } => "sample",
        }
    }

    fn copy_tooltip(&self) -> &'static str {
        match self {
            Self::NavPoints { .. } => {
                "Copy as tab-separated values: one line per matched point, \
                 starting with the number of the match it belongs to and its \
                 index in the track"
            }
            Self::ChannelSamples { .. } => {
                "Copy as tab-separated values: one line per matched sample, \
                 starting with the number of the match it belongs to and its \
                 index in the channel's samples"
            }
        }
    }
}

/// What a row click requests from the app, applied once the table is laid out
/// and the enclosing panel's `Ui` is available again.
struct PointClick {
    point: DataPointRef,
    lat_lon: (f64, f64),
    response: egui::Response,
}

/// The action a click in the table selects. Applied after the table is laid
/// out: folding a group changes the row count the table body is being laid out
/// from.
enum RowAction {
    PinPoint(PointClick),
    ToggleFold(MatchKey),
    ShowMatchOnMap(MatchRevealTarget),
}

/// One run's matches as a single table: the nav points a points query matched,
/// or the samples a channel-source query matched.
///
/// A value column reads the same way from the first group to the last: every
/// group shares the column layout and the header.
///
/// Hovering a group's name echoes it on the map and the plot, and clicking it
/// folds the group's value rows away. Hovering a nav-point row rings that
/// point. Clicking one pins it, like a point row in the side panel.
pub(super) struct MatchTable<'a> {
    id_salt: egui::Id,
    files: &'a [LoadedFile],
    groups: Vec<RowGroup>,
    columns: Vec<TableColumn<'a>>,
    source: RowSource<'a>,
    /// The colour of the rule beside a group's rows, absent for a query that
    /// draws no halos to match it to.
    rule_color: Option<egui::Color32>,
    /// Whether the data changed after the run that produced these rows.
    stale: bool,
}

impl<'a> MatchTable<'a> {
    /// The table for one points query: a row per matched nav point, under the
    /// metric columns the query tables.
    pub(super) fn of_query_matches(
        files: &'a [LoadedFile],
        results: &'a PointsResults,
        query: &'a PanelQuery,
        query_index: usize,
    ) -> Self {
        let stale = results.matches.stale;
        let groups = RowGroup::of_query_matches(&query.matches);
        let tracks: BTreeSet<TrackRef> = groups.iter().map(|group| group.track).collect();
        let columns = query
            .columns
            .iter()
            .map(|metric| TableColumn {
                name: metric.to_string(),
                doc: gt_query::metric_documentation(*metric),
                format: ColumnFormat::of_metric(*metric),
            })
            .collect();
        Self {
            id_salt: egui::Id::new(("query_match_table", query_index)),
            files,
            groups,
            columns,
            source: RowSource::NavPoints {
                metrics: &query.columns,
                track_values: tracks
                    .into_iter()
                    .filter_map(|track| Some((track, track_values(files, results, track)?)))
                    .collect(),
            },
            rule_color: query
                .color
                .map(|color| gt_ui_theme::query_halo_color(color, stale)),
            stale,
        }
    }

    /// The table for a channel-source run: a row per matched sample, under the
    /// sample time and one column per channel component.
    ///
    /// The whole table reads in the unit the first track declared: the
    /// evaluator timed and valued every track's samples in base units, and a
    /// run whose tracks declare incompatible units is rejected by the check
    /// before it gets here.
    pub(super) fn of_channel_samples(files: &'a [LoadedFile], channel: &'a ChannelResults) -> Self {
        let stale = channel.matches.stale;
        // Samples are timed finer than points, so their times keep the
        // milliseconds the point tables leave out.
        let mut columns = vec![TableColumn {
            name: "time".to_owned(),
            doc: None,
            format: ColumnFormat::time_of_day_with_millis(),
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
        columns.extend(component_names.iter().map(|name| TableColumn {
            name: name.clone(),
            doc: None,
            format: ColumnFormat::of_channel_unit(unit),
        }));
        Self {
            id_salt: egui::Id::new(("channel_match_table", channel.channel.as_str())),
            files,
            groups: RowGroup::of_channel_samples(&channel.tracks),
            columns,
            source: RowSource::ChannelSamples {
                tracks: &channel.tracks,
            },
            rule_color: channel
                .matches
                .draws
                .first()
                .map(|layer| gt_ui_theme::query_halo_color(layer.color, stale)),
            stale,
        }
    }

    /// The groups this table lists, for the section header that folds them all.
    pub(super) fn groups(&self) -> &[RowGroup] {
        &self.groups
    }

    pub(super) fn copy_tooltip(&self) -> &'static str {
        self.source.copy_tooltip()
    }

    /// What the button at the end of a group's name row frames the map on, or
    /// the reason it cannot.
    fn reveal_target(&self, group: &RowGroup) -> Result<MatchRevealTarget, String> {
        if self.stale {
            return Err(format!(
                "Data changed since this run {EM_DASH} run again to show this match"
            ));
        }
        match self.source {
            RowSource::NavPoints { .. } => Ok(MatchRevealTarget::OneMatch {
                track: group.track,
                points: group.rows.clone(),
            }),
            RowSource::ChannelSamples { .. } => Err("Channel samples have no position of their \
                 own: \"Show on map\" above frames the whole run"
                .to_owned()),
        }
    }

    pub(super) fn ui(
        &self,
        ui: &mut egui::Ui,
        scope: MapScope<'_>,
        out: &mut MatchTableOutputs<'_, '_>,
    ) {
        if self.groups.is_empty() {
            ui.label(RichText::new("No matches").weak());
            return;
        }
        let rows = MatchTableRows::of_groups(&self.groups, out.folds);
        let column_widths: Vec<f32> = self
            .columns
            .iter()
            .map(|column| column.format.column_width(ui, &column.name))
            .collect();
        let row_height = ui.text_style_height(&TextStyle::Monospace) + ROW_PADDING;
        let header_height = column_format::header_height(ui);
        let mut action: Option<RowAction> = None;

        ui.scope(|ui| {
            // A stale run's rows reference indices that may no longer address
            // the same data: they are shown, but answer nothing.
            if self.stale {
                ui.disable();
            }
            // A selectable label senses clicks and drags of its own, which would
            // take the pointer from the row it sits in.
            ui.style_mut().interaction.selectable_labels = false;

            let mut table = TableBuilder::new(ui)
                .id_salt(self.id_salt)
                .striped(true)
                .sense(egui::Sense::click())
                .auto_shrink([false, true])
                // The results tab scrolls these rows: they are virtualized
                // against its viewport.
                .vscroll(false)
                .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
                // The rule column does not clip: `paint_match_rule` paints into
                // the gap below its row as well.
                .column(
                    Column::initial(MATCH_RULE_WIDTH).range(MATCH_RULE_WIDTH..=MATCH_RULE_WIDTH),
                );
            for width in &column_widths {
                // Exact columns clip: a value wider than the width its column was
                // sized for is cut off there, and the table stays as wide as the
                // window it is in.
                table = table.column(Column::exact(*width));
            }
            // The trailing column holds no value: it stretches the striping and the
            // hover fill across the full width of the table, and is where a group's
            // name is painted from.
            table = table.column(Column::remainder());

            table
                .header(header_height, |mut header| {
                    header.col(|_| {});
                    for column in &self.columns {
                        header.col(|ui| {
                            column.format.header_ui(ui, &column.name, column.doc);
                        });
                    }
                    header.col(|_| {});
                })
                .body(|body| {
                    body.rows(row_height, rows.total_rows(), |mut row| {
                        if let Some(taken) = self.row_ui(&mut row, &rows, out.highlight) {
                            action = Some(taken);
                        }
                    });
                });
        });

        // Applied out here, where the panel's `Ui` places the pinned window beside
        // it, and where folding a group no longer changes a row count being read.
        match action {
            Some(RowAction::PinPoint(click)) => apply_point_click(
                ui,
                &click.response,
                click.point,
                click.lat_lon,
                scope,
                out.highlight,
                out.requests,
            ),
            Some(RowAction::ToggleFold(key)) => out.folds.toggle(key),
            Some(RowAction::ShowMatchOnMap(target)) => *out.reveal = Some(target),
            None => {}
        }
    }

    /// This whole result as tab-separated values: a header line naming every
    /// column in the unit its values are in, then one line per matched row,
    /// numbered by the group it belongs to.
    ///
    /// Folded groups are written out too: the copy holds the whole run,
    /// whatever the table shows.
    pub(super) fn as_tsv(&self) -> String {
        let mut tsv = format!("match\t{}", self.source.row_noun());
        for column in &self.columns {
            tsv.push('\t');
            tsv.push_str(&column.format.header_with_unit(&column.name));
        }
        tsv.push('\n');

        for (index, group) in self.groups.iter().enumerate() {
            let match_number = index + 1;
            for source_index in group.rows.clone() {
                // Writing to a String cannot fail.
                write!(tsv, "{match_number}\t{source_index}").ok();
                for (column_index, column) in self.columns.iter().enumerate() {
                    tsv.push('\t');
                    let value = self.source.value(group.track, source_index, column_index);
                    if let Some(value) = value {
                        tsv.push_str(&column.format.cell_text(Some(value)));
                    }
                }
                tsv.push('\n');
            }
        }
        tsv
    }

    /// One row of the table, and the action a click on it selects.
    fn row_ui(
        &self,
        row: &mut TableRow<'_, '_>,
        rows: &MatchTableRows,
        highlight: &mut MapHighlight,
    ) -> Option<RowAction> {
        match rows.row_at(row.index())? {
            MatchTableRow::Name { group_index } => {
                let entry = rows.entry(group_index)?;
                let reveal = self.group_name_row_ui(row, entry);
                let response = row.response().on_hover_text(entry.group.index_span_text());
                if response.hovered() {
                    self.hover_group(&entry.group, highlight);
                }
                if let Some(target) = reveal {
                    return Some(RowAction::ShowMatchOnMap(target));
                }
                response
                    .clicked()
                    .then(|| RowAction::ToggleFold(entry.group.key()))
            }
            MatchTableRow::Value {
                group_index,
                source_index,
            } => self
                .value_row_ui(
                    row,
                    &rows.entry(group_index)?.group,
                    source_index,
                    highlight,
                )
                .map(RowAction::PinPoint),
        }
    }

    /// What hovering a group's name echoes elsewhere. A channel sample range
    /// indexes the channel's own timeline, so it bands nothing on the map: its
    /// track still takes focus.
    fn hover_group(&self, group: &RowGroup, highlight: &mut MapHighlight) {
        if let RowSource::NavPoints { .. } = self.source {
            highlight.hover_match = Some(MatchHighlight::new(group.track, &group.rows));
        }
        // Track focus alongside the band: the map fades the other tracks and
        // the plot dims their series, like hovering the track in the side
        // panel.
        highlight.hover = Some(HighlightScope::Track(group.track));
    }

    /// The row naming one group: the fold caret, the group's name reading
    /// across the value columns it leaves empty, and the button framing the map
    /// on it. Answers with what that button was pressed for.
    fn group_name_row_ui(
        &self,
        row: &mut TableRow<'_, '_>,
        entry: &GroupEntry,
    ) -> Option<MatchRevealTarget> {
        // The row fills as many columns as the table declared: the name is painted
        // over them from the last one.
        debug_assert!(
            !self.columns.is_empty(),
            "a table always starts with a time column"
        );
        row.set_overline(true);
        // The hover fill would paint over the name where it reads past the first
        // value column: a hovered group answers on the map and the plot instead.
        row.set_hovered(false);
        row.col(|ui| paint_match_rule(ui, self.rule_color));
        let (first_value_cell, _) = row.col(|_| {});
        for _ in self.columns.iter().skip(1) {
            row.col(|_| {});
        }
        let caret = if entry.folded {
            ICON_CARET_RIGHT
        } else {
            ICON_CARET_DOWN
        };
        let name = format!(
            "{caret} {}",
            self.source.group_name(self.files, &entry.group)
        );
        let target = self.reveal_target(&entry.group);
        let mut reveal = None;
        row.col(|ui| {
            let button = show_match_on_map_button_ui(ui, target.as_ref().err().map(String::as_str));
            if button.clicked() {
                reveal = target.as_ref().ok().cloned();
            }
            let span = egui::Rangef::new(first_value_cell.left(), button.rect.left());
            group_name_ui(ui, span, &name);
        });
        reveal
    }

    /// One matched row: its value under each column the table shows, and the
    /// click it took.
    fn value_row_ui(
        &self,
        row: &mut TableRow<'_, '_>,
        group: &RowGroup,
        source_index: usize,
        highlight: &mut MapHighlight,
    ) -> Option<PointClick> {
        let point = self.source.map_point(group.track, source_index);
        row.set_selected(
            point.is_some_and(|point| highlight.sticky.is_some_and(|sticky| sticky == point)),
        );
        row.col(|ui| paint_match_rule(ui, self.rule_color));
        for (column_index, column) in self.columns.iter().enumerate() {
            let value = self.source.value(group.track, source_index, column_index);
            row.col(|ui| column.format.value_ui(ui, value));
        }
        row.col(|_| {});

        let response = row
            .response()
            .on_hover_text(self.source.row_hover_text(group.track, source_index));
        let point = point?;
        if response.hovered() {
            // The ring the plot cursor draws: the row and the map then agree
            // on which point is meant.
            highlight.plot_hover_point =
                Some((group.track.fi, group.track.index, point.point_index));
        }
        if !response.clicked() && !response.double_clicked() {
            return None;
        }
        Some(PointClick {
            point,
            lat_lon: self.source.lat_lon(group.track, source_index)?,
            response,
        })
    }
}

/// The button framing the map on this one group, at the right end of its row.
/// Disabled with the reason on hover where it cannot answer.
fn show_match_on_map_button_ui(ui: &mut egui::Ui, disabled_reason: Option<&str>) -> egui::Response {
    // A right-to-left scope: the button is drawn at its own size at the end of
    // the row, and the cell claims only the width it covers.
    ui.scope_builder(
        egui::UiBuilder::new().layout(egui::Layout::right_to_left(egui::Align::Center)),
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

/// One group's name, painted onto the row over `span`, which reaches from the
/// left edge of the first value column to the button at the row's right end.
///
/// The name is painted and nothing is allocated for it: a widget claims the
/// width it draws, and a cell wider than its column widens the table past the
/// window, which the auto-sized window then follows. Painting it from the last
/// cell puts it over the backgrounds the cells before it fill in as they are
/// added.
fn group_name_ui(ui: &egui::Ui, span: egui::Rangef, name: &str) {
    let galley = WidgetText::from(RichText::new(name).strong()).into_galley(
        ui,
        Some(TextWrapMode::Truncate),
        span.span().max(0.0),
        TextStyle::Body,
    );
    let top_left = egui::pos2(span.min, ui.max_rect().center().y - galley.size().y * 0.5);
    // Registered over the text it painted, without allocating it: the rect a
    // screen reader (and a UI test) reads is where the name actually is.
    ui.interact(
        egui::Rect::from_min_size(top_left, galley.size()),
        ui.id().with("match_name"),
        egui::Sense::hover(),
    )
    .widget_info(|| WidgetInfo::labeled(WidgetType::Label, ui.is_enabled(), name));
    ui.painter()
        .galley(top_left, galley, ui.visuals().strong_text_color());
}

/// The hover text of a nav-point row: which point of its track the row holds,
/// and where that point was recorded.
fn point_hover_text(point_index: usize, lat_lon: Option<(f64, f64)>) -> String {
    match lat_lon {
        Some((lat, lon)) => format!("#{point_index}\n{lat:.5}, {lon:.5}"),
        None => format!("#{point_index}"),
    }
}

/// The rule marking a group's rows in the colour its halos are drawn in.
fn paint_match_rule(ui: &egui::Ui, color: Option<egui::Color32>) {
    let Some(color) = color else {
        return;
    };
    // A group's rows read as one continuous rule: the rect is grown over the
    // gap to the row below.
    let cell = ui
        .max_rect()
        .expand2(egui::vec2(0.0, ui.spacing().item_spacing.y * 0.5));
    ui.painter().rect_filled(cell, 0.0, color);
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
    use gt_types::{FileIdx, TrackIdx};

    use super::*;

    fn track(index: usize) -> TrackRef {
        TrackRef::new(FileIdx::new(0), TrackIdx::new(index))
    }

    fn groups_of(ranges: Vec<(usize, Range<usize>)>) -> Vec<RowGroup> {
        ranges
            .into_iter()
            .map(|(track_index, rows)| RowGroup {
                track: track(track_index),
                rows,
            })
            .collect()
    }

    #[test]
    fn every_group_contributes_a_name_row_and_one_row_per_source_row() {
        let rows = MatchTableRows::of_groups(
            &groups_of(vec![(0, 0..3), (1, 10..12)]),
            &FoldedMatches::default(),
        );
        assert_eq!(rows.total_rows(), 7);
        assert_eq!(
            (0..8).map(|row| rows.row_at(row)).collect::<Vec<_>>(),
            vec![
                Some(MatchTableRow::Name { group_index: 0 }),
                Some(MatchTableRow::Value {
                    group_index: 0,
                    source_index: 0
                }),
                Some(MatchTableRow::Value {
                    group_index: 0,
                    source_index: 1
                }),
                Some(MatchTableRow::Value {
                    group_index: 0,
                    source_index: 2
                }),
                Some(MatchTableRow::Name { group_index: 1 }),
                Some(MatchTableRow::Value {
                    group_index: 1,
                    source_index: 10
                }),
                Some(MatchTableRow::Value {
                    group_index: 1,
                    source_index: 11
                }),
                None,
            ]
        );
    }

    #[test]
    fn a_track_with_several_groups_keeps_them_apart() {
        let matches = vec![TrackMatches {
            track: track(0),
            ranges: vec![0..1, 5..7],
        }];
        let groups = RowGroup::of_query_matches(&matches);
        let rows = MatchTableRows::of_groups(&groups, &FoldedMatches::default());
        assert_eq!(rows.total_rows(), 5);
        assert_eq!(rows.row_at(2), Some(MatchTableRow::Name { group_index: 1 }));
        assert_eq!(
            rows.row_at(3),
            Some(MatchTableRow::Value {
                group_index: 1,
                source_index: 5
            })
        );
        assert_eq!(
            rows.entry(1).map(|entry| entry.group.rows.clone()),
            Some(5..7)
        );
    }

    #[test]
    fn a_query_without_matches_has_no_rows() {
        let rows = MatchTableRows::of_groups(&[], &FoldedMatches::default());
        assert_eq!(rows.total_rows(), 0);
        assert_eq!(rows.row_at(0), None);
    }

    /// A folded group keeps its name row and drops its values: the group after
    /// it moves up by exactly the rows it folded away.
    #[test]
    fn a_folded_group_contributes_only_its_name_row() {
        let groups = groups_of(vec![(0, 0..3), (1, 10..12)]);
        let mut folds = FoldedMatches::default();
        folds.toggle(MatchKey {
            track: track(0),
            first_row: 0,
        });
        let rows = MatchTableRows::of_groups(&groups, &folds);

        assert_eq!(rows.total_rows(), 4);
        assert_eq!(
            (0..5).map(|row| rows.row_at(row)).collect::<Vec<_>>(),
            vec![
                Some(MatchTableRow::Name { group_index: 0 }),
                Some(MatchTableRow::Name { group_index: 1 }),
                Some(MatchTableRow::Value {
                    group_index: 1,
                    source_index: 10
                }),
                Some(MatchTableRow::Value {
                    group_index: 1,
                    source_index: 11
                }),
                None,
            ]
        );
    }

    #[test]
    fn folding_every_group_leaves_only_the_name_rows() {
        let groups = groups_of(vec![(0, 0..3), (1, 10..12)]);
        let mut folds = FoldedMatches::default();
        assert!(!folds.all_folded(&groups));
        folds.fold_all(&groups);
        assert!(folds.all_folded(&groups));
        assert_eq!(MatchTableRows::of_groups(&groups, &folds).total_rows(), 2);

        folds.expand_all(&groups);
        assert!(!folds.all_folded(&groups));
        assert_eq!(MatchTableRows::of_groups(&groups, &folds).total_rows(), 7);
    }

    /// Expanding one query's groups leaves the groups only another query lists
    /// folded: the two sections fold independently.
    #[test]
    fn expanding_one_query_leaves_the_other_queries_folds() {
        let shown = groups_of(vec![(0, 0..3)]);
        let other = groups_of(vec![(1, 10..12)]);
        let mut folds = FoldedMatches::default();
        folds.fold_all(&shown);
        folds.fold_all(&other);

        folds.expand_all(&shown);
        assert!(!folds.all_folded(&shown));
        assert!(folds.all_folded(&other));
    }

    #[test]
    fn a_group_states_the_index_span_it_covers() {
        let span = |rows: Range<usize>| {
            RowGroup {
                track: track(0),
                rows,
            }
            .index_span_text()
        };
        assert_eq!(span(150..301), "#150–#300");
        assert_eq!(span(150..151), "#150");
    }

    #[test]
    fn a_point_row_states_its_index_and_position() {
        assert_eq!(
            point_hover_text(150, Some((55.676_23, 12.568_9))),
            "#150\n55.67623, 12.56890"
        );
        assert_eq!(point_hover_text(150, None), "#150");
    }
}

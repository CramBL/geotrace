//! One query's matches as a single table: a row naming each match, a row per
//! matched point, and one column header above them all.

use std::collections::BTreeSet;
use std::ops::Range;

use egui::{Label, RichText, TextStyle, TextWrapMode};
use egui_extras::{Column, TableBuilder, TableRow};
use gt_query::{MetricProvider as _, QueryMetric, TrackMatches};
use gt_query_run::{PanelQuery, PointsResults, SliceProvider, TrackProvider, TrackQueryData};
use gt_side_panel::widgets::{PointClickRequests, apply_point_click};
use gt_types::{DataCategory, LoadedFile, NavPoint, PointIdx, TrackRef};
use gt_ui_types::{DataPointRef, HighlightScope, MapHighlight, MapScope, MatchHighlight};

use super::column_format::ColumnFormat;
use super::match_header_text;

/// Rows the table shows before it scrolls within the results panel.
const ROWS_BEFORE_SCROLLING: usize = 8;

/// Width of the rule down the left of a match's rows, in the halo colour its
/// query paints on the map.
const MATCH_RULE_WIDTH: f32 = 3.0;

/// Vertical padding a row adds around its text.
const ROW_PADDING: f32 = 2.0;

/// The shared inputs the rows read: the loaded files, the run whose derived
/// series back the values, and what the map draws.
pub(super) struct MatchTableContext<'a> {
    pub(super) files: &'a [LoadedFile],
    pub(super) results: &'a PointsResults,
    /// What the map draws: a row click pins only a point that is on it.
    pub(super) scope: MapScope<'a>,
}

/// One match of a query: the track it is on and the points it covers.
#[derive(Debug, PartialEq, Eq)]
struct MatchEntry {
    track: TrackRef,
    points: Range<usize>,
}

/// Where one row of the table comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MatchTableRow {
    /// The row naming a match and its extent.
    Header { match_index: usize },
    /// One matched point, by its index into its track's points.
    Point {
        match_index: usize,
        point_index: usize,
    },
}

/// The flat row sequence of one query's matches: each match contributes a
/// header row followed by one row per point it matched.
///
/// Resolves a row index to its match without walking the matches before it:
/// the table renders only the rows on screen.
struct MatchTableRows {
    matches: Vec<MatchEntry>,
    /// Row index of each match's header row, ascending.
    header_rows: Vec<usize>,
    total_rows: usize,
}

impl MatchTableRows {
    fn of_query_matches(matches: &[TrackMatches]) -> Self {
        let mut entries = Vec::new();
        let mut header_rows = Vec::new();
        let mut total_rows = 0;
        for track_matches in matches {
            for points in &track_matches.ranges {
                header_rows.push(total_rows);
                total_rows += 1 + points.len();
                entries.push(MatchEntry {
                    track: track_matches.track,
                    points: points.clone(),
                });
            }
        }
        Self {
            matches: entries,
            header_rows,
            total_rows,
        }
    }

    fn is_empty(&self) -> bool {
        self.matches.is_empty()
    }

    fn total_rows(&self) -> usize {
        self.total_rows
    }

    fn entry(&self, match_index: usize) -> Option<&MatchEntry> {
        self.matches.get(match_index)
    }

    /// The tracks the matches are on, each once, in track order.
    fn tracks(&self) -> BTreeSet<TrackRef> {
        self.matches.iter().map(|entry| entry.track).collect()
    }

    /// What the table draws at `row`.
    fn row_at(&self, row: usize) -> Option<MatchTableRow> {
        let match_index = self.header_rows.partition_point(|&start| start <= row);
        let match_index = match_index.checked_sub(1)?;
        let start = self.header_rows.get(match_index)?;
        let entry = self.matches.get(match_index)?;
        let offset = row.checked_sub(*start)?;
        match offset.checked_sub(1) {
            None => Some(MatchTableRow::Header { match_index }),
            Some(point_offset) => {
                let point_index = entry.points.start.checked_add(point_offset)?;
                (point_index < entry.points.end).then_some(MatchTableRow::Point {
                    match_index,
                    point_index,
                })
            }
        }
    }
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

/// What a row click requests from the app, applied once the table is laid out
/// and the enclosing panel's `Ui` is available again.
struct PointClick {
    point: DataPointRef,
    lat_lon: (f64, f64),
    response: egui::Response,
}

/// What every row of one table draws from.
struct RowContext<'a> {
    files: &'a [LoadedFile],
    query: &'a PanelQuery,
    formats: &'a [ColumnFormat<'a>],
    /// One entry per track the query matched, in track order.
    track_values: &'a [(TrackRef, TrackValues<'a>)],
    /// The colour of the rule beside a match's rows, absent for a query that
    /// draws no halos to match it to.
    rule_color: Option<egui::Color32>,
}

impl<'a> RowContext<'a> {
    fn values_of(&self, track: TrackRef) -> Option<&TrackValues<'a>> {
        self.track_values
            .iter()
            .find(|(candidate, _)| *candidate == track)
            .map(|(_, values)| values)
    }
}

/// One query's matches, as one table.
///
/// A value column reads the same way from the first match to the last: every
/// match shares the column layout and the header.
///
/// Hovering a match's name echoes the whole match on the map and the plot.
/// Hovering a point row rings that point. Clicking one pins it, like a point
/// row in the side panel.
pub(super) fn query_matches_ui(
    ui: &mut egui::Ui,
    ctx: &MatchTableContext<'_>,
    query_index: usize,
    query: &PanelQuery,
    stale: bool,
    highlight: &mut MapHighlight,
    requests: &mut PointClickRequests<'_>,
) {
    let rows = MatchTableRows::of_query_matches(&query.matches);
    if rows.is_empty() {
        ui.label(RichText::new("No matches").weak());
        return;
    }
    let formats: Vec<ColumnFormat<'static>> = query
        .columns
        .iter()
        .map(|column| ColumnFormat::of_metric(*column))
        .collect();
    let track_values: Vec<(TrackRef, TrackValues<'_>)> = rows
        .tracks()
        .into_iter()
        .filter_map(|track| Some((track, track_values(ctx, track)?)))
        .collect();
    let row_context = RowContext {
        files: ctx.files,
        query,
        formats: &formats,
        track_values: &track_values,
        rule_color: query
            .color
            .map(|color| gt_ui_theme::query_halo_color(color, stale)),
    };

    let column_widths: Vec<f32> = query
        .columns
        .iter()
        .zip(&formats)
        .map(|(column, format)| format.column_width(ui, &column.to_string()))
        .collect();
    let row_height = ui.text_style_height(&TextStyle::Monospace) + ROW_PADDING;
    let header_height = row_height + ui.text_style_height(&TextStyle::Small);
    let max_scroll_height =
        (row_height + ui.spacing().item_spacing.y) * ROWS_BEFORE_SCROLLING as f32;
    let mut clicked: Option<PointClick> = None;

    ui.scope(|ui| {
        // A stale run's rows reference point indices that may no longer address
        // the same data: they are shown, but answer nothing.
        if stale {
            ui.disable();
        }
        // A selectable label senses clicks and drags of its own, which would
        // take the pointer from the row it sits in.
        ui.style_mut().interaction.selectable_labels = false;

        let mut table = TableBuilder::new(ui)
            .id_salt(("query_match_table", query_index))
            .striped(true)
            .sense(egui::Sense::click())
            .auto_shrink([false, true])
            .max_scroll_height(max_scroll_height)
            .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
            .column(fixed_column(MATCH_RULE_WIDTH));
        for width in &column_widths {
            table = table.column(fixed_column(*width));
        }
        // The trailing column holds no value: it stretches the striping and the
        // hover fill across the full width of the table, and takes the spill of
        // a match's name.
        table = table.column(Column::remainder());

        table
            .header(header_height, |mut header| {
                header.col(|_| {});
                for (column, format) in query.columns.iter().zip(&formats) {
                    header.col(|ui| format.header_ui(ui, &column.to_string()));
                }
                header.col(|_| {});
            })
            .body(|body| {
                body.rows(row_height, rows.total_rows(), |mut row| {
                    if let Some(click) = row_ui(&mut row, &rows, &row_context, highlight) {
                        clicked = Some(click);
                    }
                });
            });
    });

    // Applied out here, where the panel's `Ui` places the pinned window beside
    // it.
    if let Some(click) = clicked {
        apply_point_click(
            ui,
            &click.response,
            click.point,
            click.lat_lon,
            ctx.scope,
            highlight,
            requests,
        );
    }
}

/// A column of exactly `width` whose content may overflow it.
/// [`Column::exact`] clips, which cuts a match's name off at the first column.
fn fixed_column(width: f32) -> Column {
    Column::initial(width).range(width..=width)
}

/// One row of the table, and the click it took.
fn row_ui(
    row: &mut TableRow<'_, '_>,
    rows: &MatchTableRows,
    cx: &RowContext<'_>,
    highlight: &mut MapHighlight,
) -> Option<PointClick> {
    match rows.row_at(row.index())? {
        MatchTableRow::Header { match_index } => {
            let entry = rows.entry(match_index)?;
            match_name_row_ui(row, cx, entry);
            if row.response().hovered() {
                highlight.hover_match = Some(MatchHighlight::new(entry.track, &entry.points));
                // Track focus alongside the band: the map fades the other
                // tracks and the plot dims their series, like hovering the
                // track in the side panel.
                highlight.hover = Some(HighlightScope::Track(entry.track));
            }
            None
        }
        MatchTableRow::Point {
            match_index,
            point_index,
        } => point_row_ui(row, cx, rows.entry(match_index)?, point_index, highlight),
    }
}

/// The row naming one match: its recording, track, start and point count,
/// spilling over the value columns it does not fill.
fn match_name_row_ui(row: &mut TableRow<'_, '_>, cx: &RowContext<'_>, entry: &MatchEntry) {
    // The row fills as many columns as the table declared: the name takes the
    // place of the first value column.
    debug_assert!(
        !cx.formats.is_empty(),
        "a query's columns always start with `time`"
    );
    row.set_overline(true);
    // The hover fill would paint over the name where it spills past its own
    // column: a hovered match answers on the map and the plot instead.
    row.set_hovered(false);
    row.col(|ui| paint_match_rule(ui, cx.rule_color));
    let name = match_header_text(cx.files, entry.track, &entry.points);
    row.col(|ui| {
        ui.add(Label::new(RichText::new(name).strong()).wrap_mode(TextWrapMode::Extend));
    });
    for _ in cx.formats.iter().skip(1) {
        row.col(|_| {});
    }
    row.col(|_| {});
}

/// One matched point: its value under each column the query tables, and the
/// click it took.
fn point_row_ui(
    row: &mut TableRow<'_, '_>,
    cx: &RowContext<'_>,
    entry: &MatchEntry,
    point_index: usize,
    highlight: &mut MapHighlight,
) -> Option<PointClick> {
    let values = cx.values_of(entry.track);
    let point = DataPointRef {
        track: entry.track,
        category: DataCategory::Tpv,
        point_index: PointIdx::new(point_index),
    };
    row.set_selected(highlight.sticky.is_some_and(|sticky| sticky == point));
    row.col(|ui| paint_match_rule(ui, cx.rule_color));
    for (column, format) in cx.query.columns.iter().zip(cx.formats) {
        let value = values.and_then(|values| values.value(*column, point_index));
        row.col(|ui| format.value_ui(ui, value));
    }
    row.col(|_| {});

    let response = row.response();
    if response.hovered() {
        // The ring the plot cursor draws: the row and the map then agree
        // on which point is meant.
        highlight.plot_hover_point = Some((entry.track.fi, entry.track.index, point.point_index));
    }
    if !response.clicked() && !response.double_clicked() {
        return None;
    }
    Some(PointClick {
        point,
        lat_lon: values.and_then(|values| values.lat_lon(point_index))?,
        response,
    })
}

/// The rule marking a match's rows in the colour its halos are drawn in.
fn paint_match_rule(ui: &egui::Ui, color: Option<egui::Color32>) {
    let Some(color) = color else {
        return;
    };
    // A match's rows read as one continuous rule: the rect is grown over the
    // gap to the row below.
    let cell = ui
        .max_rect()
        .expand2(egui::vec2(0.0, ui.spacing().item_spacing.y * 0.5));
    ui.painter().rect_filled(cell, 0.0, color);
}

/// One track's providers, absent for a track that is no longer loaded.
fn track_values<'a>(ctx: &MatchTableContext<'a>, track: TrackRef) -> Option<TrackValues<'a>> {
    let points = track.resolve(ctx.files)?.points.as_slice();
    let data = ctx.results.track_data(track);
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

    fn matches_of(ranges: Vec<(usize, Range<usize>)>) -> Vec<TrackMatches> {
        ranges
            .into_iter()
            .map(|(track_index, points)| TrackMatches {
                track: track(track_index),
                ranges: vec![points],
            })
            .collect()
    }

    #[test]
    fn every_match_contributes_a_header_row_and_one_row_per_point() {
        let rows = MatchTableRows::of_query_matches(&matches_of(vec![(0, 0..3), (1, 10..12)]));
        assert_eq!(rows.total_rows(), 7);
        assert_eq!(
            (0..8).map(|row| rows.row_at(row)).collect::<Vec<_>>(),
            vec![
                Some(MatchTableRow::Header { match_index: 0 }),
                Some(MatchTableRow::Point {
                    match_index: 0,
                    point_index: 0
                }),
                Some(MatchTableRow::Point {
                    match_index: 0,
                    point_index: 1
                }),
                Some(MatchTableRow::Point {
                    match_index: 0,
                    point_index: 2
                }),
                Some(MatchTableRow::Header { match_index: 1 }),
                Some(MatchTableRow::Point {
                    match_index: 1,
                    point_index: 10
                }),
                Some(MatchTableRow::Point {
                    match_index: 1,
                    point_index: 11
                }),
                None,
            ]
        );
    }

    #[test]
    fn a_track_with_several_matches_keeps_them_apart() {
        let matches = vec![TrackMatches {
            track: track(0),
            ranges: vec![0..1, 5..7],
        }];
        let rows = MatchTableRows::of_query_matches(&matches);
        assert_eq!(rows.total_rows(), 5);
        assert_eq!(
            rows.row_at(2),
            Some(MatchTableRow::Header { match_index: 1 })
        );
        assert_eq!(
            rows.row_at(3),
            Some(MatchTableRow::Point {
                match_index: 1,
                point_index: 5
            })
        );
        assert_eq!(rows.entry(1).map(|entry| entry.points.clone()), Some(5..7));
    }

    #[test]
    fn a_query_without_matches_has_no_rows() {
        let rows = MatchTableRows::of_query_matches(&[]);
        assert!(rows.is_empty());
        assert_eq!(rows.total_rows(), 0);
        assert_eq!(rows.row_at(0), None);
    }
}

//! One query's matches as a single table: a row naming each match, a row per
//! matched point, and one column header above them all.

use std::collections::{BTreeSet, HashSet};
use std::fmt::Write as _;
use std::ops::Range;

use egui::{Button, RichText, TextStyle, TextWrapMode, WidgetInfo, WidgetText, WidgetType};
use egui_extras::{Column, TableBuilder, TableRow};
use egui_phosphor::regular::CARET_DOWN as ICON_CARET_DOWN;
use egui_phosphor::regular::CARET_RIGHT as ICON_CARET_RIGHT;
use egui_phosphor::regular::FRAME_CORNERS as ICON_FRAME_CORNERS;
use gt_fmt::EN_DASH;
use gt_query::{MetricProvider as _, QueryMetric, TrackMatches};
use gt_query_run::{PanelQuery, PointsResults, SliceProvider, TrackProvider, TrackQueryData};
use gt_side_panel::widgets::{PointClickRequests, apply_point_click};
use gt_types::{DataCategory, LoadedFile, NavPoint, PointIdx, TrackRef};
use gt_ui_theme::EM_DASH;
use gt_ui_types::{
    DataPointRef, HighlightScope, MapHighlight, MapScope, MatchHighlight, MatchRevealTarget,
};

use super::column_format::ColumnFormat;
use super::match_name_text;

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

/// What one table writes back to the app: the cross-highlight its rows drive,
/// the point a click pins, the matches it folds, and the match to frame on the
/// map.
pub(super) struct MatchTableOutputs<'a, 'b> {
    pub(super) highlight: &'a mut MapHighlight,
    pub(super) requests: &'a mut PointClickRequests<'b>,
    pub(super) folds: &'a mut FoldedMatches,
    pub(super) reveal: &'a mut Option<MatchRevealTarget>,
}

/// A match's identity across reruns: the track it is on and the point it
/// starts at. A rerun of the same query over unchanged data produces the same
/// key, so the match keeps its fold state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) struct MatchKey {
    track: TrackRef,
    first_point: usize,
}

impl MatchKey {
    fn of(track: TrackRef, points: &Range<usize>) -> Self {
        Self {
            track,
            first_point: points.start,
        }
    }
}

/// The matches whose point rows are folded away. An empty set is every match
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

    /// Whether every match of `matches` is folded. Vacuously true for a query
    /// that matched nothing.
    pub(super) fn all_folded(&self, matches: &[TrackMatches]) -> bool {
        keys_of(matches).all(|key| self.contains(key))
    }

    pub(super) fn fold_all(&mut self, matches: &[TrackMatches]) {
        self.0.extend(keys_of(matches));
    }

    /// Expands every match of `matches`, leaving the fold state of the matches
    /// other queries list untouched.
    pub(super) fn expand_all(&mut self, matches: &[TrackMatches]) {
        for key in keys_of(matches) {
            self.0.remove(&key);
        }
    }
}

/// Every match of `matches`, by the key its fold state is kept under.
fn keys_of(matches: &[TrackMatches]) -> impl Iterator<Item = MatchKey> {
    matches.iter().flat_map(|track_matches| {
        track_matches
            .ranges
            .iter()
            .map(|points| MatchKey::of(track_matches.track, points))
    })
}

/// One match of a query: the track it is on, the points it covers, and whether
/// those points are folded away.
#[derive(Debug, PartialEq, Eq)]
struct MatchEntry {
    track: TrackRef,
    points: Range<usize>,
    folded: bool,
}

impl MatchEntry {
    fn key(&self) -> MatchKey {
        MatchKey::of(self.track, &self.points)
    }

    /// The index span the match covers, as the hover states it: `#150–#300`,
    /// or the bare index of a single-point match.
    fn index_span_text(&self) -> String {
        match gt_fmt::last_index_of_span(&self.points) {
            Some(last) => format!("#{}{EN_DASH}#{last}", self.points.start),
            None => format!("#{}", self.points.start),
        }
    }
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
/// header row, followed by one row per point it matched unless it is folded.
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
    fn of_query_matches(matches: &[TrackMatches], folds: &FoldedMatches) -> Self {
        let mut entries = Vec::new();
        let mut header_rows = Vec::new();
        let mut total_rows = 0;
        for track_matches in matches {
            for points in &track_matches.ranges {
                let folded = folds.contains(MatchKey::of(track_matches.track, points));
                header_rows.push(total_rows);
                total_rows += 1 + if folded { 0 } else { points.len() };
                entries.push(MatchEntry {
                    track: track_matches.track,
                    points: points.clone(),
                    folded,
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
            Some(_) if entry.folded => None,
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

/// The action a click in the table selects. Applied after the table is laid
/// out: folding a match changes the row count the table body is being laid out
/// from.
enum RowAction {
    PinPoint(PointClick),
    ToggleFold(MatchKey),
    ShowMatchOnMap(MatchRevealTarget),
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
    /// Whether the data changed after the run that produced these rows.
    stale: bool,
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
/// Hovering a match's name echoes the whole match on the map and the plot, and
/// clicking it folds the match's point rows away. Hovering a point row rings
/// that point. Clicking one pins it, like a point row in the side panel.
pub(super) fn query_matches_ui(
    ui: &mut egui::Ui,
    ctx: &MatchTableContext<'_>,
    query_index: usize,
    query: &PanelQuery,
    stale: bool,
    out: &mut MatchTableOutputs<'_, '_>,
) {
    let rows = MatchTableRows::of_query_matches(&query.matches, out.folds);
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
        stale,
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
    let mut action: Option<RowAction> = None;

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
            // The rule column does not clip: `paint_match_rule` paints into
            // the gap below its row as well.
            .column(Column::initial(MATCH_RULE_WIDTH).range(MATCH_RULE_WIDTH..=MATCH_RULE_WIDTH));
        for width in &column_widths {
            // Exact columns clip: a value wider than the width its column was
            // sized for is cut off there, and the table stays as wide as the
            // window it is in.
            table = table.column(Column::exact(*width));
        }
        // The trailing column holds no value: it stretches the striping and the
        // hover fill across the full width of the table, and is where a match's
        // name is painted from.
        table = table.column(Column::remainder());

        table
            .header(header_height, |mut header| {
                header.col(|_| {});
                for (column, format) in query.columns.iter().zip(&formats) {
                    header.col(|ui| {
                        format.header_ui(
                            ui,
                            &column.to_string(),
                            gt_query::metric_documentation(*column),
                        );
                    });
                }
                header.col(|_| {});
            })
            .body(|body| {
                body.rows(row_height, rows.total_rows(), |mut row| {
                    if let Some(taken) = row_ui(&mut row, &rows, &row_context, out.highlight) {
                        action = Some(taken);
                    }
                });
            });
    });

    // Applied out here, where the panel's `Ui` places the pinned window beside
    // it, and where folding a match no longer changes a row count being read.
    match action {
        Some(RowAction::PinPoint(click)) => apply_point_click(
            ui,
            &click.response,
            click.point,
            click.lat_lon,
            ctx.scope,
            out.highlight,
            out.requests,
        ),
        Some(RowAction::ToggleFold(key)) => out.folds.toggle(key),
        Some(RowAction::ShowMatchOnMap(target)) => *out.reveal = Some(target),
        None => {}
    }
}

/// One query's matches as tab-separated values: a header line naming every
/// column in the unit its values are in, then one line per matched point,
/// numbered by the match it belongs to.
///
/// Folded matches are written out too: the copy holds the whole run, whatever
/// the table shows.
pub(super) fn matches_as_tsv(ctx: &MatchTableContext<'_>, query: &PanelQuery) -> String {
    let formats: Vec<ColumnFormat<'static>> = query
        .columns
        .iter()
        .map(|column| ColumnFormat::of_metric(*column))
        .collect();
    let mut tsv = String::from("match\tpoint");
    for (column, format) in query.columns.iter().zip(&formats) {
        tsv.push('\t');
        tsv.push_str(&format.header_with_unit(&column.to_string()));
    }
    tsv.push('\n');

    let mut match_number = 0;
    for track_matches in &query.matches {
        let values = track_values(ctx, track_matches.track);
        for points in &track_matches.ranges {
            match_number += 1;
            for point_index in points.clone() {
                // Writing to a String cannot fail.
                write!(tsv, "{match_number}\t{point_index}").ok();
                for (column, format) in query.columns.iter().zip(&formats) {
                    tsv.push('\t');
                    if let Some(value) = values
                        .as_ref()
                        .and_then(|values| values.value(*column, point_index))
                    {
                        tsv.push_str(&format.cell_text(Some(value)));
                    }
                }
                tsv.push('\n');
            }
        }
    }
    tsv
}

/// One row of the table, and the action a click on it selects.
fn row_ui(
    row: &mut TableRow<'_, '_>,
    rows: &MatchTableRows,
    cx: &RowContext<'_>,
    highlight: &mut MapHighlight,
) -> Option<RowAction> {
    match rows.row_at(row.index())? {
        MatchTableRow::Header { match_index } => {
            let entry = rows.entry(match_index)?;
            let show_on_map = match_name_row_ui(row, cx, entry);
            let response = row.response().on_hover_text(entry.index_span_text());
            if response.hovered() {
                highlight.hover_match = Some(MatchHighlight::new(entry.track, &entry.points));
                // Track focus alongside the band: the map fades the other
                // tracks and the plot dims their series, like hovering the
                // track in the side panel.
                highlight.hover = Some(HighlightScope::Track(entry.track));
            }
            if show_on_map {
                return Some(RowAction::ShowMatchOnMap(MatchRevealTarget::OneMatch {
                    track: entry.track,
                    points: entry.points.clone(),
                }));
            }
            response
                .clicked()
                .then(|| RowAction::ToggleFold(entry.key()))
        }
        MatchTableRow::Point {
            match_index,
            point_index,
        } => point_row_ui(row, cx, rows.entry(match_index)?, point_index, highlight)
            .map(RowAction::PinPoint),
    }
}

/// The row naming one match: the fold caret, the match's name reading across
/// the value columns it leaves empty, and the button framing the map on it.
/// Answers whether that button was pressed.
fn match_name_row_ui(row: &mut TableRow<'_, '_>, cx: &RowContext<'_>, entry: &MatchEntry) -> bool {
    // The row fills as many columns as the table declared: the name is painted
    // over them from the last one.
    debug_assert!(
        !cx.formats.is_empty(),
        "a query's columns always start with `time`"
    );
    row.set_overline(true);
    // The hover fill would paint over the name where it reads past the first
    // value column: a hovered match answers on the map and the plot instead.
    row.set_hovered(false);
    row.col(|ui| paint_match_rule(ui, cx.rule_color));
    let (first_value_cell, _) = row.col(|_| {});
    for _ in cx.formats.iter().skip(1) {
        row.col(|_| {});
    }
    let caret = if entry.folded {
        ICON_CARET_RIGHT
    } else {
        ICON_CARET_DOWN
    };
    let name = format!(
        "{caret} {}",
        match_name_text(cx.files, entry.track, &entry.points)
    );
    let mut show_on_map = false;
    row.col(|ui| {
        let button = show_match_on_map_button_ui(ui, cx.stale);
        show_on_map = button.clicked();
        let span = egui::Rangef::new(first_value_cell.left(), button.rect.left());
        match_name_ui(ui, span, &name);
    });
    show_on_map
}

/// The button framing the map on this one match, at the right end of its row.
/// Disabled with the reason on hover once the data changed after the run.
fn show_match_on_map_button_ui(ui: &mut egui::Ui, stale: bool) -> egui::Response {
    // A right-to-left scope: the button is drawn at its own size at the end of
    // the row, and the cell claims only the width it covers.
    ui.scope_builder(
        egui::UiBuilder::new().layout(egui::Layout::right_to_left(egui::Align::Center)),
        |ui| {
            let response = ui.add(Button::new(ICON_FRAME_CORNERS).small());
            if stale {
                response.on_disabled_hover_text(format!(
                    "Data changed since this run {EM_DASH} run again to show this match"
                ))
            } else {
                response.on_hover_text("Show on map")
            }
        },
    )
    .inner
}

/// One match's name, painted onto the row over `span`, which reaches from the
/// left edge of the first value column to the button at the row's right end.
///
/// The name is painted and nothing is allocated for it: a widget claims the
/// width it draws, and a cell wider than its column widens the table past the
/// window, which the auto-sized window then follows. Painting it from the last
/// cell puts it over the backgrounds the cells before it fill in as they are
/// added.
fn match_name_ui(ui: &egui::Ui, span: egui::Rangef, name: &str) {
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

    let lat_lon = values.and_then(|values| values.lat_lon(point_index));
    let response = row
        .response()
        .on_hover_text(point_hover_text(point_index, lat_lon));
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
        lat_lon: lat_lon?,
        response,
    })
}

/// The hover text of a point row: which point of its track the row holds, and
/// where that point was recorded.
fn point_hover_text(point_index: usize, lat_lon: Option<(f64, f64)>) -> String {
    match lat_lon {
        Some((lat, lon)) => format!("#{point_index}\n{lat:.5}, {lon:.5}"),
        None => format!("#{point_index}"),
    }
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
        let rows = MatchTableRows::of_query_matches(
            &matches_of(vec![(0, 0..3), (1, 10..12)]),
            &FoldedMatches::default(),
        );
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
        let rows = MatchTableRows::of_query_matches(&matches, &FoldedMatches::default());
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
        let rows = MatchTableRows::of_query_matches(&[], &FoldedMatches::default());
        assert!(rows.is_empty());
        assert_eq!(rows.total_rows(), 0);
        assert_eq!(rows.row_at(0), None);
    }

    /// A folded match keeps its name row and drops its points: the match after
    /// it moves up by exactly the rows it folded away.
    #[test]
    fn a_folded_match_contributes_only_its_name_row() {
        let matches = matches_of(vec![(0, 0..3), (1, 10..12)]);
        let mut folds = FoldedMatches::default();
        folds.toggle(MatchKey::of(track(0), &(0..3)));
        let rows = MatchTableRows::of_query_matches(&matches, &folds);

        assert_eq!(rows.total_rows(), 4);
        assert_eq!(
            (0..5).map(|row| rows.row_at(row)).collect::<Vec<_>>(),
            vec![
                Some(MatchTableRow::Header { match_index: 0 }),
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
    fn folding_every_match_leaves_only_the_name_rows() {
        let matches = matches_of(vec![(0, 0..3), (1, 10..12)]);
        let mut folds = FoldedMatches::default();
        assert!(!folds.all_folded(&matches));
        folds.fold_all(&matches);
        assert!(folds.all_folded(&matches));
        assert_eq!(
            MatchTableRows::of_query_matches(&matches, &folds).total_rows(),
            2
        );

        folds.expand_all(&matches);
        assert!(!folds.all_folded(&matches));
        assert_eq!(
            MatchTableRows::of_query_matches(&matches, &folds).total_rows(),
            7
        );
    }

    /// Expanding one query's matches leaves the matches only another query
    /// lists folded: the two sections fold independently.
    #[test]
    fn expanding_one_query_leaves_the_other_queries_folds() {
        let shown = matches_of(vec![(0, 0..3)]);
        let other = matches_of(vec![(1, 10..12)]);
        let mut folds = FoldedMatches::default();
        folds.fold_all(&shown);
        folds.fold_all(&other);

        folds.expand_all(&shown);
        assert!(!folds.all_folded(&shown));
        assert!(folds.all_folded(&other));
    }

    #[test]
    fn a_match_states_the_index_span_it_covers() {
        let span = |points: Range<usize>| {
            MatchEntry {
                track: track(0),
                points,
                folded: false,
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

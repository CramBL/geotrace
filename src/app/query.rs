//! The query window (experimental): a small pipeline language for ad-hoc
//! analysis of the loaded data. Editor with syntax highlighting, run on the
//! currently visible tracks, and a results area whose matches also draw on
//! the map as halos.

use std::collections::HashMap;
use std::ops::Range;

use chrono::{DateTime, Utc};
use egui::text::LayoutJob;
use gt_analysis::loss_of_lock::{self, SECS_PER_MIN, SlipRatePerPoint};
use gt_analysis::satellite_utilization::{self, UtilPerPoint};
use gt_query::lexer::{self, TokenClass};
use gt_query::{
    CheckedQuery, Diagnostic, MetricProvider, Quantity, QueryMetric, RunOutput, Span, TrackInput,
    Unit,
};
use gt_types::satellites::Constellation;
use gt_types::{FileIdx, LoadedFile, NavPoint, TrackIdx, TrackRef};
use gt_ui_theme::{DEGREE_SIGN, EM_DASH};
use gt_ui_types::{MapHighlight, QueryMatches, TrackDataVisibility};

/// Rows shown per match table before truncating with a "more points" note.
const MATCH_TABLE_ROW_CAP: usize = 100;

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
    results: Option<QueryResults>,
}

/// Everything one run produced and the UI needs to show it.
struct QueryResults {
    matches: QueryMatches,
    summary: String,
    columns: Vec<QueryMetric>,
    /// Per-track derived series (only for metrics the query referenced),
    /// kept so match tables show the exact values the run used.
    track_data: HashMap<TrackRef, TrackQueryData>,
    /// Identities of the loaded files at run time - results gray out when
    /// this no longer matches.
    file_identities: Vec<String>,
}

/// Owned per-track inputs for [`TrackProvider`], computed once per run.
#[derive(Default)]
struct TrackQueryData {
    util: Option<UtilPerPoint>,
    slip: Option<SlipRatePerPoint>,
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
            results: None,
        }
    }

    /// Matches of the last run, for the map. `None` when there are none.
    pub fn matches(&self) -> Option<&QueryMatches> {
        self.results.as_ref().map(|r| &r.matches)
    }

    /// Replace the editor text, e.g. when loading a history entry or an
    /// example. Never runs - running stays an explicit action.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "exercised by the snapshot test; query history and examples load through it next"
        )
    )]
    pub fn set_text(&mut self, text: String) {
        self.text = text;
    }

    /// Render the window and handle runs. Call after the plot-hover
    /// forwarding: match-table row hover writes the same cross-highlight
    /// fields and must win for the frame.
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        files: &[LoadedFile],
        visibility: &TrackDataVisibility,
        highlight: &mut MapHighlight,
    ) {
        if !self.open {
            return;
        }

        // Results computed against a different set of loaded files gray out
        // (indices may no longer address the same tracks).
        if let Some(results) = &mut self.results {
            results.matches.stale = file_identities(files) != results.file_identities;
        }

        let mut open = self.open;
        egui::Window::new("Query (experimental)")
            .open(&mut open)
            .default_width(460.0)
            .show(ctx, |ui| {
                self.editor_ui(ui);
                ui.separator();
                self.results_ui(ui, files, highlight);
            });

        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
            open = false;
        }
        self.open = open;

        // The run button lives inside `editor_ui`; it sets this flag.
        if self.run_requested {
            self.run_requested = false;
            self.run(files, visibility);
        }
    }

    fn editor_ui(&mut self, ui: &mut egui::Ui) {
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
        ui.add(
            egui::TextEdit::multiline(&mut self.text)
                .code_editor()
                .desired_rows(5)
                .desired_width(f32::INFINITY)
                .hint_text("points | where velocity > 30 km/h")
                .layouter(&mut layouter),
        );

        match &self.checked {
            Err(diagnostic) if !self.text.trim().is_empty() => {
                let message = match &diagnostic.help {
                    Some(help) => format!("{}\n{help}", diagnostic.message),
                    None => diagnostic.message.clone(),
                };
                ui.label(
                    egui::RichText::new(message)
                        .color(gt_ui_theme::ERROR_INDICATOR)
                        .small(),
                );
            }
            _ => {}
        }

        let runnable = self.checked.is_ok();
        let run = ui.add_enabled(runnable, egui::Button::new("Run"));
        let run = if runnable {
            run
        } else {
            run.on_disabled_hover_text("Fix the error above to run")
        };
        if run.clicked() {
            self.run_requested = true;
        }
    }

    fn results_ui(&self, ui: &mut egui::Ui, files: &[LoadedFile], highlight: &mut MapHighlight) {
        let Some(results) = &self.results else {
            ui.label(egui::RichText::new("No runs yet").weak());
            return;
        };
        let stale = results.matches.stale;

        ui.label(&results.summary);
        if stale {
            ui.label(
                egui::RichText::new(format!("Data changed since this run {EM_DASH} run again"))
                    .weak()
                    .italics(),
            );
        }

        egui::ScrollArea::vertical()
            .max_height(300.0)
            .show(ui, |ui| {
                for (track_ref, ranges) in matches_in_order(&results.matches) {
                    for range in ranges {
                        match_ui(ui, files, results, track_ref, range, stale, highlight);
                    }
                }
            });
    }

    /// Parse/check are already done (`self.checked`); execute over the
    /// visible tracks and store everything the UI needs.
    fn run(&mut self, files: &[LoadedFile], visibility: &TrackDataVisibility) {
        let Ok(query) = &self.checked else {
            return;
        };

        let uses_util = query.referenced_metrics().iter().any(|m| m.is_util());
        let uses_slip = query.referenced_metrics().iter().any(|m| m.is_slip());
        let params = query.params();

        let mut track_data: HashMap<TrackRef, TrackQueryData> = HashMap::new();
        let mut track_refs: Vec<TrackRef> = Vec::new();
        for (fi, file) in files.iter().enumerate() {
            let fi = FileIdx::new(fi);
            for (ti, track) in file.tracks.iter().enumerate() {
                let track_ref = TrackRef::new(fi, TrackIdx::new(ti));
                if !visibility.track_enabled(track_ref) {
                    continue;
                }
                track_refs.push(track_ref);
                track_data.insert(
                    track_ref,
                    compute_track_data(&track.points, params, uses_util, uses_slip),
                );
            }
        }

        let providers: Vec<(TrackRef, TrackProvider<'_>)> = track_refs
            .iter()
            .filter_map(|&track_ref| {
                let points = points_of(files, track_ref)?;
                let data = track_data.get(&track_ref);
                Some((track_ref, provider_for(points, data)))
            })
            .collect();
        let inputs: Vec<TrackInput<'_>> = providers
            .iter()
            .map(|(track_ref, provider)| TrackInput {
                track: *track_ref,
                provider,
            })
            .collect();

        let output = gt_query::run(query, &inputs);
        let summary = summary_line(&output);
        let mut ranges: HashMap<TrackRef, Vec<Range<usize>>> = HashMap::new();
        for track_matches in &output.matches {
            ranges.insert(track_matches.track, track_matches.ranges.clone());
        }
        // Tracks without matches keep no entry, and unreferenced derived
        // series were never computed - drop the empties.
        track_data.retain(|track_ref, _| ranges.contains_key(track_ref));

        self.results = Some(QueryResults {
            matches: QueryMatches {
                ranges,
                stale: false,
            },
            summary,
            columns: output.columns,
            track_data,
            file_identities: file_identities(files),
        });
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
    let provider = provider_for(points, results.track_data.get(&track_ref));

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
                        gt_query::derived_accel(&provider, pi)
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

fn file_identities(files: &[LoadedFile]) -> Vec<String> {
    files.iter().map(|f| f.identity.clone()).collect()
}

fn compute_track_data(
    points: &[NavPoint],
    params: gt_query::Params,
    uses_util: bool,
    uses_slip: bool,
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
    TrackQueryData { util, slip }
}

fn summary_line(output: &RunOutput) -> String {
    let summary = &output.summary;
    let mut parts = vec![format!(
        "{} {} on {} {}",
        summary.match_count,
        gt_fmt::pluralize(summary.match_count, "match", "matches"),
        summary.tracks_with_matches,
        gt_fmt::pluralize(summary.tracks_with_matches, "track", "tracks"),
    )];
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
        let color = match class {
            TokenClass::Keyword => gt_ui_theme::QUERY_SYNTAX_KEYWORD,
            TokenClass::Number => gt_ui_theme::QUERY_SYNTAX_NUMBER,
            TokenClass::Ident => gt_ui_theme::QUERY_SYNTAX_IDENT,
            TokenClass::Comment => gt_ui_theme::QUERY_SYNTAX_COMMENT,
            TokenClass::Punctuation => default_color,
            TokenClass::Error => gt_ui_theme::ERROR_INDICATOR,
        };
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
        let line = summary_line(&output);
        assert_eq!(
            line,
            format!(
                "0 matches on 0 tracks {EM_DASH} 3 skipped (missing util_all) \
                 {EM_DASH} snr_drop declared but unused"
            )
        );
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

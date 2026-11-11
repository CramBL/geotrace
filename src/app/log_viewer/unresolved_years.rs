//! Year recovery controls and diagnostics in the active log's chronology inspector.

use std::ops::RangeInclusive;

use chrono::Datelike as _;
use egui::{Button, DragValue, Grid, Label, RichText};
use gt_logfile::{BoundComparison, ParsedLog, UnresolvedYear, YearResolution};
use gt_ui_types::LoadedLogId;

use super::LogViewerWindow;
use super::summary_panel::GRID_SPACING;

/// The year the user enters for the last anchored line of one log.
pub(super) struct LastEntryYearDraft {
    log: LoadedLogId,
    year: i32,
}

impl LastEntryYearDraft {
    /// Starts at the year the user applied last, or else the year of the summary block's bounds
    /// or of the year reference.
    fn of(log: LoadedLogId, parsed: &ParsedLog) -> Self {
        let year = match parsed.year_resolution() {
            YearResolution::PlaceLastEntryInYear(year) => year,
            YearResolution::IgnoreSummaryBounds | YearResolution::Infer => parsed
                .summary_block()
                .and_then(|summary| summary.logs_end_at.or(summary.logs_begin_at))
                .unwrap_or(parsed.year_reference())
                .year(),
        };
        Self { log, year }
    }
}

impl LogViewerWindow {
    /// Returns the year resolution the user applied this frame.
    pub(super) fn unresolved_years_ui(
        &mut self,
        ui: &mut egui::Ui,
        log_id: LoadedLogId,
        parsed: &ParsedLog,
        unresolved_year: &UnresolvedYear,
    ) -> Option<YearResolution> {
        let last_entry_year = self.last_entry_year_ui(ui, log_id, parsed);
        let ignore_summary_bounds = ignore_summary_bounds_ui(ui, parsed);
        ui.collapsing("Date diagnostics and help", |ui| {
            unresolved_year_ui(ui, unresolved_year);
            ui.add(Label::new(RichText::new(EARLIER_LINES_NOTE).weak()).wrap());
        });
        last_entry_year.or(ignore_summary_bounds)
    }

    fn last_entry_year_ui(
        &mut self,
        ui: &mut egui::Ui,
        log_id: LoadedLogId,
        parsed: &ParsedLog,
    ) -> Option<YearResolution> {
        let last_anchored = parsed
            .entries()
            .iter()
            .rev()
            .find(|entry| entry.is_anchored())?;
        let mut draft = self
            .log_state_mut(log_id)
            .last_entry_year_draft
            .take()
            .filter(|draft| draft.log == log_id)
            .unwrap_or_else(|| LastEntryYearDraft::of(log_id, parsed));
        let applied = ui
            .horizontal_wrapped(|ui| {
                ui.label(format!("Year of line {}", last_anchored.line_number));
                let year_field = ui.add(DragValue::new(&mut draft.year).range(CHOSEN_YEAR_RANGE));
                year_field
                    .ctx
                    .accesskit_node_builder(year_field.id, |node| {
                        node.set_author_id(LAST_ENTRY_YEAR_ID);
                    });
                ui.button(APPLY_YEAR_LABEL)
                    .on_hover_text(APPLY_YEAR_HOVER)
                    .clicked()
                    .then_some(YearResolution::PlaceLastEntryInYear(draft.year))
            })
            .inner;
        let message_end = usize::try_from(last_anchored.message.offset)
            .ok()?
            .checked_add(last_anchored.message.len as usize)?;
        let source_excerpt = parsed
            .text()
            .get(..message_end)?
            .rsplit('\n')
            .next()
            .unwrap_or_default();
        ui.add(Label::new(source_excerpt).selectable(true).truncate())
            .on_hover_text(source_excerpt);
        self.log_state_mut(log_id).last_entry_year_draft = Some(draft);
        applied
    }
}

fn unresolved_year_ui(ui: &mut egui::Ui, unresolved_year: &UnresolvedYear) {
    let amber = gt_ui_theme::warning_amber(ui.visuals().dark_mode);
    ui.add(
        Label::new(
            RichText::new(unresolved_year.headline())
                .strong()
                .color(amber),
        )
        .selectable(true)
        .wrap(),
    );
    Grid::new("log_viewer_unresolved_year")
        .num_columns(2)
        .spacing(GRID_SPACING)
        .show(ui, |ui| {
            for (caption, value) in detail_rows(unresolved_year) {
                ui.label(RichText::new(caption).weak());
                ui.add(Label::new(value).selectable(true).wrap());
                ui.end_row();
            }
        });
    ui.add(Label::new(RichText::new(LINES_TAKE_NO_POSITION_NOTE).weak()).wrap());
}

fn ignore_summary_bounds_ui(ui: &mut egui::Ui, parsed: &ParsedLog) -> Option<YearResolution> {
    let states_summary_bounds = parsed
        .summary_block()
        .is_some_and(|summary| summary.logs_begin_at.is_some() || summary.logs_end_at.is_some());
    let applies_summary_bounds = parsed.year_resolution() == YearResolution::Infer;
    ui.add_enabled(
        states_summary_bounds && applies_summary_bounds,
        Button::new(IGNORE_SUMMARY_BOUNDS_LABEL),
    )
    .on_hover_text(IGNORE_SUMMARY_BOUNDS_HOVER)
    .on_disabled_hover_text(if states_summary_bounds {
        SUMMARY_BOUNDS_IGNORED_HOVER
    } else {
        NO_SUMMARY_BOUNDS_HOVER
    })
    .clicked()
    .then_some(YearResolution::IgnoreSummaryBounds)
}

fn detail_rows(unresolved_year: &UnresolvedYear) -> Vec<(&'static str, String)> {
    let mut rows = vec![
        (
            WRITTEN_AS_CAPTION,
            unresolved_year.written_timestamp.clone(),
        ),
        (REASON_CAPTION, unresolved_year.conflict.reason()),
    ];
    if let Some(BoundComparison {
        candidate,
        bound_name,
        bound,
    }) = unresolved_year.conflict.bound_comparison()
    {
        rows.extend([
            (
                CANDIDATE_TIME_CAPTION,
                gt_fmt::format_utc_subsecond(candidate),
            ),
            (bound_name, gt_fmt::format_utc_subsecond(bound)),
        ]);
    }
    rows
}

const WRITTEN_AS_CAPTION: &str = "Written as";

const REASON_CAPTION: &str = "Reason";

const CANDIDATE_TIME_CAPTION: &str = "Candidate time";

pub(in crate::app) const LINES_TAKE_NO_POSITION_NOTE: &str = "Each line shows its timestamp as written and takes no position while the years of this log \
     are unresolved";

pub(in crate::app) const LAST_ENTRY_YEAR_ID: &str = "log-last-entry-year";

pub(in crate::app) const APPLY_YEAR_LABEL: &str = "Apply year";

const APPLY_YEAR_HOVER: &str =
    "Resolve the years of this log with its last timestamped line in this year";

const EARLIER_LINES_NOTE: &str = "The lines before it take their years from the order of the lines";

pub(in crate::app) const IGNORE_SUMMARY_BOUNDS_LABEL: &str =
    "Ignore \"Logs begin at\" and \"Logs end at\"";

const IGNORE_SUMMARY_BOUNDS_HOVER: &str =
    "Resolve the years of this log without the times its summary block states";

pub(in crate::app) const NO_SUMMARY_BOUNDS_HOVER: &str =
    "The summary block of this log states no \"Logs begin at\" or \"Logs end at\"";

const SUMMARY_BOUNDS_IGNORED_HOVER: &str =
    "The chosen years already ignore the times the summary block states";

const CHOSEN_YEAR_RANGE: RangeInclusive<i32> = 1970..=9999;

#[cfg(test)]
mod tests {
    use gt_logfile::YearConflict;
    use gt_types::fixtures;
    use rstest::rstest;

    use super::*;

    #[rstest]
    #[case::lack_date(
        YearConflict::LackDate { year: 2023 },
        vec![
            (WRITTEN_AS_CAPTION, "Feb 29 12:00:00"),
            (REASON_CAPTION, "The date does not exist in 2023"),
        ]
    )]
    #[case::follow_logs_end_at(
        YearConflict::FollowLogsEndAt {
            candidate: fixtures::utc_instant(2027, 5, 20, 12, 0),
            logs_end_at: fixtures::utc_instant(2026, 5, 20, 12, 0),
        },
        vec![
            (WRITTEN_AS_CAPTION, "Feb 29 12:00:00"),
            (REASON_CAPTION, "After the summary block's \"Logs end at\""),
            (CANDIDATE_TIME_CAPTION, "2027-05-20 12:00:00 UTC"),
            ("Logs end at", "2026-05-20 12:00:00 UTC"),
        ]
    )]
    #[case::follow_year_reference(
        YearConflict::FollowYearReference {
            candidate: fixtures::utc_instant(2027, 5, 20, 12, 0),
            year_reference: fixtures::utc_instant(2026, 5, 20, 12, 0),
        },
        vec![
            (WRITTEN_AS_CAPTION, "Feb 29 12:00:00"),
            (REASON_CAPTION, "More than 1h after the reference date"),
            (CANDIDATE_TIME_CAPTION, "2027-05-20 12:00:00 UTC"),
            ("Reference date", "2026-05-20 12:00:00 UTC"),
        ]
    )]
    #[case::precede_logs_begin_at(
        YearConflict::PrecedeLogsBeginAt {
            candidate: fixtures::utc_instant(2027, 5, 20, 12, 0),
            logs_begin_at: fixtures::utc_instant(2026, 5, 20, 12, 0),
        },
        vec![
            (WRITTEN_AS_CAPTION, "Feb 29 12:00:00"),
            (REASON_CAPTION, "Before the summary block's \"Logs begin at\""),
            (CANDIDATE_TIME_CAPTION, "2027-05-20 12:00:00 UTC"),
            ("Logs begin at", "2026-05-20 12:00:00 UTC"),
        ]
    )]
    fn the_rows_state_each_field_of_the_conflict(
        #[case] conflict: YearConflict,
        #[case] expected: Vec<(&str, &str)>,
    ) {
        let rows = detail_rows(&UnresolvedYear {
            line_number: 4,
            written_timestamp: "Feb 29 12:00:00".to_owned(),
            conflict,
        });

        assert_eq!(
            rows.iter()
                .map(|(caption, value)| (*caption, value.as_str()))
                .collect::<Vec<_>>(),
            expected
        );
    }
}

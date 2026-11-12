use std::ops::RangeInclusive;

use chrono::Datelike as _;
use egui::{Button, ComboBox, DragValue, Grid, Label, RichText, WidgetInfo, WidgetType};
use egui_phosphor::regular::ARROW_COUNTER_CLOCKWISE as ICON_RESET;
use gt_log_view::LoadedLog;
use gt_logfile::{BoundComparison, LogChronology, ParsedLog, UnresolvedYear, YearResolution};
use gt_ui_types::LoadedLogId;

use super::LogViewerWindow;
use super::line_table::SourceRequest;
use super::summary_panel::GRID_SPACING;

pub(super) struct ChronologyDraft {
    log: LoadedLogId,
    policy: ChronologyPolicyDraft,
}

impl ChronologyDraft {
    fn of(log: LoadedLogId, parsed: &ParsedLog) -> Self {
        let policy = match parsed.year_resolution() {
            YearResolution::Infer => ChronologyPolicyDraft::UseAutomatic,
            YearResolution::IgnoreSummaryBounds => ChronologyPolicyDraft::IgnoreExportRange,
            YearResolution::PlaceLastEntryInYear(year) => ChronologyPolicyDraft::SetLastEntryYear {
                year,
                origin: YearSuggestionOrigin::UseCurrentPolicy,
            },
        };
        Self { log, policy }
    }

    fn actions_ui(&mut self, ui: &mut egui::Ui, parsed: &ParsedLog) -> Option<YearResolution> {
        ui.horizontal_wrapped(|ui| {
            ui.label("Export range");
            ui.strong(if parsed.format().writes_years() {
                "Inapplicable"
            } else if parsed.summary_block().is_some_and(|summary| {
                summary.logs_begin_at.is_some() || summary.logs_end_at.is_some()
            }) {
                match self.policy {
                    ChronologyPolicyDraft::UseAutomatic => "Applied",
                    _ => "Ignored",
                }
            } else {
                "Absent"
            });
            let apply = ui
                .add_enabled(
                    self.policy.resolution() != parsed.year_resolution()
                        && (self.policy == ChronologyPolicyDraft::UseAutomatic
                            || !parsed.format().writes_years()),
                    Button::new(APPLY_YEAR_LABEL),
                )
                .on_hover_text("Apply this date policy to the original written timestamps")
                .on_disabled_hover_text(
                    if parsed.format().writes_years()
                        && self.policy != ChronologyPolicyDraft::UseAutomatic
                    {
                        EXPLICIT_YEARS_HOVER
                    } else {
                        "This date policy is already applied"
                    },
                )
                .clicked();
            let reset = ui
                .add_enabled(
                    self.policy.resolution() != YearResolution::Infer
                        || parsed.year_resolution() != YearResolution::Infer,
                    Button::new(ICON_RESET),
                )
                .on_hover_text(RESET_LABEL)
                .on_disabled_hover_text("Automatic dates are selected and applied");
            reset.widget_info(|| {
                WidgetInfo::labeled(WidgetType::Button, reset.enabled(), RESET_LABEL)
            });
            if reset.clicked() {
                self.policy = ChronologyPolicyDraft::UseAutomatic;
                (parsed.year_resolution() != YearResolution::Infer).then_some(YearResolution::Infer)
            } else {
                apply.then(|| self.policy.resolution())
            }
        })
        .inner
    }

    fn anchor_line(&self, parsed: &ParsedLog) -> Option<u32> {
        match (self.policy, parsed.chronology()) {
            (ChronologyPolicyDraft::SetLastEntryYear { .. }, _)
            | (_, LogChronology::Resolved(_)) => parsed
                .entries()
                .iter()
                .rev()
                .find(|entry| entry.is_anchored())
                .map(|entry| entry.line_number),
            (_, LogChronology::Unresolved(unresolved)) => {
                Some(unresolved.unresolved_year().line_number)
            }
        }
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum ChronologyPolicyDraft {
    IgnoreExportRange,
    SetLastEntryYear {
        year: i32,
        origin: YearSuggestionOrigin,
    },
    UseAutomatic,
}

impl ChronologyPolicyDraft {
    fn suggested_year(parsed: &ParsedLog) -> Self {
        let (year, origin) = match parsed.year_resolution() {
            YearResolution::PlaceLastEntryInYear(year) => {
                (year, YearSuggestionOrigin::UseCurrentPolicy)
            }
            _ => match parsed
                .summary_block()
                .and_then(|summary| summary.logs_end_at.or(summary.logs_begin_at))
            {
                Some(bound) => (bound.year(), YearSuggestionOrigin::UseExportRange),
                None => (
                    parsed.year_reference().year(),
                    YearSuggestionOrigin::UseReferenceDate,
                ),
            },
        };
        Self::SetLastEntryYear {
            year: year.clamp(*CHOSEN_YEAR_RANGE.start(), *CHOSEN_YEAR_RANGE.end()),
            origin,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::IgnoreExportRange => IGNORE_SUMMARY_BOUNDS_LABEL,
            Self::SetLastEntryYear { .. } => SET_YEAR_LABEL,
            Self::UseAutomatic => AUTOMATIC_LABEL,
        }
    }

    fn resolution(self) -> YearResolution {
        match self {
            Self::IgnoreExportRange => YearResolution::IgnoreSummaryBounds,
            Self::SetLastEntryYear { year, .. } => YearResolution::PlaceLastEntryInYear(year),
            Self::UseAutomatic => YearResolution::Infer,
        }
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum YearSuggestionOrigin {
    EnterManually,
    UseCurrentPolicy,
    UseExportRange,
    UseReferenceDate,
}

impl YearSuggestionOrigin {
    fn label(self) -> &'static str {
        match self {
            Self::EnterManually => "Edited",
            Self::UseCurrentPolicy => "Current policy",
            Self::UseExportRange => "Export range",
            Self::UseReferenceDate => "Reference date",
        }
    }
}

impl LogViewerWindow {
    pub(in crate::app) fn record_chronology_apply_result(
        &mut self,
        log: LoadedLogId,
        applied: bool,
    ) {
        self.log_state_mut(log).chronology_apply_failed = !applied;
    }

    pub(super) fn chronology_editor_ui(
        &mut self,
        ui: &mut egui::Ui,
        log_id: LoadedLogId,
        log: &LoadedLog,
    ) -> Option<YearResolution> {
        let parsed = log.parsed();
        self.chronology_result_ui(ui, log_id, parsed);
        let mut draft = self
            .log_state_mut(log_id)
            .chronology_draft
            .take()
            .filter(|draft| draft.log == log_id)
            .unwrap_or_else(|| ChronologyDraft::of(log_id, parsed));
        let previous = draft.policy;
        draft.policy.selector_ui(ui, parsed);
        let anchor = draft.anchor_line(parsed);
        ui.group(|ui| {
            if let Some(line) = anchor {
                self.chronology_source_ui(ui, log, log_id, line);
            }
            draft.policy.year_ui(ui, parsed);
        });
        let requested = draft.actions_ui(ui, parsed);
        if draft.policy != previous {
            self.log_state_mut(log_id).chronology_apply_failed = false;
        }
        self.log_state_mut(log_id).chronology_draft = Some(draft);
        self.chronology_details_ui(ui, log, log_id);
        requested
    }

    fn chronology_details_ui(&mut self, ui: &mut egui::Ui, log: &LoadedLog, log_id: LoadedLogId) {
        let parsed = log.parsed();
        ui.collapsing(DETAILS_LABEL, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label("Reference date");
                ui.add(Label::new(gt_fmt::format_utc_subsecond(parsed.year_reference())).selectable(true));
            });
            if let LogChronology::Unresolved(unresolved) = parsed.chronology() {
                let conflict = unresolved.unresolved_year();
                self.source_link_ui(ui, log, log_id, SourceRequest::NavigateToLine(conflict.line_number), Button::new(conflict.headline()).wrap());
                unresolved_year_ui(ui, conflict);
            }
            ui.add(Label::new(if parsed.format().writes_years() {
                "These timestamps include years. Automatic preserves their written dates. Reset removes an existing year policy."
            } else {
                "Set year assigns the last timestamped entry's year and derives earlier years from source order. Set year and Ignore export range ignore both exporter date bounds."
            }).wrap());
        });
    }

    fn chronology_result_ui(&mut self, ui: &mut egui::Ui, id: LoadedLogId, parsed: &ParsedLog) {
        ui.horizontal_wrapped(|ui| {
            ui.strong(if parsed.chronology().is_resolved() {
                "Dates resolved"
            } else {
                "Dates unresolved"
            });
            if let Some(range) = parsed.chronology().time_range() {
                ui.add(
                    Label::new(gt_fmt::format_time_range(range.start, range.end)).selectable(true),
                );
            }
        });
        if self.log_state_mut(id).chronology_apply_failed {
            ui.colored_label(
                gt_ui_theme::warning_amber(ui.visuals().dark_mode),
                "Date policy could not be applied",
            );
        } else if let LogChronology::Unresolved(unresolved) = parsed.chronology() {
            ui.add(Label::new(unresolved.unresolved_year().conflict.reason()).wrap());
        }
    }

    fn chronology_source_ui(
        &mut self,
        ui: &mut egui::Ui,
        log: &LoadedLog,
        id: LoadedLogId,
        line: u32,
    ) {
        let parsed = log.parsed();
        let entry = parsed
            .entries()
            .binary_search_by_key(&line, |entry| entry.line_number)
            .ok();
        ui.horizontal_wrapped(|ui| {
            self.source_link_ui(
                ui,
                log,
                id,
                SourceRequest::NavigateToLine(line),
                Button::new(format!("Line {line}")),
            );
            if let Some(written) = entry.and_then(|entry| parsed.written_timestamp(entry)) {
                ui.add(Label::new(RichText::new(written).monospace()).selectable(true));
            }
        });
        if let Some(source) = parsed.source_line(line) {
            ui.add(Label::new(source).selectable(true).truncate())
                .on_hover_text(source);
        }
    }
}

impl ChronologyPolicyDraft {
    fn selector_ui(&mut self, ui: &mut egui::Ui, parsed: &ParsedLog) {
        let response = ComboBox::from_id_salt("chronology_policy")
            .selected_text(self.label())
            .show_ui(ui, |ui| {
                ui.selectable_value(self, Self::UseAutomatic, AUTOMATIC_LABEL)
                    .on_hover_text(if parsed.format().writes_years() {
                        "Preserve the original written dates"
                    } else {
                        "Infer dates with the original reference date and exporter bounds"
                    });
                let year = match *self {
                    Self::SetLastEntryYear { .. } => *self,
                    _ => Self::suggested_year(parsed),
                };
                ui.add_enabled_ui(!parsed.format().writes_years(), |ui| {
                    ui.selectable_value(self, year, SET_YEAR_LABEL)
                        .on_hover_text("Choose the year of the last timestamped entry")
                        .on_disabled_hover_text(EXPLICIT_YEARS_HOVER);
                });
                ui.add_enabled_ui(
                    !parsed.format().writes_years()
                        && parsed.summary_block().is_some_and(|summary| {
                            summary.logs_begin_at.is_some() || summary.logs_end_at.is_some()
                        }),
                    |ui| {
                        ui.selectable_value(
                            self,
                            Self::IgnoreExportRange,
                            IGNORE_SUMMARY_BOUNDS_LABEL,
                        )
                        .on_hover_text("Infer dates without exporter date bounds")
                        .on_disabled_hover_text(
                            if parsed.format().writes_years() {
                                EXPLICIT_YEARS_HOVER
                            } else {
                                NO_SUMMARY_BOUNDS_HOVER
                            },
                        );
                    },
                );
            })
            .response
            .on_hover_text("Choose a date policy");
        response.ctx.accesskit_node_builder(response.id, |node| {
            node.set_author_id(POLICY_ID);
            node.set_label("Date policy");
        });
    }

    fn year_ui(&mut self, ui: &mut egui::Ui, parsed: &ParsedLog) {
        if let Self::SetLastEntryYear { year, origin } = self {
            ui.horizontal_wrapped(|ui| {
                let label = if parsed.format().writes_years() {
                    "Year policy"
                } else {
                    "Last entry year"
                };
                ui.label(label);
                let field = ui
                    .add_enabled(
                        !parsed.format().writes_years(),
                        DragValue::new(year).range(CHOSEN_YEAR_RANGE),
                    )
                    .on_disabled_hover_text(EXPLICIT_YEARS_HOVER);
                field.ctx.accesskit_node_builder(field.id, |node| {
                    node.set_author_id(LAST_ENTRY_YEAR_ID);
                    node.set_label(label);
                });
                if field.changed() {
                    *origin = YearSuggestionOrigin::EnterManually;
                }
                ui.label(RichText::new(origin.label()).small().weak())
                    .on_hover_text("Origin of the suggested year");
            });
        }
    }
}

fn unresolved_year_ui(ui: &mut egui::Ui, unresolved_year: &UnresolvedYear) {
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
pub(in crate::app) const LINES_TAKE_NO_POSITION_NOTE: &str = "Each line shows its timestamp as written and takes no position while the years of this log are unresolved";
pub(in crate::app) const LAST_ENTRY_YEAR_ID: &str = "log-last-entry-year";
pub(in crate::app) const APPLY_YEAR_LABEL: &str = "Apply";
pub(in crate::app) const IGNORE_SUMMARY_BOUNDS_LABEL: &str = "Ignore export range";
pub(in crate::app) const NO_SUMMARY_BOUNDS_HOVER: &str = "This log has no exporter date bounds";
pub(in crate::app) const SET_YEAR_LABEL: &str = "Set year";
pub(in crate::app) const AUTOMATIC_LABEL: &str = "Automatic";
pub(in crate::app) const RESET_LABEL: &str = "Reset dates to automatic";
pub(in crate::app) const POLICY_ID: &str = "log-chronology-policy";
pub(in crate::app) const DETAILS_LABEL: &str = "Details";
pub(in crate::app) const EXPLICIT_YEARS_HOVER: &str = "These timestamps already include years";
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

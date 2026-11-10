//! The block the viewer shows above the summary panel of a log whose years are unresolved: the
//! line at which year resolution stopped, its written timestamp, and the conflict.

use egui::{Grid, Label, RichText};
use gt_logfile::{BoundComparison, UnresolvedYear};

use super::summary_panel::GRID_SPACING;

pub(super) fn unresolved_years_ui(ui: &mut egui::Ui, unresolved_year: &UnresolvedYear) {
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

use std::fs;
use std::path::Path;

use super::*;
use crate::app::load_failures::LoadProblemRequest;
use crate::app::loader::GtdLoadMode;

#[derive(Clone, Copy)]
enum FileSelectionOutcome {
    Cancel,
    Disconnect,
    Dismiss,
    Load,
    LoadRecording,
    Supersede,
}

#[rstest]
fn replacement_file_dialogs_preserve_their_owner_beside_a_normal_open(
    #[values(
        FileSelectionOutcome::Cancel,
        FileSelectionOutcome::Disconnect,
        FileSelectionOutcome::Dismiss,
        FileSelectionOutcome::Load,
        FileSelectionOutcome::LoadRecording,
        FileSelectionOutcome::Supersede
    )]
    outcome: FileSelectionOutcome,
) {
    let mut harness = harness(false);
    let dir = tempfile::tempdir().unwrap();
    let replacement = dir
        .path()
        .join(if matches!(outcome, FileSelectionOutcome::LoadRecording) {
            "replacement.gtd"
        } else {
            "replacement.log"
        });
    let normal = dir.path().join("normal.log");
    for path in [&replacement, &normal] {
        let bytes =
            if path == &replacement && matches!(outcome, FileSelectionOutcome::LoadRecording) {
                ui_tests::minimal_gtd_bytes()
            } else {
                format!(
                    "{} {}\n",
                    ui_tests::base_time().format(gt_fmt::UTC_SECOND_FORMAT),
                    path.display()
                )
                .into_bytes()
            };
        fs::write(path, bytes).unwrap();
    }
    let id = harness.state_mut().load_failures.push(LoadFailure::new(
        LoadSource::Filename("original-label.log".into()),
        LoadFailureCause::ParseLog(LogParseError::Empty),
    ));
    let attempt = harness.state_mut().load_failures.begin_input(id).unwrap();
    let picker = harness
        .state_mut()
        .loader
        .controlled_file_dialog_for_test(Some(attempt));
    let normal_picker = harness
        .state_mut()
        .loader
        .controlled_file_dialog_for_test(None);
    let next = if matches!(outcome, FileSelectionOutcome::Supersede) {
        harness.state_mut().load_failures.cancel_input(attempt);
        Some(harness.state_mut().load_failures.begin_input(id).unwrap())
    } else {
        None
    };
    if matches!(outcome, FileSelectionOutcome::Dismiss) {
        harness.state_mut().load_failures.dismiss(id);
    }
    match outcome {
        FileSelectionOutcome::Cancel => picker.send(None).unwrap(),
        FileSelectionOutcome::Disconnect => drop(picker),
        _ => picker
            .send(Some((replacement.clone(), GtdLoadMode::Regular)))
            .unwrap(),
    }
    normal_picker
        .send(Some((normal, GtdLoadMode::Regular)))
        .unwrap();
    let expected = if matches!(outcome, FileSelectionOutcome::Load) {
        2
    } else {
        1
    };
    assert!(
        harness.step_until(|harness| harness.state().logs.len() == expected
            && harness.state().loader.loading_jobs.is_empty())
    );
    let selected = harness.state().log_viewer.selected_log().unwrap();
    assert_eq!(
        harness.state().logs.get_by_id(selected).unwrap().name(),
        "normal.log"
    );
    match outcome {
        FileSelectionOutcome::Cancel | FileSelectionOutcome::Disconnect => {
            assert!(harness.state_mut().load_failures.begin_input(id).is_some())
        }
        FileSelectionOutcome::Dismiss => {
            assert!(harness.state_mut().load_failures.begin_input(id).is_none())
        }
        FileSelectionOutcome::Load | FileSelectionOutcome::LoadRecording => {
            harness.get_by_role_and_label(
                Role::Window,
                format!("Recovered {}", replacement.display()).as_str(),
            );
            harness.get_by_label("Original source original-label.log");
            assert!(harness.state_mut().load_failures.begin_input(id).is_none());
            assert_eq!(harness.state().load_failures.iter().count(), 0);
            assert_eq!(
                harness.state().shared.borrow().loaded_files.len(),
                usize::from(matches!(outcome, FileSelectionOutcome::LoadRecording))
            );
        }
        FileSelectionOutcome::Supersede => {
            harness
                .state_mut()
                .apply_recovery_file_selection(attempt, None);
            assert!(
                harness
                    .state_mut()
                    .load_failures
                    .accept_file_input(next.unwrap(), Path::new("next.log"))
            );
        }
    }
}

#[derive(Clone, Copy)]
enum TextInputOutcome {
    Cancel,
    Dismiss,
    DismissLoading,
    Fail,
    Load,
}

#[rstest]
fn replacement_text_stays_in_its_problem_and_preserves_the_active_reading_context(
    #[values(
        TextInputOutcome::Cancel,
        TextInputOutcome::Dismiss,
        TextInputOutcome::DismissLoading,
        TextInputOutcome::Fail,
        TextInputOutcome::Load
    )]
    outcome: TextInputOutcome,
) {
    let mut harness = harness(false);
    harness.state_mut().load_parsed_log(
        Some("reading.log".into()),
        parsed_log("reading anchor"),
        None,
    );
    harness.run_steps(3);
    let active = harness.state().log_viewer.selected_log();
    let reading_y = harness
        .get(By::new().role(Role::Label).value("reading anchor"))
        .rect()
        .top();
    let source = LoadSource::Filename("original-text.log".into());
    let id = harness.state_mut().load_failures.push(
        LoadFailure::new(source, LoadFailureCause::ParseLog(LogParseError::Empty))
            .with_decoded_text("retained invalid text".to_owned().into()),
    );
    let attempt = harness.state_mut().load_failures.begin_input(id).unwrap();
    harness.run_steps(2);
    let text = if matches!(outcome, TextInputOutcome::Fail) {
        "edited invalid text".to_owned()
    } else {
        format!(
            "{} recovered source\n",
            ui_tests::base_time().format(gt_fmt::UTC_SECOND_FORMAT)
        )
    };
    let expected_text = text.clone();
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    harness.run_steps(2);
    harness.input_mut().events.push(egui::Event::Paste(text));
    harness.run_steps(2);
    assert!(
        harness
            .query(
                By::new()
                    .role(Role::MultilineTextInput)
                    .value(&expected_text)
            )
            .is_some()
    );
    match outcome {
        TextInputOutcome::Cancel => {
            harness.get_by_label("Cancel").click();
            harness.step();
        }
        TextInputOutcome::Dismiss => {
            harness.get_by_label("Dismiss").click();
            harness.step();
        }
        TextInputOutcome::DismissLoading | TextInputOutcome::Fail | TextInputOutcome::Load => {
            harness.get_by_label("Load text").click();
            harness.step();
        }
    }
    if matches!(outcome, TextInputOutcome::DismissLoading) {
        harness.state_mut().load_failures.dismiss(id);
        assert!(harness.step_until(|harness| harness.state().loader.loading_jobs.is_empty()));
    }
    if matches!(
        outcome,
        TextInputOutcome::Cancel | TextInputOutcome::Dismiss | TextInputOutcome::DismissLoading
    ) {
        assert_eq!(
            harness.state_mut().load_failures.begin_input(id).is_some(),
            matches!(outcome, TextInputOutcome::Cancel)
        );
        harness
            .state_mut()
            .apply_load_problem_request(LoadProblemRequest::LoadText(attempt));
        assert!(harness.state().loader.loading_jobs.is_empty());
        assert_eq!(harness.state().logs.len(), 1);
    } else {
        assert!(
            harness.step_until(|harness| !harness.state().load_failures.retry_is_pending(attempt))
        );
        if matches!(outcome, TextInputOutcome::Load) {
            assert_eq!(
                harness.state().logs.len(),
                2,
                "{:?}",
                harness
                    .state()
                    .load_failures
                    .iter()
                    .map(|failure| &failure.cause)
                    .collect::<Vec<_>>()
            );
            harness.get_by_role_and_label(Role::Window, "Recovered pasted log text");
            harness.get_by_label("Original source original-text.log");
        } else {
            assert_eq!(harness.state().logs.len(), 1);
            harness.get_by_role_and_label(Role::Window, "Failed to load original-text.log");
            harness.get_by_label_contains("Copy details").click();
            harness.step();
            let copied = harness.copied_text().unwrap();
            assert!(copied.contains("Original source: original-text.log"));
            assert!(copied.contains("edited invalid text"));
        }
    }
    assert_eq!(harness.state().log_viewer.selected_log(), active);
    assert_eq!(
        harness
            .get(By::new().role(Role::Label).value("reading anchor"))
            .rect()
            .top(),
        reading_y
    );
}

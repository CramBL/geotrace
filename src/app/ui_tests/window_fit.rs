use std::path::PathBuf;

use egui_kittest::Harness;
use gt_test_utils::{ControlLabel, DEMO_BYTES, TestHarness, WindowFitAssertions as _};
use rstest::rstest;

use crate::app::App;
use crate::app::frame::LOADING_OVERLAY_WINDOW_ID;
use crate::app::history_open::{
    AUTO_PRUNE_TITLE, HISTORY_DATABASE_CORRUPTED_TITLE, HISTORY_DATABASE_IN_USE_TITLE,
    HISTORY_DATABASE_LOCKED_TITLE, TRACK_SETTINGS_DIFFER_TITLE,
};
use crate::app::query;
use crate::app::settings_ui;
use crate::app::test_util;
use crate::app::test_util::harness::TestDroppedFile;
use crate::app::ui_tests;

/// A window the app owns, opened with content far larger than any of the audit
/// viewports.
#[derive(Debug, Clone, Copy)]
enum OversizedAppWindow {
    HistoryDatabaseInUse,
    HistoryDatabaseLocked,
    HistoryDatabaseCorrupted,
    TrackSettingsDiffer,
    AutoPrune,
    Settings,
    Query,
    TrackData,
    LoadingProgress,
}

impl OversizedAppWindow {
    fn audited(self) -> gt_test_utils::AuditedWindow<'static> {
        match self {
            Self::HistoryDatabaseInUse => {
                gt_test_utils::AuditedWindow::titled(HISTORY_DATABASE_IN_USE_TITLE)
            }
            Self::HistoryDatabaseLocked => {
                gt_test_utils::AuditedWindow::titled(HISTORY_DATABASE_LOCKED_TITLE)
            }
            Self::HistoryDatabaseCorrupted => {
                gt_test_utils::AuditedWindow::titled(HISTORY_DATABASE_CORRUPTED_TITLE)
            }
            Self::TrackSettingsDiffer => {
                gt_test_utils::AuditedWindow::titled(TRACK_SETTINGS_DIFFER_TITLE)
            }
            Self::AutoPrune => gt_test_utils::AuditedWindow::titled(AUTO_PRUNE_TITLE),
            Self::Settings => gt_test_utils::AuditedWindow::identified(
                "Settings",
                egui::Id::new(settings_ui::WINDOW_ID),
            ),
            Self::Query => gt_test_utils::AuditedWindow::titled("Query"),
            Self::TrackData => gt_test_utils::AuditedWindow::identified(
                "Track data",
                egui::Id::new("detached_panel"),
            ),
            Self::LoadingProgress => {
                gt_test_utils::AuditedWindow::titled(LOADING_OVERLAY_WINDOW_ID)
            }
        }
    }

    /// The control the user must still be able to reach. The windows a user
    /// only reads have none of their own.
    fn reachable_control(self) -> Option<&'static str> {
        match self {
            Self::HistoryDatabaseInUse => Some("Try again"),
            Self::HistoryDatabaseLocked
            | Self::HistoryDatabaseCorrupted
            | Self::TrackSettingsDiffer
            | Self::AutoPrune => Some("Cancel"),
            Self::Settings | Self::Query | Self::TrackData | Self::LoadingProgress => None,
        }
    }

    fn open_on(self, app: &mut App) {
        let long = gt_test_utils::oversized_text('a');
        match self {
            Self::HistoryDatabaseInUse => {
                app.history_failure = Some(crate::app::storage::HistoryFailure::Busy(
                    PathBuf::from(long),
                ));
            }
            Self::HistoryDatabaseLocked => {
                app.history_failure = Some(crate::app::storage::HistoryFailure::Locked(
                    PathBuf::from(long),
                ));
            }
            Self::HistoryDatabaseCorrupted => {
                app.history_failure = Some(crate::app::storage::HistoryFailure::Unreadable(
                    PathBuf::from(long),
                ));
            }
            Self::TrackSettingsDiffer => {
                app.pending_resegment = Some(crate::app::ResegmentPrompt {
                    db_ref: gt_store::DatabaseRef {
                        identity: long.clone(),
                        group_name: long.clone(),
                    },
                    filename: long,
                    bytes: std::sync::Arc::from(Vec::<u8>::new()),
                    stored: gt_store::StoredSegmentation {
                        track_split_gap_us: 60_000_000,
                        track_split_rule: gt_store::StoredTrackSplitRule::StepInEitherDirection,
                        fix_placement_rule:
                            gt_store::StoredFixPlacementRule::MissingHeadingAndNothingInFix,
                        detect_clock_discontinuities: false,
                        clock_discontinuity_sigmas: 4.0,
                    },
                    stored_tracks: Vec::new(),
                    marker_settings_changed: false,
                });
            }
            Self::AutoPrune => {
                app.pending_auto_prune = Some(
                    (0..gt_test_utils::window_fit::OVERSIZED_ROW_COUNT)
                        .map(|index| gt_store::DatabaseRef {
                            identity: format!("{long}/{index}"),
                            group_name: long.clone(),
                        })
                        .collect(),
                );
            }
            Self::Settings => app.settings_open = true,
            Self::Query => app.query_window.open = true,
            Self::TrackData => app.shared.borrow_mut().tree.detached = true,
            Self::LoadingProgress => {
                app.loader.loading_jobs = (0..gt_test_utils::window_fit::OVERSIZED_ROW_COUNT)
                    .map(|index| crate::app::loader::LoadingJob {
                        id: index as u64,
                        filename: format!("{long}/{index}"),
                        progress: 0.5,
                        stage: "reading",
                        started_at: 0.0,
                    })
                    .collect();
            }
        }
    }
}

/// Every window the app owns stays inside the screen and keeps its action
/// reachable, however much the state behind it holds.
#[rstest]
fn every_app_window_fits_the_audit_viewports(
    #[values(
        OversizedAppWindow::HistoryDatabaseInUse,
        OversizedAppWindow::HistoryDatabaseLocked,
        OversizedAppWindow::HistoryDatabaseCorrupted,
        OversizedAppWindow::TrackSettingsDiffer,
        OversizedAppWindow::AutoPrune,
        OversizedAppWindow::Settings,
        OversizedAppWindow::Query,
        OversizedAppWindow::TrackData,
        OversizedAppWindow::LoadingProgress
    )]
    window: OversizedAppWindow,
    #[values(
        gt_test_utils::window_fit::CRAMPED_VIEWPORT,
        gt_test_utils::window_fit::NARROW_VIEWPORT,
        gt_test_utils::window_fit::SHORT_VIEWPORT
    )]
    viewport: egui::Vec2,
) {
    let (mut harness, _config_path) = TestHarness::builder()
        .size(viewport)
        .eframe(test_util::harness::build_app);
    harness.inner.step();
    window.open_on(harness.inner.state_mut());
    harness.inner.run_steps(8);

    harness
        .inner
        .assert_window_fits_the_viewport(window.audited());
    if let Some(control) = window.reachable_control() {
        harness
            .inner
            .assert_control_is_reachable(window.audited(), ControlLabel(control));
    }
}

/// The self-update prompt stays inside the screen and keeps its dismissals
/// reachable, however long the offered version reads.
#[cfg(feature = "self-update")]
#[rstest]
fn the_update_prompt_fits_the_audit_viewports(
    #[values(
        gt_test_utils::window_fit::CRAMPED_VIEWPORT,
        gt_test_utils::window_fit::NARROW_VIEWPORT,
        gt_test_utils::window_fit::SHORT_VIEWPORT
    )]
    viewport: egui::Vec2,
) {
    let window = gt_test_utils::AuditedWindow::titled(crate::app::update::UPDATE_DIALOG_TITLE);
    let (mut harness, _config_path) = TestHarness::builder()
        .size(viewport)
        .eframe(test_util::harness::build_app);
    harness.inner.step();
    harness.inner.state_mut().update_checker =
        crate::app::update::UpdateChecker::available_for_test(
            &gt_test_utils::oversized_text('a'),
            true,
        );
    harness.inner.run_steps(8);

    harness.inner.assert_window_fits_the_viewport(window);
    harness
        .inner
        .assert_control_is_reachable(window, ControlLabel("Skip this version"));
}

/// The popped-out match list stays inside the screen at any viewport: its rows
/// scroll inside it.
#[rstest]
fn the_match_list_window_fits_the_audit_viewports(
    #[values(
        gt_test_utils::window_fit::CRAMPED_VIEWPORT,
        gt_test_utils::window_fit::NARROW_VIEWPORT,
        gt_test_utils::window_fit::SHORT_VIEWPORT
    )]
    viewport: egui::Vec2,
) {
    let mut harness = Harness::builder()
        .with_size(viewport)
        .with_wait_for_pending_images(false)
        .build_eframe(test_util::harness::transient_app);
    test_util::harness::drop_file_and_wait_for_load(
        &mut harness,
        TestDroppedFile::bytes(DEMO_BYTES, "demo_trip.gtd"),
    );
    harness.state_mut().query_window.open = true;
    harness.run_steps(3);
    ui_tests::run_query(&mut harness, ui_tests::MANY_MATCH_QUERY);
    ui_tests::pop_out_button(&harness).click();
    harness.run_steps(8);

    harness.assert_window_fits_the_viewport(gt_test_utils::AuditedWindow::titled(
        query::results::MATCH_LIST_WINDOW_TITLE,
    ));
}

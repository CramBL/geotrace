use std::fs;
use std::ops::Range;

use chrono::TimeDelta;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use gt_filter::TimeWindow;
use gt_session_bridge::{
    DisplayCategoryChange, ErrorCode, OpenPanel, PanelOpenState, QueryJobState, Reply, Request,
    SessionMapLayer, SessionState, SessionTimeFilter, SetSessionView, TimeFilterChange,
};
use gt_types::{FileIdx, FixRef, MarkerRequirement, PointIdx, TimeRange, TrackIdx, TrackRef};
use gt_ui_types::{
    DisplayCategory, EligibilityWithheld, MapElementRef, MapEligibility, MapEligibilityResult,
};
use rstest::rstest;

use crate::app::settings_autosave::SettingsAutosaver;
use crate::app::test_util::{harness, session};
use crate::app::{App, ui_tests};
use crate::settings::{MapLayerSetting, Settings};

#[test]
fn a_view_update_applies_without_painting_and_saves_map_settings() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    let mut harness = Harness::builder()
        .with_context_setup(gt_ui_theme::install_app_style)
        .with_wait_for_pending_images(false)
        .build_eframe(|cc| harness::build_app(cc, &path, false));
    harness
        .state_mut()
        .map
        .set_mapbox_token("configured".to_owned());
    let start = ui_tests::base_time() + TimeDelta::nanoseconds(123_456_789);
    let app = harness.state_mut();
    app.config = SettingsAutosaver::new(app.collect_snapshot());
    let change = SetSessionView {
        base_layer: Some(SessionMapLayer::Satellite),
        display_categories: vec![DisplayCategoryChange {
            category: DisplayCategory::SatelliteLabels,
            visible: false,
        }],
        time_filter: Some(TimeFilterChange::Set {
            start: Some(start),
            end: None,
        }),
    };
    let state = self::update_view(&mut harness, change.clone());
    assert_eq!(state.map.base_layer, SessionMapLayer::Satellite);
    assert!(
        !state
            .map
            .visible_categories
            .contains(&DisplayCategory::SatelliteLabels)
    );
    assert!(
        state
            .map
            .visible_categories
            .contains(&DisplayCategory::Tracks)
    );
    assert_eq!(state.time_filter, TimeWindow::From(start).into());
    assert_eq!(
        session::execute(&mut harness, Request::GetSessionState),
        Reply::SessionState {
            state: state.clone()
        }
    );
    assert!(harness.state().config.time_until_flush().is_some());
    harness.state().flush_settings();
    let saved: Settings = toml::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    assert_eq!(saved.map.layer, MapLayerSetting::Satellite);
    assert!(
        !saved
            .map
            .display_mask
            .is_visible(DisplayCategory::SatelliteLabels)
    );

    let notice_count = harness.state().toasts.len();
    assert_eq!(self::update_view(&mut harness, change), state);
    assert_eq!(harness.state().toasts.len(), notice_count);
    harness.run_steps(2);
    assert_eq!(
        session::visible_action_notices(&harness),
        ["MCP changed the map and time filter"]
    );
}

#[test]
fn clearing_the_time_filter_preserves_the_other_filters_and_map() {
    let mut harness = ui_tests::app_with_a_recording();
    let start = ui_tests::base_time();
    let min_duration = Some(TimeDelta::seconds(8));
    {
        let mut shared = harness.state_mut().shared.borrow_mut();
        shared.filter.set_time_window(TimeWindow::From(start));
        shared.filter.set_minimum_duration(min_duration);
        shared
            .filter
            .set_marker_requirement(MarkerRequirement::AnyMarker);
        shared.filter_state.secondary_zoom =
            Some(TimeRange::new(start, start + TimeDelta::seconds(1)));
    }
    let app = harness.state_mut();
    app.config = SettingsAutosaver::new(app.collect_snapshot());
    let before = app.session_state(session::SESSION_ID);
    let cleared = self::update_view(
        &mut harness,
        SetSessionView {
            time_filter: Some(TimeFilterChange::Clear {}),
            ..SetSessionView::default()
        },
    );
    assert_eq!(cleared.time_filter, SessionTimeFilter::All);
    assert_eq!(cleared.map, before.map);
    let shared = harness.state().shared.borrow();
    assert_eq!(shared.filter.minimum_duration(), min_duration);
    assert_eq!(
        shared.filter.marker_requirement(),
        MarkerRequirement::AnyMarker
    );
    assert!(shared.filter_state.secondary_zoom.is_none());
    assert!(harness.state().config.time_until_flush().is_none());
}

#[test]
fn a_rejected_view_update_leaves_all_owners_unchanged() {
    let mut harness = ui_tests::app_with_a_recording();
    let before = session::execute(&mut harness, Request::GetSessionState);
    let notice_count = harness.state().toasts.len();
    let start = ui_tests::base_time();
    let hidden = DisplayCategoryChange {
        category: DisplayCategory::Tracks,
        visible: false,
    };
    let change = SetSessionView {
        base_layer: None,
        display_categories: vec![hidden],
        time_filter: Some(TimeFilterChange::Set {
            start: Some(start),
            end: None,
        }),
    };
    for (invalid, error) in [
        (
            SetSessionView {
                base_layer: Some(SessionMapLayer::Satellite),
                ..change.clone()
            },
            ErrorCode::MapboxTokenRequired,
        ),
        (
            SetSessionView {
                time_filter: Some(TimeFilterChange::Set {
                    start: Some(start),
                    end: Some(start - TimeDelta::nanoseconds(1)),
                }),
                ..change.clone()
            },
            ErrorCode::InvalidTimeFilter,
        ),
        (
            SetSessionView {
                time_filter: Some(TimeFilterChange::Set {
                    start: None,
                    end: None,
                }),
                ..change.clone()
            },
            ErrorCode::InvalidTimeFilter,
        ),
        (
            SetSessionView {
                display_categories: vec![
                    hidden,
                    DisplayCategoryChange {
                        visible: true,
                        ..hidden
                    },
                ],
                ..change.clone()
            },
            ErrorCode::DuplicateDisplayCategory,
        ),
    ] {
        assert_eq!(
            session::execute(&mut harness, Request::SetSessionView(invalid)),
            Reply::from(error)
        );
        assert_eq!(
            session::execute(&mut harness, Request::GetSessionState),
            before
        );
        assert_eq!(harness.state().toasts.len(), notice_count);
    }
    harness.state_mut().begin_shutdown();
    assert_eq!(
        session::execute(&mut harness, Request::SetSessionView(change)),
        Reply::from(ErrorCode::ShuttingDown)
    );
}

#[test]
fn only_the_time_filter_change_makes_query_results_stale() {
    let mut harness = ui_tests::app_with_a_recording();
    let text = "points | where velocity > 1 km/h";
    let first = session::start_query(&mut harness, text);
    let completed = session::completed_single_track_query(1);
    assert_eq!(
        session::wait_for_query_to_stop(&mut harness, first),
        completed
    );
    self::update_view(
        &mut harness,
        SetSessionView {
            display_categories: vec![DisplayCategoryChange {
                category: DisplayCategory::QueryHighlights,
                visible: false,
            }],
            ..SetSessionView::default()
        },
    );
    assert_eq!(session::query_status(&mut harness, first), completed);

    let instant = ui_tests::base_time() + TimeDelta::seconds(1);
    self::update_view(
        &mut harness,
        SetSessionView {
            time_filter: Some(TimeFilterChange::Set {
                start: Some(instant),
                end: Some(instant),
            }),
            ..SetSessionView::default()
        },
    );
    assert_eq!(
        session::query_status(&mut harness, first),
        QueryJobState::Stale
    );
    let filtered = session::start_query(&mut harness, text);
    assert_eq!(
        session::wait_for_query_to_stop(&mut harness, filtered),
        completed
    );
    let results = session::results(&mut harness, session::first_query_match_request(filtered));
    let (matches, _) = session::match_page(results.page);
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].range, 1..2);
}

#[rstest]
#[case::at_the_fix(0, Some(0..1))]
#[case::one_nanosecond_after_the_fix(1, None)]
fn map_plot_and_query_use_the_same_exact_session_time_bound(
    #[case] start_offset_ns: i64,
    #[case] expected_match: Option<Range<usize>>,
) {
    let mut harness = ui_tests::app_with_a_recording();
    let base = ui_tests::base_time();
    let end = base + TimeDelta::nanoseconds(1);
    self::update_view(
        &mut harness,
        SetSessionView {
            time_filter: Some(TimeFilterChange::Set {
                start: Some(base + TimeDelta::nanoseconds(start_offset_ns)),
                end: Some(end),
            }),
            ..SetSessionView::default()
        },
    );
    let track = TrackRef::new(FileIdx::new(0), TrackIdx::new(0));
    let first_fix = MapElementRef::Fix(FixRef::new(track, PointIdx::new(0)));
    {
        let app = harness.state();
        let shared = app.shared.borrow();
        let map = MapEligibility::new(
            shared.loaded_files.files(),
            shared.tree.visibility(),
            &shared.filter,
            app.editor_query.session().matches(),
            shared.tree.generated_marker_visibility(),
            shared.tree.event_marker_visibility(),
        );
        let expected = if expected_match.is_some() {
            MapEligibilityResult::Eligible
        } else {
            MapEligibilityResult::Withheld(EligibilityWithheld::OutsideTimeFilter)
        };
        assert_eq!(map.element_eligibility(first_fix), expected);
    }
    let plot = OpenPanel::Plot {};
    assert_eq!(
        session::execute(&mut harness, Request::OpenPanel(plot)),
        Reply::PanelOpen {
            panel: plot.panel(),
            state: PanelOpenState::Docked,
        }
    );
    harness.run_steps(3);
    assert_eq!(
        harness
            .get_by_label("EPH (m)")
            .accesskit_node()
            .is_disabled(),
        expected_match.is_none()
    );

    let job = session::start_query(&mut harness, "points | where velocity > 1 km/h");
    assert_eq!(
        session::wait_for_query_to_stop(&mut harness, job),
        session::completed_single_track_query(usize::from(expected_match.is_some()))
    );
    let results = session::results(&mut harness, session::first_query_match_request(job));
    let (matches, _) = session::match_page(results.page);
    assert_eq!(
        matches
            .into_iter()
            .map(|matched| matched.range)
            .collect::<Vec<_>>(),
        expected_match.into_iter().collect::<Vec<_>>()
    );
}

fn update_view(harness: &mut Harness<'_, App>, change: SetSessionView) -> SessionState {
    let Reply::SessionState { state } = session::execute(harness, Request::SetSessionView(change))
    else {
        panic!("expected session state");
    };
    state
}

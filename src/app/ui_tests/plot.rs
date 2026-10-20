use chrono::TimeDelta;
use egui::epaint::Shape;
use egui_kittest::{Harness, kittest::Queryable as _};
use egui_phosphor::regular::ARROW_LINE_UP_LEFT as ICON_ARROW_LINE_UP_LEFT;
use egui_phosphor::regular::DOTS_SIX as ICON_DOTS_SIX;
use geotrace_sdk::{
    Angle, NavFileBuilder, NavFix, NavFixTime, SatelliteCoverage, SatelliteScope,
    SatelliteUsageReport, SolutionState,
};
use gt_test_utils::HarnessInteraction as _;
use gt_types::{FixRef, MetricKind, PointIdx};
use gt_ui_types::{MapElementRef, MapEligibility};
use rstest::rstest;
use strum::IntoEnumIterator as _;

use crate::app::App;
use crate::app::test_util;
use crate::app::test_util::harness::TestDroppedFile;
use crate::app::ui_tests;

/// With Sync to map off, no frame scans the loaded fixes for a range: the plot
/// takes a range from the map viewport only while the toggle is on.
#[rstest]
#[case::synced_to_the_map(true)]
#[case::not_synced_to_the_map(false)]
fn the_plot_takes_a_range_from_the_map_only_while_sync_to_map_is_on(#[case] sync_to_map: bool) {
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(test_util::harness::transient_app);
    harness.state().shared.borrow_mut().plot_state.sync_to_map = sync_to_map;
    test_util::harness::drop_file_and_wait_for_load(
        &mut harness,
        TestDroppedFile::bytes(ui_tests::minimal_gtd_bytes(), "test.gtd"),
    );
    harness.run_steps(3);

    let scanned = harness
        .state()
        .shared
        .borrow()
        .map_synced_plot_range
        .scanned_range();
    assert_eq!(scanned.is_some(), sync_to_map, "scanned range {scanned:?}");
}

/// Moves the legend overlay away from its docked position and expands it,
/// for tests that exercise dragging it back.
fn detach_legend(harness: &mut Harness<App>, offset: egui::Vec2) {
    {
        let mut shared = harness.state_mut().shared.borrow_mut();
        shared.plot_state.file_legend_offset = offset;
        shared.plot_state.file_legend_collapsed = false;
    }
    harness.step();
}

#[test]
fn legend_redock_icon_resets_offset_to_default() {
    let mut harness = ui_tests::harness_with_three_files_loaded();
    detach_legend(&mut harness, egui::vec2(220.0, 120.0));

    harness.get_by_label(ICON_ARROW_LINE_UP_LEFT).click();
    harness.step();

    let offset = harness
        .state()
        .shared
        .borrow()
        .plot_state
        .file_legend_offset;
    assert!(
        gt_plot::legend_is_docked(offset),
        "expected legend to re-dock at {:?}, got ({:.2},{:.2})",
        gt_plot::LEGEND_DOCK_OFFSET,
        offset.x,
        offset.y
    );
}

#[test]
fn dragging_files_header_far_across_many_frames_does_not_snap_back() {
    let mut harness = ui_tests::harness_with_three_files_loaded();

    let start = harness.get_by_label(ICON_DOTS_SIX).rect().center();
    harness.press_drag_release(start, egui::vec2(200.0, 150.0), 10);

    let offset = harness
        .state()
        .shared
        .borrow()
        .plot_state
        .file_legend_offset;
    assert!(
        !gt_plot::legend_is_docked(offset),
        "expected legend dragged far away to stay detached, got ({:.2},{:.2})",
        offset.x,
        offset.y
    );
}

/// Only the snap can dock the legend from this release point: it is 21 points
/// from the dock, inside the snap radius and past the tolerance
/// `gt_plot::legend_is_docked` allows.
#[test]
fn a_legend_drag_released_short_of_the_dock_snaps_to_it() {
    let mut harness = ui_tests::harness_with_three_files_loaded();
    detach_legend(&mut harness, egui::vec2(220.0, 120.0));

    let start = harness.get_by_label(ICON_DOTS_SIX).rect().center();
    harness.press_drag_release(start, egui::vec2(-195.0, -95.0), 1);

    let offset = harness
        .state()
        .shared
        .borrow()
        .plot_state
        .file_legend_offset;
    assert!(
        gt_plot::legend_is_docked(offset),
        "expected the legend released near the dock to snap to {:?}, got ({:.2},{:.2})",
        gt_plot::LEGEND_DOCK_OFFSET,
        offset.x,
        offset.y
    );
}

#[derive(Clone, Copy)]
enum CountSampleFixture {
    Isolated,
    Decimated,
}

impl CountSampleFixture {
    fn recording(self) -> Vec<u8> {
        let count = match self {
            Self::Isolated => 3,
            Self::Decimated => 12_000,
        };
        let mut recorder = NavFileBuilder::new().open();
        for index in 0..count {
            let time = ui_tests::base_time() + TimeDelta::milliseconds(i64::from(index) * 200);
            recorder.add_nav_fix(
                NavFix::builder()
                    .time(NavFixTime::Receiver(time))
                    .lat(Angle::degrees(
                        51.5 + f64::from(index) * FIX_LATITUDE_STEP_DEG,
                    ))
                    .lon(Angle::degrees(-0.1))
                    .heading(Angle::degrees(0.0))
                    .state(SolutionState::Fix)
                    .build(),
            );
            if matches!(self, Self::Decimated) || index == 1 {
                recorder.add_satellite_usage_report(
                    SatelliteUsageReport::builder()
                        .time(NavFixTime::Receiver(time + TimeDelta::milliseconds(50)))
                        .scope(SatelliteScope::Global)
                        .coverage(SatelliteCoverage::Partial)
                        .reported_count(8 + index % 5)
                        .satellites(Vec::new())
                        .build(),
                );
            }
        }
        let mut bytes = Vec::new();
        recorder
            .finish()
            .expect("valid recording")
            .write(&mut bytes)
            .expect("write to memory");
        bytes
    }

    fn drawn_sample(self, harness: &Harness<'_, App>) -> egui::Pos2 {
        let app = harness.state();
        let pane = app
            .tiles_tree
            .tiles
            .rect(app.plot_tile_id)
            .expect("plot pane");
        let mut shapes = Vec::new();
        let mut pending: Vec<_> = harness
            .output()
            .shapes
            .iter()
            .map(|clipped| &clipped.shape)
            .collect();
        while let Some(shape) = pending.pop() {
            match shape {
                Shape::Vec(children) => pending.extend(children),
                shape => shapes.push(shape),
            }
        }
        match self {
            Self::Isolated => shapes
                .iter()
                .find_map(|shape| match shape {
                    Shape::Circle(circle)
                        if pane.contains(circle.center) && (circle.radius - 3.0).abs() < 0.001 =>
                    {
                        Some(circle.center)
                    }
                    _ => None,
                })
                .expect("isolated count dot"),
            Self::Decimated => {
                let line = shapes
                    .iter()
                    .filter_map(|shape| match shape {
                        Shape::Path(path)
                            if !path.closed
                                && (path.stroke.width - gt_plot::DEFAULT_PLOT_LINE_WIDTH).abs()
                                    < 0.001
                                && path.points.iter().all(|point| pane.contains(*point)) =>
                        {
                            Some(path)
                        }
                        _ => None,
                    })
                    .max_by_key(|path| path.points.len())
                    .expect("count line");
                assert!(
                    line.points.len() < 12_000,
                    "the renderer selected a coarse mipmap level"
                );
                assert!(line.points.len() > 2);
                *line
                    .points
                    .get(line.points.len() / 2)
                    .expect("middle rendered sample")
            }
        }
    }
}

#[rstest]
#[case::isolated(CountSampleFixture::Isolated)]
#[case::decimated(CountSampleFixture::Decimated)]
fn count_sample_hover_highlights_its_associated_fix_through_map_policy(
    #[case] fixture: CountSampleFixture,
) {
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(test_util::harness::transient_app);
    {
        let mut shared = harness.state().shared.borrow_mut();
        shared.plot_state.sync_to_map = false;
        for kind in MetricKind::iter() {
            shared
                .plot_state
                .metric_vis
                .set(kind, kind == MetricKind::SatsFix);
        }
    }
    test_util::harness::drop_file_and_wait_for_load(
        &mut harness,
        TestDroppedFile::bytes(fixture.recording(), "counts.gtd"),
    );
    harness.run_steps(3);
    let position = fixture.drawn_sample(&harness);
    harness.hover_at_and_settle(position, 3);
    let (source_fix, source_latitude) = {
        let app = harness.state();
        let shared = app.shared.borrow();
        let fix = shared
            .plot_state
            .hovered_metric_fix
            .expect("selected count source");
        assert!(shared.plot_state.plot_cursor_snapped);
        assert!(shared.highlight.plot_hover_snapped);
        assert_eq!(
            shared.highlight.plot_hover_point,
            Some((fix.track.fi, fix.track.index, fix.point))
        );
        let point = fix
            .track
            .resolve(&shared.loaded_files)
            .expect("loaded track")
            .points
            .get(fix.point.as_usize())
            .expect("source point");
        let report = point
            .satellite_evidence
            .usage
            .first()
            .expect("source report");
        assert_eq!(
            shared.highlight.plot_hover_time,
            Some(report.metadata.normalized_time.utc())
        );
        assert_eq!(
            report.metadata.normalized_time.utc() - point.tpv.time().utc(),
            TimeDelta::milliseconds(50)
        );
        if matches!(fixture, CountSampleFixture::Isolated) {
            assert_eq!(fix.point, PointIdx::new(1));
            assert_eq!(
                shared.highlight.plot_hover_time,
                Some(ui_tests::base_time() + TimeDelta::milliseconds(250))
            );
        }
        (fix, point.tpv.lat().as_written())
    };
    harness.state_mut().query_window.open = true;
    harness.run_steps(3);
    ui_tests::run_query(
        &mut harness,
        &format!(
            "points | where {} deg < lat and lat < {} deg | hide",
            source_latitude - FIX_LATITUDE_STEP_DEG / 2.0,
            source_latitude + FIX_LATITUDE_STEP_DEG / 2.0,
        ),
    );
    harness.state_mut().query_window.open = false;
    harness.run_steps(3);
    harness.hover_at_and_settle(position, 3);
    {
        let app = harness.state();
        let shared = app.shared.borrow();
        assert!(shared.plot_state.plot_cursor_snapped);
        assert_eq!(shared.plot_state.hovered_metric_fix, Some(source_fix));
        assert_eq!(shared.highlight.plot_hover_point, None);
        assert!(!shared.highlight.plot_hover_snapped);
        let scope = MapEligibility::new(
            shared.loaded_files.files(),
            shared.tree.visibility(),
            &shared.filter,
            app.query_window.matches(),
            shared.tree.generated_marker_visibility(),
            shared.tree.event_marker_visibility(),
        )
        .with_display_mask(shared.display_mask);
        assert!(!scope.draws(MapElementRef::Fix(source_fix)));
        let next = FixRef::new(
            source_fix.track,
            PointIdx::new(source_fix.point.as_usize() + 1),
        );
        assert!(scope.draws(MapElementRef::Fix(next)));
    }
    harness
        .state()
        .shared
        .borrow_mut()
        .plot_state
        .metric_vis
        .set(MetricKind::SatsFix, false);
    harness.run_steps(3);
    {
        let shared = harness.state().shared.borrow();
        assert_eq!(shared.plot_state.hovered_metric_fix, None);
        assert!(!shared.plot_state.plot_cursor_snapped);
    }
    harness.hover_at_and_settle(egui::pos2(2.0, 2.0), 3);
    let shared = harness.state().shared.borrow();
    assert!(!shared.plot_state.plot_cursor_snapped);
    assert_eq!(shared.plot_state.hovered_time, None);
    assert_eq!(shared.plot_state.hovered_metric_fix, None);
    assert_eq!(shared.highlight.plot_hover_point, None);
    assert!(!shared.highlight.plot_hover_snapped);
}

const FIX_LATITUDE_STEP_DEG: f64 = 0.000_001;

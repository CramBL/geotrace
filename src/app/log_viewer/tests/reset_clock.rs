use egui_phosphor::regular::CLOCK_COUNTER_CLOCKWISE as ICON_DEVICE_CLOCK;
use gt_types::LoadedFile;

use super::*;

fn reset_clock_harness() -> Harness<'static, ViewerState> {
    let text = gt_test_utils::MULTI_REBOOT_CLOCK_RESET_LOG.replace(
        "host svc: after reset\n",
        "host svc: after reset\ncontinuation before synchronization\n",
    );
    let january = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 5).single().unwrap();
    let november = Utc.with_ymd_and_hms(2026, 11, 9, 8, 0, 0).single().unwrap();
    let recordings: Vec<LoadedFile> = [january, november]
        .into_iter()
        .map(|start| {
            gt_test_utils::loaded_file_with_tracks(vec![gt_test_utils::loaded_track_with_points(
                gt_test_utils::fixtures::nav_points_from(start, 3, 3600),
            )])
        })
        .collect();
    super::harness_of(recordings, &[("reboots.log", &text)])
}

#[test]
fn device_clock_row_clicks_preserve_the_map_and_synchronized_rows_navigate() {
    let mut harness = self::reset_clock_harness();
    let timestamp = format!("{ICON_DEVICE_CLOCK}2026-01-01 00:00:05");
    for recording in [0, 1] {
        super::anchor_log_to_recording(&mut harness, "reboots.log", recording);
        harness.state_mut().map_center = Some((-10.0, -20.0));
        let row = harness.topmost_matching(By::new().label(&timestamp)).rect();
        harness.hover_at_and_settle(row.center(), TOOLTIP_DELAY_FRAMES);
        harness.get_by_label(line_table::DEVICE_CLOCK_ROW_HOVER);
        assert_eq!(harness.state().log_hover.row_placement, None);
        harness.click_at(row.center());
        harness.run_steps(2);
        assert_eq!(harness.state().map_center, Some((-10.0, -20.0)));
    }
    super::type_into_live_filter(&mut harness, "synced");
    let placed = harness
        .state()
        .shown_log()
        .unwrap()
        .entry_placement(4)
        .unwrap()
        .position;
    let expected = Some((placed.0.as_degrees(), placed.1.as_degrees()));
    let synchronized = harness
        .topmost_matching(By::new().label("host svc: synced"))
        .rect();
    harness.click_at(synchronized.center());
    harness.run_steps(2);
    assert_eq!(harness.state().map_center, expected);
    assert!(harness.state().log_hover.row_placement.is_some());
}

#[test]
fn device_clock_source_rows_remain_copyable_and_reachable_without_a_map_target() {
    let mut harness = self::reset_clock_harness();
    let timestamp = format!("{ICON_DEVICE_CLOCK}2026-01-01 00:00:05");
    let timestamp_rect = harness.topmost_matching(By::new().label(&timestamp)).rect();
    let message = harness
        .topmost_matching(By::new().label("host svc: after reset"))
        .rect();
    let from = timestamp_rect.left_center() + egui::vec2(2.0, 0.0);
    let to = message.right_center() - egui::vec2(2.0, 0.0);
    harness.press_drag_release(from, to - from, 4);
    harness.input_mut().events.push(egui::Event::Copy);
    harness.step();
    let copied = harness.copied_text().unwrap();
    assert!(copied.contains("2026-01-01"));
    assert!(copied.contains("00:00:05"));
    assert!(copied.contains("host svc: after reset"));

    let continuation = harness
        .topmost_matching(By::new().label("continuation before synchronization"))
        .rect();
    harness.hover_at_and_settle(continuation.center(), TOOLTIP_DELAY_FRAMES);
    harness.get_by_label(&format!(
        "{}. {}.",
        line_table::DEVICE_CLOCK_ROW_HOVER,
        line_table::INTERPOLATED_TIMESTAMP_HOVER
    ));
    assert_eq!(harness.state().log_hover.row_placement, None);

    super::type_into_live_filter(&mut harness, "reset");
    assert_eq!(super::match_count(&harness), "2 of 9");
    harness.get_by_label("Clear").click();
    harness.run_steps(2);
    super::type_into_live_filter(&mut harness, "first");
    super::unfold_the_summary_panel(&mut harness);
    harness.get_by_label("Boot 2").click();
    harness.run_steps(2);
    let reveal = harness.state().viewer.diagnostic_reveal.unwrap();
    assert_eq!(reveal.entry_index, 1);
    assert_eq!(
        harness.state().shown_log().unwrap().parsed().entries()[reveal.entry_index].line_number,
        3
    );
    let row = harness.get_by_label(&timestamp).rect();
    let number = harness.bottommost_matching(By::new().label("3")).rect();
    assert!((number.center().y - row.center().y).abs() < 0.1);
    harness.click_at(row.center());
    harness.run_steps(2);
    assert_eq!(harness.state().map_center, None);
    assert_eq!(harness.state().log_hover.row_placement, None);
}

#[rstest]
#[case::dark(egui::Theme::Dark)]
#[case::light(egui::Theme::Light)]
fn viewer_draws_device_clock_messages_with_normal_contrast_and_shows_clock_status_snapshot(
    #[case] theme: egui::Theme,
) {
    let mut harness = super::rendering_harness_of(
        Vec::new(),
        &[("reboots.log", gt_test_utils::MULTI_REBOOT_CLOCK_RESET_LOG)],
    );
    harness.inner.ctx.set_theme(theme);
    harness.inner.run_steps(3);
    let message = harness
        .inner
        .topmost_matching(By::new().label("host svc: after reset"))
        .rect();
    let ordinary = harness.inner.get_by_label("host svc: first").rect();
    assert!((message.left() - ordinary.left()).abs() < 0.1);
    let text = harness
        .inner
        .output()
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.visual_bounding_rect().intersects(message) => {
                Some(text)
            }
            _ => None,
        })
        .unwrap();
    assert_eq!(
        text.galley.job.sections.last().unwrap().format.color,
        harness.inner.ctx.global_style().visuals.text_color()
    );
    let timestamp = format!("{ICON_DEVICE_CLOCK}2026-01-01 00:00:05");
    let row = harness
        .inner
        .topmost_matching(By::new().label(&timestamp))
        .rect();
    harness
        .inner
        .hover_at_and_settle(row.center(), TOOLTIP_DELAY_FRAMES);
    harness
        .inner
        .get_by_label(line_table::DEVICE_CLOCK_ROW_HOVER);
    harness.inner.run_steps(3);
    harness.snapshot(&format!("log_viewer_device_clock_{theme:?}").to_ascii_lowercase());
}

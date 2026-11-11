use egui_kittest::kittest::Queryable as _;
use gt_test_utils::HarnessInteraction as _;

#[test]
fn table_filter_edits_preserve_selectable_exporter_totals_and_the_count_warning() {
    let mut harness = super::harness_of(Vec::new(), &[("exported.log", EXPORTED_LOG)]);
    super::unfold_the_summary_panel(&mut harness);
    for text in ["", "fix acquired", ""] {
        if text.is_empty() {
            let id = harness.state().first_loaded_log();
            harness
                .state_mut()
                .logs
                .filter_stack_mut_by_id(id)
                .unwrap()
                .0
                .clear_live_filter();
            harness.run_steps(2);
        } else {
            super::type_into_live_filter(&mut harness, text);
        }
        assert_eq!(
            super::match_count(&harness),
            if text.is_empty() { "2 of 2" } else { "1 of 2" }
        );
        harness.get_by_label("Exporter-reported service totals");
        harness.get_by_label("Reported by the exporter. Table filters do not change these totals.");
        harness.get_by_label("The exporter counted 99 entries, this parse anchored 2");
        for (service, errors, warnings) in [("hal-powerd", "56,429", "17"), ("kernel", "3", "0")] {
            let row = harness.get_by_label(service).rect();
            for count in [errors, warnings] {
                assert!(
                    harness
                        .query_all_by_label(count)
                        .any(|node| row.y_range().contains(node.rect().center().y))
                );
            }
        }
        assert!(
            harness.get_by_label("hal-powerd").rect().top()
                < harness.get_by_label("kernel").rect().top()
        );
    }

    let summary = harness.get_by_label("Format").rect().center();
    harness.scroll_wheel_at(summary, -SUMMARY_SCROLL_PX, super::WHEEL_SETTLE_FRAMES);
    let count = harness.get_by_label("56,429").rect();
    harness.press_drag_release(count.left_center(), count.size().x * egui::Vec2::X, 4);
    harness.input_mut().events.push(egui::Event::Copy);
    harness.step();
    assert_eq!(harness.copied_text().unwrap().trim(), "56,429");
}

const SUMMARY_SCROLL_PX: f32 = 300.0;

const EXPORTED_LOG: &str = "\
2026-05-29 18:48:25 hal-powerd: fix acquired
2026-05-29 18:48:26 kernel: signal lost
----------- Journal summary -----------
Log entries: 99
--- Service error count ---
kernel -> 3 Errors
hal-powerd -> 56429 Errors
--- Service warning count ---
hal-powerd -> 17 Warnings
";

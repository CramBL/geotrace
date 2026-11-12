use std::sync::Arc;

use egui::Color32;
use egui::text::{ByteIndex, LayoutJob};
use gt_types::LoadedFile;

use super::*;

pub(super) struct RenderedMessage(Arc<LayoutJob>);

impl RenderedMessage {
    pub(super) fn of(harness: &Harness<ViewerState>, message: &str) -> Self {
        Self(
            harness
                .output()
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.job.text == message => {
                        Some(text.galley.job.clone())
                    }
                    _ => None,
                })
                .unwrap_or_else(|| panic!("message {message:?} is not rendered")),
        )
    }

    pub(super) fn color_of(&self, text: &str) -> Color32 {
        let offset = self
            .0
            .text
            .find(text)
            .expect("the message contains the text");
        self.0
            .sections
            .iter()
            .find(|section| section.byte_range.contains(&ByteIndex(offset)))
            .expect("a section contains the text")
            .format
            .color
    }
}

#[rstest]
#[case::standalone(Vec::new())]
#[case::mixed_positions(vec![super::recording("walk.gtd", 55.0)])]
fn message_contrast_and_service_colors_are_independent_of_positions(
    #[case] recordings: Vec<LoadedFile>,
    #[values(egui::Theme::Dark, egui::Theme::Light)] theme: egui::Theme,
    #[values(true, false)] service_colors: bool,
) {
    let with_recording = !recordings.is_empty();
    let mut harness = super::harness_of(recordings, &[("messages.log", MIXED_POSITION_LOG)]);
    harness.ctx.set_theme(theme);
    harness.state_mut().viewer.color_services = service_colors;
    harness.run_steps(3);
    let log = harness.state().shown_log().unwrap();
    assert_eq!(log.entry_placement(0).is_some(), with_recording);
    assert!(log.entry_placement(1).is_none());
    let visuals = &harness.ctx.global_style().visuals;
    let ordinary = visuals.text_color();
    let weak = visuals.weak_text_color();
    let expected_service = if service_colors {
        gt_ui_theme::log_service_slot_color(0).resolve(theme == egui::Theme::Dark)
    } else {
        ordinary
    };
    for (message, level, severity) in [
        (
            "host navsyncd: [ERROR receiver] first",
            "[ERROR receiver]",
            gt_ui_theme::error_indicator(theme == egui::Theme::Dark),
        ),
        (
            "host navsyncd: [DEBUG receiver] second",
            "[DEBUG receiver]",
            weak,
        ),
    ] {
        let rendered = RenderedMessage::of(&harness, message);
        assert_eq!(rendered.color_of("navsyncd:"), expected_service);
        assert_eq!(
            rendered.color_of("host"),
            if service_colors { weak } else { ordinary }
        );
        assert_eq!(rendered.color_of(level), severity);
        assert_eq!(rendered.0.sections.last().unwrap().format.color, ordinary);
    }
}

#[test]
fn resolving_years_preserves_normal_message_contrast_and_service_identity() {
    let mut harness = super::harness_of(
        Vec::new(),
        &[("unresolved.log", gt_test_utils::UNRESOLVED_YEARS_LOG)],
    );
    let message = "host navsyncd: gnss fix acquired";
    let before = RenderedMessage::of(&harness, message);
    let visuals = &harness.ctx.global_style().visuals;
    assert_eq!(before.color_of("acquired"), visuals.text_color());
    assert_eq!(
        before.color_of("navsyncd:"),
        gt_ui_theme::log_service_slot_color(0).resolve(visuals.dark_mode)
    );
    let continuation = RenderedMessage::of(&harness, "at 0x0000c3f4 in gnss_task+0x54");
    assert_eq!(continuation.color_of("gnss_task"), visuals.text_color());
    harness.get_by_label("    --05-20 12:00:00");
    let id = harness.state().first_loaded_log();
    let state = harness.state_mut();
    assert!(state.logs.get_mut_by_id(id).unwrap().resolve_years_as(
        YearResolution::PlaceLastEntryInYear(2026),
        &state.recordings.view(),
    ));
    harness.run_steps(3);
    let after = RenderedMessage::of(&harness, message);
    assert_eq!(after.0.sections, before.0.sections);
    assert!(harness.query_by_label("    --05-20 12:00:00").is_none());
}

const MIXED_POSITION_LOG: &str = "\
2026-05-29 18:48:25 host navsyncd: [ERROR receiver] first
2026-05-29 19:48:25 host navsyncd: [DEBUG receiver] second
";

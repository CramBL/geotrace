//! The toolbar's count of the logs that came back with a recording, and the
//! pulse that draws the eye to it when one does.

use std::f32::consts::TAU;

use egui::RichText;
use egui_phosphor::regular::ARTICLE as ICON_ARTICLE;

/// What the toolbar's log button says while no log is waiting for the viewer.
const BUTTON_HOVER: &str = "Read the loaded logs against the recordings";

/// How long the badge pulses after a recording load restored a log.
const PULSE_SECONDS: f32 = 2.0;

/// How many times the badge dims and comes back over [`PULSE_SECONDS`].
const PULSE_CYCLES: f32 = 3.0;

/// How far the badge dims at the bottom of a pulse, as a share of its opacity.
const PULSE_DEEPEST_DIP: f32 = 0.7;

/// The toolbar's standing announcement of the logs that came back with a
/// recording: how many the viewer has yet to be opened on, and what is left of
/// the pulse.
#[derive(Default)]
pub(in crate::app) struct RestoredLogsBadge {
    count: usize,

    pulse_seconds_left: f32,

    /// The frame that last advanced the pulse. A second layout pass of one
    /// frame advances it by nothing: the step is the time between frames.
    advanced_at: Option<f64>,
}

impl RestoredLogsBadge {
    /// Counts one log the app loaded with a recording, and starts a pulse
    /// where none is running.
    pub(in crate::app) fn note_log_loaded_with_a_recording(&mut self) {
        self.count += 1;
        if self.pulse_seconds_left <= 0.0 {
            self.pulse_seconds_left = PULSE_SECONDS;
            self.advanced_at = None;
        }
    }

    /// Takes the badge off the button, which is what opening the viewer does.
    pub(in crate::app) fn clear(&mut self) {
        self.count = 0;
        self.pulse_seconds_left = 0.0;
        self.advanced_at = None;
    }

    #[cfg(test)]
    pub(in crate::app) fn count(&self) -> usize {
        self.count
    }

    #[cfg(test)]
    pub(in crate::app) fn is_pulsing(&self) -> bool {
        self.pulse_seconds_left > 0.0
    }

    /// The label the toolbar's log button draws this frame, which is the icon
    /// with the amber count once logs came back with a recording. Advances the
    /// pulse by the time since the last frame.
    pub(in crate::app) fn toolbar_label(&mut self, ui: &egui::Ui) -> RichText {
        if self.count == 0 {
            return RichText::new(ICON_ARTICLE);
        }
        let opacity = self.advance_pulse(ui);
        let amber = gt_ui_theme::warning_amber(ui.visuals().dark_mode);
        RichText::new(format!("{ICON_ARTICLE} {}", self.count)).color(amber.gamma_multiply(opacity))
    }

    pub(in crate::app) fn toolbar_hover_text(&self) -> String {
        if self.count == 0 {
            return BUTTON_HOVER.to_owned();
        }
        format!(
            "{BUTTON_HOVER}. {} {} loaded with recordings since the viewer was last open.",
            self.count,
            gt_fmt::pluralize(self.count, "log", "logs")
        )
    }

    /// How opaque the badge draws this frame: full amber once the pulse is
    /// over, dimming [`PULSE_CYCLES`] times while it runs.
    ///
    /// The animation ends by itself: the last frame of a pulse is the last one
    /// to request a repaint.
    fn advance_pulse(&mut self, ui: &egui::Ui) -> f32 {
        if self.pulse_seconds_left <= 0.0 {
            return 1.0;
        }
        ui.ctx().request_repaint();
        self.advance_pulse_to(ui.input(|i| i.time))
    }

    /// The pulse advanced to the frame drawn at `now`, in seconds of the egui
    /// clock, which sets how opaque the badge draws.
    fn advance_pulse_to(&mut self, now: f64) -> f32 {
        let step = self.advanced_at.map_or(0.0, |last| (now - last).max(0.0));
        self.advanced_at = Some(now);
        self.pulse_seconds_left = (self.pulse_seconds_left - step as f32).max(0.0);

        let elapsed = PULSE_SECONDS - self.pulse_seconds_left;
        let dip = 0.5 - 0.5 * (elapsed / PULSE_SECONDS * PULSE_CYCLES * TAU).cos();
        1.0 - dip * PULSE_DEEPEST_DIP
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// egui lays a frame out a second time where a widget calls
    /// [`egui::Context::request_discard`]. The second pass draws the badge as
    /// the first one did.
    #[test]
    #[expect(clippy::float_cmp, reason = "the second pass changes nothing")]
    fn a_second_layout_pass_of_one_frame_leaves_the_pulse_where_it_is() {
        let mut badge = RestoredLogsBadge::default();
        badge.note_log_loaded_with_a_recording();
        badge.advance_pulse_to(10.0);

        let first_pass = badge.advance_pulse_to(10.5);
        let second_pass = badge.advance_pulse_to(10.5);

        assert_eq!(first_pass, second_pass);
        assert_eq!(badge.pulse_seconds_left, PULSE_SECONDS - 0.5);
    }
}

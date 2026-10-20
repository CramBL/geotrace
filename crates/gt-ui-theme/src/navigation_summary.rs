use egui::{Color32, Frame, RichText, Stroke};
use egui_phosphor::regular::{CHECK, QUESTION, X};
use gt_types::{PositionProvenance, SkyCounts, SolutionState};

#[derive(Clone, Copy, Debug)]
pub struct SatelliteCountBadge(pub SkyCounts);

pub struct SatelliteCountResponses {
    pub used: egui::Response,
    pub seen: egui::Response,
}

impl SatelliteCountBadge {
    pub fn ui(self, ui: &mut egui::Ui) -> SatelliteCountResponses {
        let dark_mode = ui.visuals().dark_mode;
        let neutral = ui.visuals().weak_text_color();
        let pair = ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = COUNT_SLOT_GAP_PX;
            let used = ui.colored_label(
                self.0.used.map_or(neutral, |value| {
                    crate::fix_count_tier(value).color(dark_mode)
                }),
                self.0
                    .used
                    .map_or_else(|| crate::EM_DASH.to_owned(), |value| value.to_string()),
            );
            ui.colored_label(neutral, "/");
            let seen = ui.colored_label(
                self.0.seen.map_or(neutral, |value| {
                    crate::seen_count_tier(value).color(dark_mode)
                }),
                self.0
                    .seen
                    .map_or_else(|| crate::EM_DASH.to_owned(), |value| value.to_string()),
            );
            SatelliteCountResponses { used, seen }
        });
        pair.response.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::WidgetType::Label,
                true,
                format!("Satellites used / seen {}", gt_fmt::SatelliteCounts(self.0)),
            )
        });
        pair.response.on_hover_text("Satellites used / seen");
        pair.inner
    }
}

#[derive(Clone, Copy, Debug)]
pub struct NavigationStatus {
    pub state: SolutionState,
    pub provenance: Option<PositionProvenance>,
}

impl NavigationStatus {
    pub fn color(&self, dark_mode: bool) -> Color32 {
        match self.state {
            SolutionState::Fix => crate::FIX_QUALITY_GREEN.resolve(dark_mode),
            SolutionState::NoFix => crate::FIX_QUALITY_RED.resolve(dark_mode),
            SolutionState::EstimatedOnly => crate::NAVIGATION_ESTIMATED.resolve(dark_mode),
            SolutionState::Unknown => crate::NAVIGATION_UNKNOWN.resolve(dark_mode),
        }
    }

    pub fn ui(&self, ui: &mut egui::Ui) -> egui::Response {
        let color = self.color(ui.visuals().dark_mode);
        let (symbol, explanation) = match self.state {
            SolutionState::Fix => (CHECK, "GNSS contribution reported or established"),
            SolutionState::NoFix => (X, "No navigation fix"),
            SolutionState::EstimatedOnly => (
                crate::ALMOST_EQUAL_TO,
                "Estimated without measured GNSS contribution",
            ),
            SolutionState::Unknown => (QUESTION, "Navigation state unknown"),
        };
        let synthesized = self.provenance == Some(PositionProvenance::SdkSynthesized);
        let frame = Frame::NONE.inner_margin(STATUS_MARGIN_PX);
        let frame = if synthesized {
            frame.stroke(Stroke::new(SYNTHESIZED_OUTLINE_WIDTH_PX, color))
        } else {
            frame
        };
        let response = frame
            .show(ui, |ui| ui.label(RichText::new(symbol).color(color)))
            .inner;
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Label, true, self.state.to_string())
        });
        response.on_hover_text(if synthesized {
            format!("{} · SDK synthesized\n{explanation}", self.state)
        } else {
            format!("{}\n{explanation}", self.state)
        })
    }
}

const COUNT_SLOT_GAP_PX: f32 = 4.0;
const STATUS_MARGIN_PX: i8 = 2;
const SYNTHESIZED_OUTLINE_WIDTH_PX: f32 = 1.0;

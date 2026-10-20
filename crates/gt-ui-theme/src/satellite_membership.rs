use egui::{Color32, Shape, Stroke, Vec2};

use gt_types::SatelliteMembership;

#[derive(Clone, Copy, Debug)]
pub struct SatelliteMembershipMark {
    pub membership: SatelliteMembership,
    pub radius: f32,
    pub minimum_unknown_radius: f32,
    pub color: Color32,
    pub edge: Stroke,
    pub hollow_fill: Color32,
}

impl SatelliteMembershipMark {
    pub fn label(&self) -> &'static str {
        match self.membership {
            SatelliteMembership::Used => "Used",
            SatelliteMembership::Unused => "Unused",
            SatelliteMembership::Unknown => "Usage unknown",
        }
    }

    pub fn paint(&self, painter: &egui::Painter, center: egui::Pos2) {
        match self.membership {
            SatelliteMembership::Used => {
                painter.circle(center, self.radius, self.color, self.edge);
            }
            SatelliteMembership::Unused => {
                painter.circle(center, self.radius, self.hollow_fill, self.edge);
            }
            SatelliteMembership::Unknown => {
                let radius = self.radius.max(self.minimum_unknown_radius);
                painter.add(Shape::convex_polygon(
                    vec![
                        center - Vec2::Y * radius,
                        center + Vec2::X * radius,
                        center + Vec2::Y * radius,
                        center - Vec2::X * radius,
                    ],
                    self.color,
                    Stroke::NONE,
                ));
            }
        }
    }

    pub fn ui(&self, ui: &mut egui::Ui) -> egui::Response {
        let diameter = self.radius.max(self.minimum_unknown_radius) * 2.0;
        let (rect, response) = ui.allocate_exact_size(Vec2::splat(diameter), egui::Sense::hover());
        self.paint(ui.painter(), rect.center());
        let label = self.label();
        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, label));
        response.on_hover_text(label)
    }
}

pub const SKY_UNKNOWN_MEMBERSHIP_MIN_RADIUS_PX: f32 = 2.0;
pub const MAP_UNKNOWN_MEMBERSHIP_MIN_RADIUS_PX: f32 = 1.5;

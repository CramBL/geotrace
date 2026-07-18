//! The sky plot's polar grid, shared by the per-report plot and the
//! whole-track trails plot: horizon rim, elevation rings, cardinal spokes,
//! and the dashed elevation-mask ring.

use egui::{Align2, FontId, Pos2, Shape, Stroke, Vec2};

use crate::projection;
use crate::style;

/// Draw the grid: the horizon rim, the inner elevation rings, and the
/// cardinal spokes. `full` labels the rings and all four cardinals; the
/// compact size labels north only and ticks the other cardinals.
pub(crate) fn draw_grid(ui: &egui::Ui, center: Pos2, radius: f32, full: bool) {
    let painter = ui.painter();
    let visuals = ui.visuals();
    let grid_stroke = Stroke::new(style::GRID_STROKE_WIDTH_PX, visuals.window_stroke.color);
    let label_color = visuals.weak_text_color();

    // Horizon rim plus the inner elevation rings.
    painter.circle_stroke(center, radius, grid_stroke);
    for elevation_deg in style::GRID_RING_ELEVATIONS_DEG {
        let ring_radius = radius * projection::unit_disc_radius(elevation_deg);
        painter.circle_stroke(center, ring_radius, grid_stroke);
        if full {
            painter.text(
                center
                    + Vec2::new(
                        style::ELEVATION_LABEL_OFFSET_X_PX,
                        -ring_radius - style::ELEVATION_LABEL_OFFSET_Y_PX,
                    ),
                Align2::LEFT_BOTTOM,
                format!("{elevation_deg:.0}{}", gt_ui_theme::DEGREE_SIGN),
                FontId::proportional(style::ELEVATION_LABEL_FONT_SIZE),
                label_color,
            );
        }
    }

    // Cardinal spokes through the center.
    painter.line_segment(
        [
            center - Vec2::new(radius, 0.0),
            center + Vec2::new(radius, 0.0),
        ],
        grid_stroke,
    );
    painter.line_segment(
        [
            center - Vec2::new(0.0, radius),
            center + Vec2::new(0.0, radius),
        ],
        grid_stroke,
    );

    // North up. The compact size labels north only and marks the other
    // cardinals with rim ticks.
    let cardinals: &[(&str, Vec2)] = if full {
        &[
            ("N", Vec2::new(0.0, -1.0)),
            ("E", Vec2::new(1.0, 0.0)),
            ("S", Vec2::new(0.0, 1.0)),
            ("W", Vec2::new(-1.0, 0.0)),
        ]
    } else {
        &[("N", Vec2::new(0.0, -1.0))]
    };
    let (font_size, label_offset) = if full {
        (
            style::FULL_CARDINAL_FONT_SIZE,
            style::FULL_CARDINAL_LABEL_OFFSET_PX,
        )
    } else {
        (
            style::COMPACT_CARDINAL_FONT_SIZE,
            style::COMPACT_CARDINAL_LABEL_OFFSET_PX,
        )
    };
    for (label, direction) in cardinals {
        painter.text(
            center + *direction * (radius + label_offset),
            Align2::CENTER_CENTER,
            *label,
            FontId::proportional(font_size),
            label_color,
        );
    }
    if !full {
        for direction in [
            Vec2::new(1.0, 0.0),
            Vec2::new(0.0, 1.0),
            Vec2::new(-1.0, 0.0),
        ] {
            painter.line_segment(
                [
                    center + direction * radius,
                    center + direction * (radius + style::COMPACT_CARDINAL_TICK_PX),
                ],
                grid_stroke,
            );
        }
    }
}

/// Draw the elevation mask as a dashed ring. Satellites below the mask stay
/// visible - the ring is context, not a filter.
pub(crate) fn draw_mask_ring(ui: &egui::Ui, center: Pos2, radius: f32, mask_deg: f32) {
    let ring_radius = radius * projection::unit_disc_radius(mask_deg);
    let points: Vec<Pos2> = (0..=style::MASK_RING_SEGMENTS)
        .map(|i| {
            let angle = i as f32 / style::MASK_RING_SEGMENTS as f32 * std::f32::consts::TAU;
            center + Vec2::new(angle.sin(), -angle.cos()) * ring_radius
        })
        .collect();
    let stroke = Stroke::new(style::GRID_STROKE_WIDTH_PX, ui.visuals().weak_text_color());
    ui.painter().add(Shape::dashed_line(
        &points,
        stroke,
        style::MASK_RING_DASH_PX,
        style::MASK_RING_GAP_PX,
    ));
}

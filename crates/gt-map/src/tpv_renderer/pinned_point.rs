use std::ptr;

use egui::{RichText, ScrollArea};
use egui_phosphor::regular::ARROW_SQUARE_OUT as ICON_ARROW_SQUARE_OUT;
use gt_fmt::UTC_SUBSECOND_FORMAT;
use gt_sky::{SkyHighlight, SkyPlotSize};
use gt_types::{NavPoint, SatelliteQueryScope};
use gt_ui_types::PointWindowFolds;

use super::navigation_details::NavigationDetails;
use super::point_metrics::{FixPlacement, PointMetrics};
use super::point_summary::PointSummary;
use super::satellite_tables::{self, MIN_SATELLITE_COLUMN_WIDTH_PX, STICKY_COLUMN_GAP_PX};
use super::sky_context::{self, SkyHost, SkySection};

#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(super) enum StickyRegion {
    Body,
    Inspection,
    Metrics,
    Sky,
    SkyHeader,
    Summary,
    Tables,
}

/// The containing window displays the timestamp in its title.
/// Returns whether the user requested sky trails at this point's time.
#[must_use]
pub(crate) fn show_sticky_tpv_content(
    ui: &mut egui::Ui,
    p: &NavPoint,
    sky: &SkySection<'_>,
    folds: &mut PointWindowFolds,
    recording_name: Option<&str>,
    placement: FixPlacement,
) -> bool {
    // The sky plot uses the preceding frame's table highlight.
    let highlight_id = sky_table_highlight_id(ui);
    let prev_highlight: Option<SkyHighlight> =
        ui.ctx().data(|d| d.get_temp(highlight_id)).flatten();
    let mut highlight: Option<SkyHighlight> = None;

    let metrics = PointMetrics {
        point: p,
        recording_name,
        placement,
    };
    let side_by_side =
        matches!(sky, SkySection::Report(_)) && ui.available_width() >= MIN_SIDE_BY_SIDE_WIDTH_PX;
    let inspection_id = ui.id().with("point_inspection");
    #[cfg(test)]
    ui.ctx().data_mut(|data| {
        for region in [
            StickyRegion::SkyHeader,
            StickyRegion::Sky,
            StickyRegion::Inspection,
        ] {
            data.remove::<egui::Rect>(egui::Id::new(("sticky_region", region)));
        }
    });
    #[cfg(test)]
    sticky_layout_rect(ui, StickyRegion::Body, ui.available_rect_before_wrap());
    let sky_and_metrics = |ui: &mut egui::Ui, folds: &mut PointWindowFolds| {
        let mut open_trails = false;
        if !matches!(sky, SkySection::TrackWithoutReports) {
            // Sized to the plot it titles: a right-aligned button in an
            // unbounded row would stretch the whole column to the window's
            // width and squeeze the satellite tables out.
            let header = ui.allocate_ui_with_layout(
                    egui::vec2(SkyPlotSize::Full.diameter(), ui.spacing().interact_size.y),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        let tint = ui.visuals().weak_text_color();
                        satellite_tables::fold_arrow(ui, folds.plot_folded, tint);
                        ui.label(RichText::new("Sky").strong());
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.add_enabled(
                                matches!(sky, SkySection::Report(report) if report.evidence.observation.is_some_and(|(_, observation)| observation.has_sky_position())),
                                egui::Button::new(ICON_ARROW_SQUARE_OUT).small(),
                            )
                            .on_hover_text("Open sky trails at this moment")
                            .on_disabled_hover_text("Satellite azimuth and elevation are required")
                        })
                        .inner
                    },
                );
            #[cfg(test)]
            sticky_layout_rect(ui, StickyRegion::SkyHeader, header.response.rect);
            let button = header.inner;
            open_trails = button.clicked();
            // The fold covers the header up to the button, not through it:
            // an interaction registered after the button would sit on top
            // of it and swallow its clicks.
            let mut fold_rect = header.response.rect;
            fold_rect.max.x = button.rect.left() - ui.spacing().item_spacing.x;
            // Explicit id: `.interact` on the container response reuses an
            // auto-generated id, which collides with identically laid out
            // siblings.
            let fold = ui.interact(fold_rect, ui.id().with("sky_fold"), egui::Sense::click());
            crate::hover_labels::hover_affordance(ui, fold.rect);
            if fold.clicked() {
                folds.plot_folded = !folds.plot_folded;
            }
            if !folds.plot_folded {
                let _plot = ui.scope(|ui| {
                    sky_context::sky_section_ui(
                        ui,
                        sky,
                        SkyPlotSize::Full,
                        prev_highlight,
                        SkyHost {
                            point: p,
                            identities_in_table: true,
                        },
                    );
                });
                #[cfg(test)]
                sticky_layout_rect(ui, StickyRegion::Sky, _plot.response.rect);
            }
            ui.add_space(6.0);
        }
        let _metrics = ui.scope(|ui| metrics.pinned_ui(ui));
        #[cfg(test)]
        sticky_layout_rect(ui, StickyRegion::Metrics, _metrics.response.rect);
        open_trails
    };
    let mut open_trails = false;
    if side_by_side {
        let width = ui.available_width();
        let height = ui.available_height();
        ui.scope(|ui| {
            ui.spacing_mut().item_spacing.x = STICKY_COLUMN_GAP_PX;
            ui.horizontal_top(|ui| {
                ui.allocate_ui_with_layout(
                    egui::vec2(SkyPlotSize::Full.diameter(), height),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        ui.set_width(SkyPlotSize::Full.diameter());
                        ScrollArea::both()
                            .id_salt(inspection_id.with("sky_metrics_scroll"))
                            .show(ui, |ui| open_trails = sky_and_metrics(ui, folds));
                    },
                );
                ui.allocate_ui_with_layout(
                    egui::vec2(
                        width - SkyPlotSize::Full.diameter() - STICKY_COLUMN_GAP_PX,
                        height,
                    ),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        ui.set_width(width - SkyPlotSize::Full.diameter() - STICKY_COLUMN_GAP_PX);
                        sticky_inspection_ui(ui, p, sky, inspection_id, &mut highlight);
                        let _tables = ScrollArea::both()
                            .id_salt(inspection_id.with("satellites_scroll"))
                            .show(ui, |ui| {
                                satellite_tables::sticky_satellites(ui, p, folds, &mut highlight)
                            });
                        #[cfg(test)]
                        sticky_layout_rect(ui, StickyRegion::Tables, _tables.inner_rect);
                    },
                );
            });
        });
    } else {
        sticky_inspection_ui(ui, p, sky, inspection_id, &mut highlight);
        let _stacked = ScrollArea::both()
            .id_salt(inspection_id.with("stacked_scroll"))
            .show(ui, |ui| {
                open_trails = sky_and_metrics(ui, folds);
                ui.add_space(6.0);
                let _tables = ui
                    .scope(|ui| satellite_tables::sticky_satellites(ui, p, folds, &mut highlight));
                #[cfg(test)]
                sticky_layout_rect(
                    ui,
                    StickyRegion::Tables,
                    _tables.response.rect.intersect(ui.clip_rect()),
                );
            });
    }

    if highlight != prev_highlight {
        ui.ctx().request_repaint();
    }
    ui.ctx()
        .data_mut(|d| d.insert_temp(highlight_id, highlight));
    open_trails
}

/// The containing window scopes the highlight key so concurrent point surfaces remain independent.
pub(super) fn sky_table_highlight_id(ui: &egui::Ui) -> egui::Id {
    ui.id().with("sky_table_highlight")
}

fn sticky_inspection_ui(
    ui: &mut egui::Ui,
    p: &NavPoint,
    sky: &SkySection<'_>,
    inspection_id: egui::Id,
    highlight: &mut Option<SkyHighlight>,
) {
    let borrowed = match sky {
        SkySection::Report(report) if !ptr::eq(report.evidence.point, p) => Some(report),
        _ => None,
    };
    let details_id = inspection_id.with("details_open");
    let nearby_id = inspection_id.with("nearby_open");
    let mut details_open = ui
        .ctx()
        .data_mut(|data| data.get_persisted::<bool>(details_id))
        .unwrap_or(false);
    let mut nearby_open = ui
        .ctx()
        .data_mut(|data| data.get_persisted::<bool>(nearby_id))
        .unwrap_or(false);
    let _header = ui.vertical(|ui| {
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("Satellites").strong());
            PointSummary(p).status_ui(ui);
            let counts = PointSummary(p).counts_ui(ui, SatelliteQueryScope::Global);
            if p.resolved_used_count(SatelliteQueryScope::Global).is_some()
                && crate::hover_labels::hover_affordance(ui, counts.used.rect)
            {
                *highlight = Some(SkyHighlight::in_fix());
            }
        });
        if p.navigation.method.is_some() || p.navigation.dimension.is_some() {
            ui.horizontal_wrapped(|ui| PointSummary(p).solution_fields_ui(ui));
        }
        ui.horizontal_wrapped(|ui| {
            ui.toggle_value(&mut details_open, "Details")
                .on_hover_text("Inspect navigation evidence and recorded reports");
            if borrowed.is_some() {
                ui.toggle_value(&mut nearby_open, "Nearby sky details")
                    .on_hover_text("Inspect the source of nearby sky context");
            }
        });
    });
    #[cfg(test)]
    sticky_layout_rect(ui, StickyRegion::Summary, _header.response.rect);
    ui.ctx().data_mut(|data| {
        data.insert_persisted(details_id, details_open);
        data.insert_persisted(nearby_id, nearby_open);
    });
    if details_open || (nearby_open && borrowed.is_some()) {
        let height = (ui.available_height() * DETAILS_HEIGHT_FRACTION)
            .max(DETAILS_MIN_HEIGHT_PX)
            .min(ui.available_height());
        ui.scope_builder(egui::UiBuilder::new().id(inspection_id), |ui| {
            let _inspection = ScrollArea::both()
                .id_salt("evidence_scroll")
                .max_height(height)
                .show(ui, |ui| {
                    if details_open {
                        ui.push_id("point_evidence", |ui| NavigationDetails(p).inspector_ui(ui));
                    }
                    if nearby_open && let Some(report) = borrowed {
                        ui.push_id("nearby_evidence", |ui| {
                            ui.label(sky_context::report_age_description(report.age));
                            if let Some((index, observation)) = report.evidence.observation {
                                ui.label(format!(
                                    "Sky observation {} · {} UTC",
                                    index + 1,
                                    observation
                                        .metadata
                                        .normalized_time
                                        .utc()
                                        .format(UTC_SUBSECOND_FORMAT)
                                ));
                            }
                            for (index, usage) in report.evidence.usage_reports() {
                                ui.label(format!(
                                    "Sky usage {} · {} UTC",
                                    index + 1,
                                    usage
                                        .metadata
                                        .normalized_time
                                        .utc()
                                        .format(UTC_SUBSECOND_FORMAT)
                                ));
                            }
                            NavigationDetails(report.evidence.point).reports_ui(ui);
                        });
                    }
                });
            #[cfg(test)]
            sticky_layout_rect(ui, StickyRegion::Inspection, _inspection.inner_rect);
        });
    }
}

#[cfg(test)]
fn sticky_layout_rect(ui: &egui::Ui, region: StickyRegion, rect: egui::Rect) {
    ui.ctx()
        .data_mut(|data| data.insert_temp(egui::Id::new(("sticky_region", region)), rect));
}

const DETAILS_HEIGHT_FRACTION: f32 = 0.5;
const DETAILS_MIN_HEIGHT_PX: f32 = 60.0;

/// Width the window needs to put the sky plot beside the satellite tables:
/// the full plot's own diameter, the column gap, and enough left for a
/// readable table. Narrower than this the plot stacks above them instead.
pub(super) const MIN_SIDE_BY_SIDE_WIDTH_PX: f32 =
    SkyPlotSize::Full.diameter() + STICKY_COLUMN_GAP_PX + MIN_SATELLITE_COLUMN_WIDTH_PX;

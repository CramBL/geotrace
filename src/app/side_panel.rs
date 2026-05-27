use nav_fmt;
use nav_types::{
    DataCategory, DataPointRef, GlobalFilter, HighlightScope, LoadedFile, LoadedTrip, MapHighlight,
    TripDataVisibility,
};
use uom::si::angle::degree;

use super::filter_panel::{FilterPanelState, render_filter_panel};
use super::trip_data_panel::{DeleteConfirmState, SelectionKey, TripDataPanelState, apply_click};

/// Bundles the mutable state required to render the trip data side panel.
/// Avoids passing each field individually down the call chain.
pub struct PanelContext<'a> {
    pub files: &'a [LoadedFile],
    pub visibility: &'a mut TripDataVisibility,
    pub highlight: &'a mut MapHighlight,
    pub filter: &'a mut GlobalFilter,
    pub filter_state: &'a mut FilterPanelState,
    pub panel: &'a mut TripDataPanelState,
    pub map_center_request: &'a mut Option<(f64, f64)>,
    /// When a panel list item click opens a sticky popup, this carries the
    /// suggested screen position for the popup window (just right of the panel).
    pub popup_pos_request: &'a mut Option<egui::Pos2>,
    /// Set to `true` by any action that changes which trips are visible, so the
    /// map can zoom to fit the newly visible data on the same frame.
    pub zoom_to_visible_request: &'a mut bool,
}

pub fn show_side_panel(ui: &mut egui::Ui, ctx: &mut PanelContext<'_>) {
    let header = ui.horizontal(|ui| {
        let (_, grip) = ui.allocate_exact_size(egui::vec2(10.0, 18.0), egui::Sense::drag());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ctx.panel.detached {
                if ui.small_button("Dock").clicked() {
                    ctx.panel.detached = false;
                }
            } else if ui
                .small_button(egui_phosphor::regular::ARROW_SQUARE_OUT)
                .on_hover_text("Pop out")
                .clicked()
            {
                ctx.panel.detached = true;
            }
        });
        grip
    });
    if header.inner.dragged()
        && ui
            .ctx()
            .pointer_latest_pos()
            .is_some_and(|p| !ui.clip_rect().contains(p))
    {
        ctx.panel.detached = true;
    }

    ui.separator();
    render_filter_panel(ui, ctx.files, ctx.filter, ctx.filter_state);

    let filter_snapshot = *ctx.filter;
    // Collect every trip that is not currently visible: disabled file, disabled
    // trip, or fails the active filter.  All three cases are "filtered out".
    let vis = &*ctx.visibility;
    let filtered_out: Vec<SelectionKey> = ctx
        .files
        .iter()
        .enumerate()
        .flat_map(|(fi, file)| {
            let file_enabled = vis.files.get(fi).is_some_and(|fv| fv.enabled);
            file.trips.iter().enumerate().filter_map(move |(ti, trip)| {
                let trip_enabled = file_enabled
                    && vis
                        .files
                        .get(fi)
                        .and_then(|fv| fv.trips.get(ti))
                        .is_some_and(|tv| tv.enabled);
                let passes = nav_types::trip_passes_filter(&trip.metadata, &filter_snapshot);
                if !trip_enabled || !passes {
                    Some(SelectionKey::Trip(fi, ti))
                } else {
                    None
                }
            })
        })
        .collect();
    if !filtered_out.is_empty() {
        let clicked = ui
            .scope(|ui| {
                // Turn the button red on hover/active to signal it's destructive.
                let v = ui.visuals_mut();
                v.widgets.hovered.bg_fill = egui::Color32::from_rgb(160, 35, 35);
                v.widgets.hovered.fg_stroke.color = egui::Color32::WHITE;
                v.widgets.active.bg_fill = egui::Color32::from_rgb(130, 25, 25);
                v.widgets.active.fg_stroke.color = egui::Color32::WHITE;
                ui.button(format!(
                    "{} Delete all filtered data",
                    egui_phosphor::regular::TRASH
                ))
                .clicked()
            })
            .inner;
        if clicked {
            ctx.panel.delete_confirm = Some(DeleteConfirmState {
                items: filtered_out,
            });
        }
    }

    ui.separator();

    // Convenience buttons: show / hide all files and trips at once.
    ui.horizontal(|ui| {
        if ui.small_button("Show all").clicked() {
            ctx.visibility.set_all_enabled(true);
            *ctx.zoom_to_visible_request = true;
        }
        if ui.small_button("Hide all").clicked() {
            ctx.visibility.set_all_enabled(false);
        }
    });

    let ordered_keys: Vec<SelectionKey> = ctx
        .files
        .iter()
        .enumerate()
        .flat_map(|(fi, file)| {
            let mut keys = vec![SelectionKey::File(fi)];
            if ctx.panel.expanded_files.contains(&fi) {
                for ti in 0..file.trips.len() {
                    keys.push(SelectionKey::Trip(fi, ti));
                }
            }
            keys
        })
        .collect();

    egui::ScrollArea::vertical().show(ui, |ui| {
        for fi in 0..ctx.files.len() {
            render_file_row(ui, fi, ctx, &ordered_keys);
        }
    });
}

fn render_file_row(
    ui: &mut egui::Ui,
    fi: usize,
    ctx: &mut PanelContext<'_>,
    ordered_keys: &[SelectionKey],
) {
    let Some(file) = ctx.files.get(fi) else {
        return;
    };
    let is_expanded = ctx.panel.expanded_files.contains(&fi);
    let file_map_hovered = ctx.highlight.hover.is_some_and(|s| match s {
        HighlightScope::Point(r) => r.file_index == fi,
        HighlightScope::Trip { file_index, .. }
        | HighlightScope::TripCategory { file_index, .. }
        | HighlightScope::File { file_index } => file_index == fi,
    });
    let file_key = SelectionKey::File(fi);

    // Yellow map-hover highlight; pick a shade that reads well in both themes.
    let map_hover_bg = map_hover_color(ui);

    let row_response = ui.horizontal(|ui| {
        let Some(file_vis) = ctx.visibility.files.get_mut(fi) else {
            return ui.label("").clone();
        };
        ui.checkbox(&mut file_vis.enabled, "");
        let arrow = if is_expanded {
            egui_phosphor::regular::CARET_DOWN
        } else {
            egui_phosphor::regular::CARET_RIGHT
        };
        let dist = nav_fmt::format_distance(file.metadata.total_distance_km);
        let dur = nav_fmt::format_human_terse_duration(file.metadata.total_duration);
        let label = format!("{arrow} {}  {dist}  {dur}", file.metadata.filename);
        let text = egui::RichText::new(label);
        ui.selectable_label(ctx.panel.selection.contains(&file_key), text)
            .on_hover_text(&file.metadata.filename)
    });
    if file_map_hovered {
        paint_map_hover_bg(ui, row_response.response.rect, map_hover_bg);
    }
    let response = row_response.inner;
    let modifiers = ui.ctx().input(|i| i.modifiers);
    if response.clicked() {
        if modifiers.ctrl || modifiers.shift {
            apply_click(
                ctx.panel,
                file_key.clone(),
                modifiers.ctrl,
                modifiers.shift,
                ordered_keys,
            );
        } else {
            if is_expanded {
                ctx.panel.expanded_files.remove(&fi);
            } else {
                ctx.panel.expanded_files.insert(fi);
            }
            apply_click(ctx.panel, file_key.clone(), false, false, ordered_keys);
        }
    }
    response.context_menu(|ui| {
        if ui.button("Show only this file").clicked() {
            ctx.visibility.show_only_file(fi);
            ui.close();
        }
        ui.separator();
        if ui.button("Delete").clicked() {
            ctx.panel.delete_confirm = Some(DeleteConfirmState {
                items: vec![file_key.clone()],
            });
            ui.close();
        }
        if ctx.panel.selection.len() >= 2 && ui.button("Delete selected").clicked() {
            ctx.panel.delete_confirm = Some(DeleteConfirmState {
                items: ctx.panel.selection.iter().cloned().collect(),
            });
            ui.close();
        }
    });

    if is_expanded {
        ui.indent(format!("file_{fi}"), |ui| {
            render_file_trips(ui, fi, ctx, ordered_keys);
        });
    }
}

fn render_file_trips(
    ui: &mut egui::Ui,
    fi: usize,
    ctx: &mut PanelContext<'_>,
    ordered_keys: &[SelectionKey],
) {
    let trip_count = ctx.files.get(fi).map_or(0, |f| f.trips.len());
    for ti in 0..trip_count {
        render_trip_row(ui, fi, ti, ctx, ordered_keys);
    }
}

fn render_trip_row(
    ui: &mut egui::Ui,
    fi: usize,
    ti: usize,
    ctx: &mut PanelContext<'_>,
    ordered_keys: &[SelectionKey],
) {
    let (trip, passes, is_expanded, trip_panel_hovered, trip_map_hovered, key) = {
        let Some(file) = ctx.files.get(fi) else {
            return;
        };
        let Some(trip) = file.trips.get(ti) else {
            return;
        };
        let passes = nav_types::trip_passes_filter(&trip.metadata, ctx.filter);
        let is_expanded = ctx.panel.expanded_trips.contains(&(fi, ti));
        // Blue text: side-panel hover (mouse over trip name in the list).
        let trip_panel_hovered = ctx.highlight.hover.is_some_and(|s| {
            matches!(s, HighlightScope::Trip { file_index, trip_index }
                if file_index == fi && trip_index == ti)
        });
        // Yellow background: map hover (mouse over a TPV/marker on the map).
        let trip_map_hovered = ctx.highlight.hover.is_some_and(
            |s| matches!(s, HighlightScope::Point(r) if r.file_index == fi && r.trip_index == ti),
        );
        let key = SelectionKey::Trip(fi, ti);
        (
            trip.clone(),
            passes,
            is_expanded,
            trip_panel_hovered,
            trip_map_hovered,
            key,
        )
    };

    // Capture before any mutable borrow so we can detect the all-hidden → visible transition.
    let was_all_hidden = !ctx
        .visibility
        .files
        .iter()
        .any(|f| f.enabled && f.trips.iter().any(|t| t.enabled));

    let map_hover_bg = map_hover_color(ui);

    let row_response = ui.horizontal(|ui| {
        let Some(file_vis) = ctx.visibility.files.get_mut(fi) else {
            return (ui.label("").clone(), false);
        };
        let Some(trip_vis) = file_vis.trips.get_mut(ti) else {
            return (ui.label("").clone(), false);
        };
        let chk = ui.checkbox(&mut trip_vis.enabled, "");
        // If the trip was just enabled while its parent file was disabled,
        // also enable the file so the trip actually becomes visible.
        let need_enable_file = chk.changed() && trip_vis.enabled && !file_vis.enabled;
        if need_enable_file {
            file_vis.enabled = true;
        }
        let newly_enabled = chk.changed() && trip_vis.enabled;
        let arrow = if is_expanded {
            egui_phosphor::regular::CARET_DOWN
        } else {
            egui_phosphor::regular::CARET_RIGHT
        };
        let dist = nav_fmt::format_distance(trip.metadata.distance_km);
        let dur = nav_fmt::format_human_terse_duration(trip.metadata.duration);
        let label = format!("{arrow} T{}  {dist}  {dur}", trip.metadata.index);
        let mut text = egui::RichText::new(label);
        if !passes {
            text = text.weak();
        }
        if trip_panel_hovered {
            text = text.color(egui::Color32::from_rgb(100, 200, 255));
        }
        (
            ui.selectable_label(ctx.panel.selection.contains(&key), text),
            newly_enabled,
        )
    });
    if trip_map_hovered {
        paint_map_hover_bg(ui, row_response.response.rect, map_hover_bg);
    }
    let (response, newly_enabled) = row_response.inner;
    // Zoom to fit when a trip becomes visible after everything was hidden.
    if newly_enabled && was_all_hidden {
        *ctx.zoom_to_visible_request = true;
    }
    if response.hovered() {
        ctx.highlight.hover = Some(HighlightScope::Trip {
            file_index: fi,
            trip_index: ti,
        });
    }
    let modifiers = ui.ctx().input(|i| i.modifiers);
    if response.clicked() {
        if modifiers.ctrl || modifiers.shift {
            apply_click(
                ctx.panel,
                key.clone(),
                modifiers.ctrl,
                modifiers.shift,
                ordered_keys,
            );
        } else {
            if is_expanded {
                ctx.panel.expanded_trips.remove(&(fi, ti));
            } else {
                ctx.panel.expanded_trips.insert((fi, ti));
            }
            apply_click(ctx.panel, key.clone(), false, false, ordered_keys);
        }
    }
    response.context_menu(|ui| {
        if ui.button("Show only this trip").clicked() {
            ctx.visibility.show_only_trip(fi, ti);
            *ctx.zoom_to_visible_request = true;
            ui.close();
        }
        ui.separator();
        if ui.button("Delete").clicked() {
            ctx.panel.delete_confirm = Some(DeleteConfirmState {
                items: vec![key.clone()],
            });
            ui.close();
        }
        if ctx.panel.selection.len() >= 2 && ui.button("Delete selected").clicked() {
            ctx.panel.delete_confirm = Some(DeleteConfirmState {
                items: ctx.panel.selection.iter().cloned().collect(),
            });
            ui.close();
        }
    });

    if is_expanded {
        ui.indent(format!("trip_{fi}_{ti}"), |ui| {
            render_trip_categories(ui, fi, ti, &trip, ctx);
        });
    }
}

fn render_trip_categories(
    ui: &mut egui::Ui,
    fi: usize,
    ti: usize,
    trip: &LoadedTrip,
    ctx: &mut PanelContext<'_>,
) {
    let Some(file_vis) = ctx.visibility.files.get_mut(fi) else {
        return;
    };
    let Some(trip_vis) = file_vis.trips.get_mut(ti) else {
        return;
    };

    let track_resp = ui.horizontal(|ui| {
        ui.checkbox(&mut trip_vis.track_visible, "");
        ui.label("Trip Track")
    });
    if track_resp.inner.hovered() {
        ctx.highlight.hover = Some(HighlightScope::TripCategory {
            file_index: fi,
            trip_index: ti,
            category: DataCategory::TripTrack,
        });
    }

    let tpv_count = trip.points.len();
    if tpv_count > 0 {
        let cat = DataCategory::Tpv;
        let expanded = ctx.panel.expanded_categories.contains(&(fi, ti, cat));
        let tpv_cat_resp = ui.horizontal(|ui| {
            ui.checkbox(&mut trip_vis.tpv_visible, "");
            let arrow = if expanded {
                egui_phosphor::regular::CARET_DOWN
            } else {
                egui_phosphor::regular::CARET_RIGHT
            };
            let resp = ui.selectable_label(expanded, format!("{arrow} Trip Points  {tpv_count}"));
            if resp.clicked() {
                toggle_category(ctx.panel, fi, ti, cat);
            }
            resp
        });
        if tpv_cat_resp.inner.hovered() {
            ctx.highlight.hover = Some(HighlightScope::TripCategory {
                file_index: fi,
                trip_index: ti,
                category: DataCategory::Tpv,
            });
        }
        if expanded {
            ui.indent(format!("tpv_{fi}_{ti}"), |ui| {
                render_tpv_items(
                    ui,
                    fi,
                    ti,
                    trip,
                    ctx.highlight,
                    ctx.map_center_request,
                    ctx.popup_pos_request,
                );
            });
        }
    }

    let sat_count = trip
        .points
        .iter()
        .filter(|p| p.satellites.is_some())
        .count();
    if sat_count > 0 {
        let cat = DataCategory::SatelliteReport;
        let expanded = ctx.panel.expanded_categories.contains(&(fi, ti, cat));
        let sat_cat_resp = ui.horizontal(|ui| {
            ui.checkbox(&mut trip_vis.satellites_visible, "");
            let arrow = if expanded {
                egui_phosphor::regular::CARET_DOWN
            } else {
                egui_phosphor::regular::CARET_RIGHT
            };
            let resp =
                ui.selectable_label(expanded, format!("{arrow} Satellite Reports  {sat_count}"));
            if resp.clicked() {
                toggle_category(ctx.panel, fi, ti, cat);
            }
            resp
        });
        if sat_cat_resp.inner.hovered() {
            ctx.highlight.hover = Some(HighlightScope::TripCategory {
                file_index: fi,
                trip_index: ti,
                category: DataCategory::SatelliteReport,
            });
        }
        if expanded {
            ui.indent(format!("sat_{fi}_{ti}"), |ui| {
                render_satellite_report_items(
                    ui,
                    fi,
                    ti,
                    trip,
                    ctx.highlight,
                    ctx.map_center_request,
                    ctx.popup_pos_request,
                );
            });
        }
    }

    let custom_count = trip.custom_markers.len();
    if custom_count > 0 {
        let cat = DataCategory::CustomMarker;
        let expanded = ctx.panel.expanded_categories.contains(&(fi, ti, cat));
        let custom_cat_resp = ui.horizontal(|ui| {
            ui.checkbox(&mut trip_vis.custom_markers_visible, "");
            let arrow = if expanded {
                egui_phosphor::regular::CARET_DOWN
            } else {
                egui_phosphor::regular::CARET_RIGHT
            };
            let resp =
                ui.selectable_label(expanded, format!("{arrow} Custom Markers  {custom_count}"));
            if resp.clicked() {
                toggle_category(ctx.panel, fi, ti, cat);
            }
            resp
        });
        if custom_cat_resp.inner.hovered() {
            ctx.highlight.hover = Some(HighlightScope::TripCategory {
                file_index: fi,
                trip_index: ti,
                category: DataCategory::CustomMarker,
            });
        }
        if expanded {
            ui.indent(format!("custom_{fi}_{ti}"), |ui| {
                render_custom_marker_items(
                    ui,
                    fi,
                    ti,
                    trip,
                    ctx.highlight,
                    ctx.map_center_request,
                    ctx.popup_pos_request,
                );
            });
        }
    }

    let gen_count = trip.generated_markers.len();
    if gen_count > 0 {
        let cat = DataCategory::GeneratedMarker;
        let expanded = ctx.panel.expanded_categories.contains(&(fi, ti, cat));
        let gen_cat_resp = ui.horizontal(|ui| {
            ui.checkbox(&mut trip_vis.generated_markers_visible, "");
            let arrow = if expanded {
                egui_phosphor::regular::CARET_DOWN
            } else {
                egui_phosphor::regular::CARET_RIGHT
            };
            let resp =
                ui.selectable_label(expanded, format!("{arrow} Generated Markers  {gen_count}"));
            if resp.clicked() {
                toggle_category(ctx.panel, fi, ti, cat);
            }
            resp
        });
        if gen_cat_resp.inner.hovered() {
            ctx.highlight.hover = Some(HighlightScope::TripCategory {
                file_index: fi,
                trip_index: ti,
                category: DataCategory::GeneratedMarker,
            });
        }
        if expanded {
            ui.indent(format!("gen_{fi}_{ti}"), |ui| {
                render_generated_marker_items(
                    ui,
                    fi,
                    ti,
                    trip,
                    ctx.highlight,
                    ctx.map_center_request,
                    ctx.popup_pos_request,
                );
            });
        }
    }
}

/// Returns a yellow tint color appropriate for the current light/dark theme.
/// Used to highlight rows that correspond to a map-hovered element.
fn map_hover_color(ui: &egui::Ui) -> egui::Color32 {
    if ui.visuals().dark_mode {
        // Warm amber on a dark background — bright enough to notice, dim enough
        // not to wash out the white text.
        egui::Color32::from_rgba_unmultiplied(210, 160, 0, 90)
    } else {
        // Soft golden tint on a light background — subtle so black text stays
        // readable, but clearly different from the plain white row.
        egui::Color32::from_rgba_unmultiplied(200, 140, 0, 55)
    }
}

/// Paints a filled rounded rectangle behind a row to signal that the hovered
/// map element belongs to that row. The shape is submitted to the background
/// layer so it renders beneath the row widgets.
fn paint_map_hover_bg(ui: &egui::Ui, rect: egui::Rect, color: egui::Color32) {
    let bg_layer = egui::LayerId::new(egui::Order::Background, egui::Id::new("map_hover_bg"));
    let painter = ui
        .ctx()
        .layer_painter(bg_layer)
        .with_clip_rect(ui.clip_rect());
    painter.rect_filled(rect.expand2(egui::vec2(2.0, 1.0)), 3.0, color);
}

fn toggle_category(panel: &mut TripDataPanelState, fi: usize, ti: usize, cat: DataCategory) {
    let key = (fi, ti, cat);
    if panel.expanded_categories.contains(&key) {
        panel.expanded_categories.remove(&key);
    } else {
        panel.expanded_categories.insert(key);
    }
}

fn render_tpv_items(
    ui: &mut egui::Ui,
    fi: usize,
    ti: usize,
    trip: &LoadedTrip,
    highlight: &mut MapHighlight,
    map_center_request: &mut Option<(f64, f64)>,
    popup_pos_request: &mut Option<egui::Pos2>,
) {
    for (pi, point) in trip.points.iter().enumerate() {
        let point_ref = DataPointRef {
            file_index: fi,
            trip_index: ti,
            category: DataCategory::Tpv,
            point_index: pi,
        };
        let is_sticky = highlight.sticky.is_some_and(|r| r == point_ref);
        let label = point.tpv.time().utc().format("%H:%M:%S").to_string();
        let response = ui.selectable_label(is_sticky, label);
        if response.hovered() {
            highlight.hover = Some(HighlightScope::Point(point_ref));
        }
        if response.clicked() {
            if !is_sticky {
                highlight.sticky = Some(point_ref);
                *popup_pos_request =
                    Some(egui::pos2(ui.clip_rect().max.x + 8.0, response.rect.min.y));
            } else {
                highlight.sticky = None;
            }
        }
        if response.double_clicked() {
            let lat = point.tpv.lat().get::<degree>();
            let lon = point.tpv.lon().get::<degree>();
            *map_center_request = Some((lat, lon));
        }
    }
}

fn render_satellite_report_items(
    ui: &mut egui::Ui,
    fi: usize,
    ti: usize,
    trip: &LoadedTrip,
    highlight: &mut MapHighlight,
    map_center_request: &mut Option<(f64, f64)>,
    popup_pos_request: &mut Option<egui::Pos2>,
) {
    for (pi, point) in trip.points.iter().enumerate() {
        let Some(sats) = &point.satellites else {
            continue;
        };
        let point_ref = DataPointRef {
            file_index: fi,
            trip_index: ti,
            category: DataCategory::SatelliteReport,
            point_index: pi,
        };
        let is_sticky = highlight.sticky.is_some_and(|r| r == point_ref);
        let time_str = sats
            .best_time()
            .map_or_else(|| "—".to_string(), |t| t.format("%H:%M:%S").to_string());
        let label = format!(
            "{time_str}  {}/{}",
            sats.fix_count(),
            sats.satellite_count()
        );
        let response = ui.selectable_label(is_sticky, label);
        if response.hovered() {
            highlight.hover = Some(HighlightScope::Point(point_ref));
        }
        if response.clicked() {
            if !is_sticky {
                highlight.sticky = Some(point_ref);
                *popup_pos_request =
                    Some(egui::pos2(ui.clip_rect().max.x + 8.0, response.rect.min.y));
            } else {
                highlight.sticky = None;
            }
        }
        if response.double_clicked() {
            let lat = point.tpv.lat().get::<degree>();
            let lon = point.tpv.lon().get::<degree>();
            *map_center_request = Some((lat, lon));
        }
    }
}

fn render_custom_marker_items(
    ui: &mut egui::Ui,
    fi: usize,
    ti: usize,
    trip: &LoadedTrip,
    highlight: &mut MapHighlight,
    map_center_request: &mut Option<(f64, f64)>,
    popup_pos_request: &mut Option<egui::Pos2>,
) {
    for (pi, marker) in trip.custom_markers.iter().enumerate() {
        let point_ref = DataPointRef {
            file_index: fi,
            trip_index: ti,
            category: DataCategory::CustomMarker,
            point_index: pi,
        };
        let is_sticky = highlight.sticky.is_some_and(|r| r == point_ref);
        let time_str = marker.time.format("%H:%M:%S").to_string();
        let label = format!("{time_str}  {}", marker.label);
        let response = ui.selectable_label(is_sticky, label);
        if response.hovered() {
            highlight.hover = Some(HighlightScope::Point(point_ref));
        }
        if response.clicked() {
            if !is_sticky {
                highlight.sticky = Some(point_ref);
                *popup_pos_request =
                    Some(egui::pos2(ui.clip_rect().max.x + 8.0, response.rect.min.y));
            } else {
                highlight.sticky = None;
            }
        }
        if response.double_clicked() {
            *map_center_request = Some((marker.lat.get::<degree>(), marker.lon.get::<degree>()));
        }
    }
}

fn render_generated_marker_items(
    ui: &mut egui::Ui,
    fi: usize,
    ti: usize,
    trip: &LoadedTrip,
    highlight: &mut MapHighlight,
    map_center_request: &mut Option<(f64, f64)>,
    popup_pos_request: &mut Option<egui::Pos2>,
) {
    use nav_types::GeneratedMarkerKind;
    for (pi, marker) in trip.generated_markers.iter().enumerate() {
        let point_ref = DataPointRef {
            file_index: fi,
            trip_index: ti,
            category: DataCategory::GeneratedMarker,
            point_index: pi,
        };
        let is_sticky = highlight.sticky.is_some_and(|r| r == point_ref);
        let time_str = marker.time.format("%H:%M:%S").to_string();
        let kind_str = match marker.kind {
            GeneratedMarkerKind::GpsFixLost => "GPS fix lost",
            GeneratedMarkerKind::GpsFixRegained => "GPS fix regained",
        };
        let label = format!("{time_str}  {kind_str}");
        let response = ui.selectable_label(is_sticky, label);
        if response.hovered() {
            highlight.hover = Some(HighlightScope::Point(point_ref));
        }
        if response.clicked() {
            if !is_sticky {
                highlight.sticky = Some(point_ref);
                *popup_pos_request =
                    Some(egui::pos2(ui.clip_rect().max.x + 8.0, response.rect.min.y));
            } else {
                highlight.sticky = None;
            }
        }
        if response.double_clicked() {
            *map_center_request = Some((marker.lat.get::<degree>(), marker.lon.get::<degree>()));
        }
    }
}

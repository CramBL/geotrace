use gt_fmt;
use gt_types::{
    DataCategory, DataPointRef, EventMarkerVisibility, FileIdx, GlobalFilter, HighlightScope,
    LoadedFile, LoadedTrip, MapHighlight, PointIdx, TripDataVisibility, TripIdx,
};
use uom::si::angle::degree;

use super::filter_panel::{FilterPanelState, render_filter_panel};
use super::trip_data_panel::{
    DeleteConfirmState, SelectionKey, TripDataPanelState, TripRef, apply_click,
};

/// Bundles the mutable state required to render the trip data side panel.
/// Avoids passing each field individually down the call chain.
pub struct PanelContext<'a> {
    pub files: &'a [LoadedFile],
    pub visibility: &'a mut TripDataVisibility,
    pub event_marker_visibility: &'a mut EventMarkerVisibility,
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
            let fi = FileIdx(fi);
            let file_enabled = vis.files.get(fi.0).is_some_and(|fv| fv.enabled);
            file.trips.iter().enumerate().filter_map(move |(ti, trip)| {
                let ti = TripIdx(ti);
                let trip_enabled = file_enabled
                    && vis
                        .files
                        .get(fi.0)
                        .and_then(|fv| fv.trips.get(ti.0))
                        .is_some_and(|tv| tv.enabled);
                let passes = gt_types::trip_passes_filter(&trip.metadata, &filter_snapshot);
                if !trip_enabled || !passes {
                    Some(SelectionKey::Trip(TripRef { file: fi, trip: ti }))
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
            let fi = FileIdx(fi);
            let mut keys = vec![SelectionKey::File(fi)];
            if ctx.panel.expanded_files.contains(&fi) {
                for ti in (0..file.trips.len()).map(TripIdx) {
                    keys.push(SelectionKey::Trip(TripRef { file: fi, trip: ti }));
                }
            }
            keys
        })
        .collect();

    egui::ScrollArea::vertical().show(ui, |ui| {
        for fi in (0..ctx.files.len()).map(FileIdx) {
            render_file_row(ui, fi, ctx, &ordered_keys);
        }
    });
}

fn render_file_row(
    ui: &mut egui::Ui,
    fi: FileIdx,
    ctx: &mut PanelContext<'_>,
    ordered_keys: &[SelectionKey],
) {
    let Some(file) = ctx.files.get(fi.0) else {
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
        let Some(file_vis) = ctx.visibility.files.get_mut(fi.0) else {
            return (ui.label("").clone(), false, false);
        };
        let chk = ui.checkbox(&mut file_vis.enabled, "");
        let cascade = chk.changed();
        let new_enabled = file_vis.enabled;
        let arrow = if is_expanded {
            egui_phosphor::regular::CARET_DOWN
        } else {
            egui_phosphor::regular::CARET_RIGHT
        };
        let dist = gt_fmt::format_distance(file.metadata.total_distance_km);
        let dur = gt_fmt::format_human_terse_duration(file.metadata.total_duration);
        let label = format!("{arrow} {}  {dist}  {dur}", file.metadata.filename);
        let text = egui::RichText::new(label);
        (
            ui.selectable_label(ctx.panel.selection.contains(&file_key), text)
                .on_hover_text(&file.metadata.filename),
            cascade,
            new_enabled,
        )
    });
    let (file_label_resp, file_cascade, file_new_enabled) = row_response.inner;
    if file_cascade && let Some(file_vis) = ctx.visibility.files.get_mut(fi.0) {
        for trip_vis in &mut file_vis.trips {
            trip_vis.enabled = file_new_enabled;
        }
    }
    if file_map_hovered {
        paint_map_hover_bg(ui, row_response.response.rect, map_hover_bg);
    }
    let modifiers = ui.ctx().input(|i| i.modifiers);
    if file_label_resp.clicked() {
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
    file_label_resp.context_menu(|ui| {
        if ui.button("Show only this file").clicked() {
            ctx.visibility.show_only_file(fi.0);
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
        ui.indent(format!("file_{}", fi.0), |ui| {
            render_file_trips(ui, fi, ctx, ordered_keys);
        });
    }
}

fn render_file_trips(
    ui: &mut egui::Ui,
    fi: FileIdx,
    ctx: &mut PanelContext<'_>,
    ordered_keys: &[SelectionKey],
) {
    let trip_count = ctx.files.get(fi.0).map_or(0, |f| f.trips.len());
    for ti in (0..trip_count).map(TripIdx) {
        render_trip_row(ui, fi, ti, ctx, ordered_keys);
    }
}

fn render_trip_row(
    ui: &mut egui::Ui,
    fi: FileIdx,
    ti: TripIdx,
    ctx: &mut PanelContext<'_>,
    ordered_keys: &[SelectionKey],
) {
    let (trip, passes, is_expanded, trip_panel_hovered, trip_map_hovered, key) = {
        let Some(file) = ctx.files.get(fi.0) else {
            return;
        };
        let Some(trip) = file.trips.get(ti.0) else {
            return;
        };
        let passes = gt_types::trip_passes_filter(&trip.metadata, ctx.filter);
        let trip_ref = TripRef { file: fi, trip: ti };
        let is_expanded = ctx.panel.expanded_trips.contains(&trip_ref);
        // Blue text: side-panel hover (mouse over trip name in the list).
        let trip_panel_hovered = ctx.highlight.hover.is_some_and(|s| {
            matches!(s, HighlightScope::Trip { file_index, trip_index }
                if file_index == fi && trip_index == ti)
        });
        // Yellow background: map hover (mouse over a TPV/marker on the map).
        let trip_map_hovered = ctx.highlight.hover.is_some_and(
            |s| matches!(s, HighlightScope::Point(r) if r.file_index == fi && r.trip_index == ti),
        );
        let key = SelectionKey::Trip(trip_ref);
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
        let Some(file_vis) = ctx.visibility.files.get_mut(fi.0) else {
            return (ui.label("").clone(), false);
        };
        let Some(trip_vis) = file_vis.trips.get_mut(ti.0) else {
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
        let dist = gt_fmt::format_distance(trip.metadata.distance_km);
        let dur = gt_fmt::format_human_terse_duration(trip.metadata.duration);
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
            let trip_ref = TripRef { file: fi, trip: ti };
            if is_expanded {
                ctx.panel.expanded_trips.remove(&trip_ref);
            } else {
                ctx.panel.expanded_trips.insert(trip_ref);
            }
            apply_click(ctx.panel, key.clone(), false, false, ordered_keys);
        }
    }
    response.context_menu(|ui| {
        if ui.button("Show only this trip").clicked() {
            ctx.visibility.show_only_trip(fi.0, ti.0);
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
        ui.indent(format!("trip_{}_{}", fi.0, ti.0), |ui| {
            render_trip_categories(ui, fi, ti, &trip, ctx);
        });
    }
}

/// Renders one expandable data-category row (checkbox + caret label + optional
/// indented item list) and sets the hover highlight when the header is hovered.
///
/// Returns without rendering when `count == 0`.
#[expect(
    clippy::too_many_arguments,
    reason = "each argument plays a distinct role; no natural grouping reduces the count without inventing a one-off struct"
)]
fn render_category_section(
    ui: &mut egui::Ui,
    fi: FileIdx,
    ti: TripIdx,
    cat: DataCategory,
    count: usize,
    label: &str,
    visible: &mut bool,
    panel: &mut TripDataPanelState,
    highlight: &mut MapHighlight,
    render_items: impl FnOnce(&mut egui::Ui, &mut MapHighlight),
) {
    if count == 0 {
        return;
    }
    let trip_ref = TripRef { file: fi, trip: ti };
    let expanded = panel.expanded_categories.contains(&(trip_ref, cat));
    let header = ui.horizontal(|ui| {
        ui.checkbox(visible, "");
        let arrow = if expanded {
            egui_phosphor::regular::CARET_DOWN
        } else {
            egui_phosphor::regular::CARET_RIGHT
        };
        let resp = ui.selectable_label(expanded, format!("{arrow} {label}  {count}"));
        if resp.clicked() {
            toggle_category(panel, fi, ti, cat);
        }
        resp
    });
    if header.inner.hovered() {
        highlight.hover = Some(HighlightScope::TripCategory {
            file_index: fi,
            trip_index: ti,
            category: cat,
        });
    }
    if expanded {
        ui.indent((cat, trip_ref), |ui| {
            render_items(ui, highlight);
        });
    }
}

fn render_trip_categories(
    ui: &mut egui::Ui,
    fi: FileIdx,
    ti: TripIdx,
    trip: &LoadedTrip,
    ctx: &mut PanelContext<'_>,
) {
    let Some(file_vis) = ctx.visibility.files.get_mut(fi.0) else {
        return;
    };
    let Some(trip_vis) = file_vis.trips.get_mut(ti.0) else {
        return;
    };

    // TripTrack has no expandable items — just a checkbox with a hover highlight.
    let track_resp = ui.horizontal(|ui| {
        ui.checkbox(&mut trip_vis.track_visible, "");
        ui.label("Trip track")
    });
    if track_resp.inner.hovered() {
        ctx.highlight.hover = Some(HighlightScope::TripCategory {
            file_index: fi,
            trip_index: ti,
            category: DataCategory::TripTrack,
        });
    }

    render_category_section(
        ui,
        fi,
        ti,
        DataCategory::Tpv,
        trip.points.len(),
        "Trip points",
        &mut trip_vis.tpv_visible,
        ctx.panel,
        ctx.highlight,
        |ui, highlight| {
            render_tpv_items(
                ui,
                fi,
                ti,
                trip,
                highlight,
                ctx.map_center_request,
                ctx.popup_pos_request,
            )
        },
    );

    let sat_count = trip
        .points
        .iter()
        .filter(|p| p.satellites.is_some())
        .count();
    render_category_section(
        ui,
        fi,
        ti,
        DataCategory::SatelliteReport,
        sat_count,
        "Satellite reports",
        &mut trip_vis.satellites_visible,
        ctx.panel,
        ctx.highlight,
        |ui, highlight| {
            render_satellite_report_items(
                ui,
                fi,
                ti,
                trip,
                highlight,
                ctx.map_center_request,
                ctx.popup_pos_request,
            )
        },
    );

    render_category_section(
        ui,
        fi,
        ti,
        DataCategory::CustomMarker,
        trip.custom_markers.len(),
        "Custom markers",
        &mut trip_vis.custom_markers_visible,
        ctx.panel,
        ctx.highlight,
        |ui, highlight| {
            render_custom_marker_items(
                ui,
                fi,
                ti,
                trip,
                highlight,
                ctx.map_center_request,
                ctx.popup_pos_request,
            )
        },
    );

    render_category_section(
        ui,
        fi,
        ti,
        DataCategory::GeneratedMarker,
        trip.generated_markers.len(),
        "Generated markers",
        &mut trip_vis.generated_markers_visible,
        ctx.panel,
        ctx.highlight,
        |ui, highlight| {
            render_generated_marker_items(
                ui,
                fi,
                ti,
                trip,
                highlight,
                ctx.map_center_request,
                ctx.popup_pos_request,
            )
        },
    );

    if !trip.event_markers.is_empty() {
        let trip_ref = super::trip_data_panel::TripRef { file: fi, trip: ti };
        render_event_markers_section(ui, fi, ti, trip, trip_ref, ctx);
    }
}

fn render_event_markers_section(
    ui: &mut egui::Ui,
    fi: FileIdx,
    ti: TripIdx,
    trip: &LoadedTrip,
    trip_ref: super::trip_data_panel::TripRef,
    ctx: &mut PanelContext<'_>,
) {
    let Some(file_vis) = ctx.visibility.files.get_mut(fi.0) else {
        return;
    };
    let Some(trip_vis) = file_vis.trips.get_mut(ti.0) else {
        return;
    };

    let count = trip.event_markers.len();
    let header_id = egui::Id::new(("events_section", fi, ti));
    let is_open = ctx
        .panel
        .expanded_categories
        .contains(&(trip_ref, DataCategory::EventMarker));

    let header_response = ui.horizontal(|ui| {
        ui.checkbox(&mut trip_vis.event_markers_visible, "");
        let arrow = if is_open {
            egui_phosphor::regular::CARET_DOWN
        } else {
            egui_phosphor::regular::CARET_RIGHT
        };
        let label = format!("{arrow} Events  {count}");
        ui.selectable_label(false, label)
    });

    if header_response.inner.clicked() {
        if is_open {
            ctx.panel
                .expanded_categories
                .remove(&(trip_ref, DataCategory::EventMarker));
        } else {
            ctx.panel
                .expanded_categories
                .insert((trip_ref, DataCategory::EventMarker));
        }
    }

    if !is_open {
        return;
    }

    // We need immutable refs to event_marker_visibility for this call,
    // but we've already taken a mutable borrow on visibility above.
    // Drop it before re-borrowing event_marker_visibility.
    let _ = file_vis;

    let filter_text = ctx
        .panel
        .event_marker_filter
        .entry(trip_ref)
        .or_default()
        .clone();

    ui.horizontal(|ui| {
        ui.add_space(16.0);
        let mut text = filter_text.clone();
        let resp = ui.add(
            egui::TextEdit::singleline(&mut text)
                .hint_text("Filter…")
                .desired_width(120.0)
                .id(egui::Id::new(("event_filter", header_id))),
        );
        if resp.changed() {
            ctx.panel.event_marker_filter.insert(trip_ref, text.clone());
        }
        if !text.is_empty() && ui.small_button("×").clicked() {
            ctx.panel
                .event_marker_filter
                .insert(trip_ref, String::new());
        }
    });

    let filter_text = ctx
        .panel
        .event_marker_filter
        .get(&trip_ref)
        .cloned()
        .unwrap_or_default();

    // Collect all unique variant paths and build the tree structure.
    let mut paths: Vec<&str> = trip
        .event_markers
        .iter()
        .map(|m| m.variant_path.as_str())
        .collect();
    paths.sort_unstable();
    paths.dedup();

    // Apply prefix filter.
    let filtered: Vec<&str> = if filter_text.is_empty() {
        paths
    } else {
        paths
            .into_iter()
            .filter(|p| p.contains(filter_text.as_str()))
            .collect()
    };

    // Build a prefix tree: collect all unique prefixes.
    let mut prefix_set: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for path in &filtered {
        let segments: Vec<&str> = path.split('/').collect();
        for depth in 1..=segments.len() {
            if let Some(slice) = segments.get(..depth) {
                prefix_set.insert(slice.join("/"));
            }
        }
    }

    egui::ScrollArea::vertical()
        .max_height(200.0)
        .id_salt(egui::Id::new(("events_scroll", header_id)))
        .show(ui, |ui| {
            for prefix in &prefix_set {
                let depth = prefix.chars().filter(|&c| c == '/').count();
                let segment = prefix.split('/').next_back().unwrap_or(prefix.as_str());
                let marker_count = trip
                    .event_markers
                    .iter()
                    .filter(|m| {
                        m.variant_path == *prefix
                            || m.variant_path.starts_with(&format!("{prefix}/"))
                    })
                    .count();

                let mut visible = ctx.event_marker_visibility.is_visible(fi.0, ti.0, prefix);

                ui.horizontal(|ui| {
                    ui.add_space(16.0 + depth as f32 * 12.0);
                    let resp = ui.checkbox(&mut visible, "");
                    if resp.changed() {
                        if visible {
                            ctx.event_marker_visibility
                                .set_visible_cascade(fi.0, ti.0, prefix);
                        } else {
                            ctx.event_marker_visibility
                                .set_hidden_cascade(fi.0, ti.0, prefix);
                        }
                    }
                    ui.label(format!("{segment}  {marker_count}"));
                });
            }
        });
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

fn toggle_category(panel: &mut TripDataPanelState, fi: FileIdx, ti: TripIdx, cat: DataCategory) {
    let key = (TripRef { file: fi, trip: ti }, cat);
    if panel.expanded_categories.contains(&key) {
        panel.expanded_categories.remove(&key);
    } else {
        panel.expanded_categories.insert(key);
    }
}

fn render_tpv_items(
    ui: &mut egui::Ui,
    fi: FileIdx,
    ti: TripIdx,
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
            point_index: PointIdx(pi),
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
    fi: FileIdx,
    ti: TripIdx,
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
            point_index: PointIdx(pi),
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
    fi: FileIdx,
    ti: TripIdx,
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
            point_index: PointIdx(pi),
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
    fi: FileIdx,
    ti: TripIdx,
    trip: &LoadedTrip,
    highlight: &mut MapHighlight,
    map_center_request: &mut Option<(f64, f64)>,
    popup_pos_request: &mut Option<egui::Pos2>,
) {
    use gt_types::GeneratedMarkerKind;
    for (pi, marker) in trip.generated_markers.iter().enumerate() {
        let point_ref = DataPointRef {
            file_index: fi,
            trip_index: ti,
            category: DataCategory::GeneratedMarker,
            point_index: PointIdx(pi),
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

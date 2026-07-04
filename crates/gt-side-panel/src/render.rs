use gt_filter::GlobalFilter;
use gt_loaded_files::LoadedFilesView;
use gt_types::{
    DataCategory, FileIdx, GeneratedMarkerKind, LoadWarning, LoadedFile, LoadedTrack, PointIdx,
    TrackIdx, TrackRef,
};
use gt_ui_types::{DataPointRef, HighlightScope, MapHighlight};

use crate::filter::{FilterPanelState, render_filter_panel};
use crate::tree::{CheckState, DeleteConfirmState, NodeKey, TreeState};
use crate::widgets::{
    expand_arrow, fix_stats_tooltip_row, paint_map_hover_bg, point_item_row, tri_checkbox,
};

pub struct PanelContext<'a> {
    pub loaded_files: LoadedFilesView<'a>,
    pub tree: &'a mut TreeState,
    pub highlight: &'a mut MapHighlight,
    pub filter: &'a mut GlobalFilter,
    pub filter_state: &'a mut FilterPanelState,
    pub map_center_request: &'a mut Option<(f64, f64)>,
    pub popup_pos_request: &'a mut Option<egui::Pos2>,
    pub zoom_to_visible_request: &'a mut bool,
    /// Set by clicking the ⚠ icon on a file row. Consumed by the app to show a centered dialog.
    pub warnings_request: &'a mut Option<(String, Vec<LoadWarning>)>,
}

impl<'a> PanelContext<'a> {
    fn files(&self) -> &'a [LoadedFile] {
        self.loaded_files.files()
    }

    fn file(&self, file: FileIdx) -> Option<&'a LoadedFile> {
        self.loaded_files.entry_for(file).map(|entry| entry.file())
    }

    fn file_stored_in_history(&self, file: FileIdx) -> bool {
        self.loaded_files.file_stored_in_history(file)
    }
}

pub fn show_side_panel(ui: &mut egui::Ui, ctx: &mut PanelContext<'_>) {
    let header = ui.horizontal(|ui| {
        let (_, grip) = ui.allocate_exact_size(egui::vec2(10.0, 18.0), egui::Sense::drag());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ctx.tree.detached {
                if ui.small_button("Dock").clicked() {
                    ctx.tree.detached = false;
                }
            } else if ui
                .small_button(egui_phosphor::regular::ARROW_SQUARE_OUT)
                .on_hover_text("Pop out")
                .clicked()
            {
                ctx.tree.detached = true;
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
        ctx.tree.detached = true;
    }

    ui.separator();
    render_filter_panel(ui, ctx.files(), ctx.filter, ctx.filter_state);

    let filter_snapshot = *ctx.filter;
    let vis = ctx.tree.visibility();
    let filtered_out: Vec<NodeKey> = ctx
        .files()
        .iter()
        .enumerate()
        .flat_map(|(fi, file)| {
            let fi = FileIdx::new(fi);
            let file_enabled = fi.get(&vis.files).is_some_and(|fv| fv.enabled);
            file.tracks
                .iter()
                .enumerate()
                .filter_map(move |(ti, track)| {
                    let ti = TrackIdx::new(ti);
                    let track_enabled = file_enabled
                        && fi
                            .get(&vis.files)
                            .and_then(|fv| ti.get(&fv.tracks))
                            .is_some_and(|tv| tv.enabled);
                    let passes = gt_filter::track_passes_filter(&track.metadata, &filter_snapshot);
                    if !track_enabled || !passes {
                        Some(NodeKey::Track(TrackRef::new(fi, ti)))
                    } else {
                        None
                    }
                })
        })
        .collect();
    let has_filtered = !filtered_out.is_empty();
    let clicked = ui
        .scope(|ui| {
            if has_filtered {
                let v = ui.visuals_mut();
                v.widgets.hovered.bg_fill = gt_ui_theme::DANGER_HOVER;
                v.widgets.hovered.fg_stroke.color = gt_ui_theme::DANGER_FG;
                v.widgets.active.bg_fill = gt_ui_theme::DANGER_ACTIVE;
                v.widgets.active.fg_stroke.color = gt_ui_theme::DANGER_FG;
            }
            ui.add_enabled(
                has_filtered,
                egui::Button::new(format!(
                    "{} Remove filtered data",
                    egui_phosphor::regular::TRASH
                )),
            )
            .clicked()
        })
        .inner;
    if clicked {
        ctx.tree.delete_confirm = Some(DeleteConfirmState {
            items: filtered_out,
            delete_permanently: false,
        });
    }

    ui.separator();

    ui.horizontal(|ui| {
        if ui.small_button("Show all").clicked() {
            ctx.tree.set_all_enabled(true);
            *ctx.zoom_to_visible_request = true;
        }
        if ui.small_button("Hide all").clicked() {
            ctx.tree.set_all_enabled(false);
        }
    });

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for fi in 0..ctx.files().len() {
                render_file_row(ui, FileIdx::new(fi), ctx);
            }
        });
}

fn render_file_row(ui: &mut egui::Ui, fi: FileIdx, ctx: &mut PanelContext<'_>) {
    let Some(file) = ctx.file(fi) else {
        return;
    };
    let Some(file_node) = ctx.tree.file_node(fi) else {
        return;
    };
    let is_expanded = file_node.expanded;
    let check = file_node.check;
    let file_key = NodeKey::File(fi);

    let file_map_hovered = ctx.highlight.hover.is_some_and(|s| match s {
        HighlightScope::Point(r) => r.track.fi == fi,
        HighlightScope::Track(track) | HighlightScope::TrackCategory { track, .. } => {
            track.fi == fi
        }
        HighlightScope::File { file_index } => file_index == fi,
    });

    let map_hover_bg = gt_ui_theme::map_hover_color(ui.visuals().dark_mode);

    let row_response = ui.horizontal(|ui| {
        let chk_resp = tri_checkbox(ui, check);
        if chk_resp.clicked() {
            ctx.tree.toggle_file_check(fi);
        }
        let arrow = expand_arrow(is_expanded);
        let dist = gt_fmt::format_distance(file.metadata.total_distance_km);
        let dur = gt_fmt::format_human_terse_duration(file.metadata.total_duration);
        let label = format!("{arrow} {}  {dist}  {dur}", file.metadata.filename);
        let is_selected = ctx.tree.selection.contains(&file_key);
        let resp = ui.selectable_label(is_selected, egui::RichText::new(label));
        let resp = if let Some(stats) = file.metadata.fix_stats {
            resp.on_hover_ui(|ui| {
                ui.label(file.metadata.filename.as_str());
                fix_stats_tooltip_row(ui, stats);
            })
        } else {
            resp.on_hover_text(file.metadata.filename.as_str())
        };
        if !file.load_warnings.is_empty() {
            let icon = egui::RichText::new(egui_phosphor::regular::WARNING)
                .color(gt_ui_theme::WARNING_AMBER);
            if ui
                .add(egui::Label::new(icon).sense(egui::Sense::click()))
                .on_hover_text("Data quality warnings - click for details")
                .clicked()
            {
                *ctx.warnings_request =
                    Some((file.metadata.filename.clone(), file.load_warnings.clone()));
            }
        }
        resp
    });

    let file_label_resp = row_response.inner;
    if file_map_hovered {
        paint_map_hover_bg(ui, row_response.response.rect, map_hover_bg);
    }
    if file_label_resp.hovered() {
        ctx.highlight.hover = Some(HighlightScope::File { file_index: fi });
    }
    let modifiers = ui.ctx().input(|i| i.modifiers);
    if file_label_resp.double_clicked() {
        if let Some(center) = file_bounding_center(ctx.file(fi)) {
            *ctx.map_center_request = Some(center);
        }
    } else if file_label_resp.clicked() {
        if modifiers.ctrl || modifiers.shift {
            ctx.tree
                .apply_click(file_key, modifiers.ctrl, modifiers.shift);
        } else {
            ctx.tree.toggle_expand_file(fi);
            ctx.tree.apply_click(file_key, false, false);
        }
    }
    file_label_resp.context_menu(|ui| {
        if ui.button("Show only this file").clicked() {
            ctx.tree.show_only_file(fi);
            ui.close();
        }
        ui.separator();
        let stored_in_history = ctx.file_stored_in_history(fi);
        let unload = ui.button("Unload").on_hover_text(if stored_in_history {
            "Unloads this recording from the view; it stays in History"
        } else {
            "Unloads this file from the current view"
        });
        if unload.clicked() {
            ctx.tree.pending_unload = Some(vec![file_key]);
            ui.close();
        }
        if ctx.tree.selection.len() >= 2 && ui.button("Unload selected").clicked() {
            ctx.tree.pending_unload = Some(ctx.tree.selection.iter().cloned().collect());
            ui.close();
        }
    });

    if is_expanded {
        ui.indent(format!("file_{fi}"), |ui| {
            let track_count = ctx.file(fi).map_or(0, |f| f.tracks.len());
            for ti in 0..track_count {
                render_track_row(ui, fi, TrackIdx::new(ti), ctx);
            }
        });
    }
}

fn render_track_row(ui: &mut egui::Ui, fi: FileIdx, ti: TrackIdx, ctx: &mut PanelContext<'_>) {
    let track_ref = TrackRef::new(fi, ti);
    let (track, passes, is_expanded, panel_hovered, map_hovered, key) = {
        let Some(file) = ctx.file(fi) else {
            return;
        };
        let Some(track) = ti.get(&file.tracks) else {
            return;
        };
        let passes = gt_filter::track_passes_filter(&track.metadata, ctx.filter);
        let is_expanded = ctx.tree.track_node(track_ref).is_some_and(|t| t.expanded);
        let panel_hovered = ctx
            .highlight
            .hover
            .is_some_and(|s| matches!(s, HighlightScope::Track(t) if t == track_ref));
        let map_hovered = ctx
            .highlight
            .hover
            .is_some_and(|s| matches!(s, HighlightScope::Point(r) if r.track == track_ref));
        let key = NodeKey::Track(track_ref);
        (
            track.clone(),
            passes,
            is_expanded,
            panel_hovered,
            map_hovered,
            key,
        )
    };

    let was_all_hidden = ctx.tree.all_hidden();
    let map_hover_bg = gt_ui_theme::map_hover_color(ui.visuals().dark_mode);

    let check = ctx
        .tree
        .track_node(track_ref)
        .map_or(CheckState::On, |t| t.check);

    let row_response = ui.horizontal(|ui| {
        let chk_resp = tri_checkbox(ui, check);
        if chk_resp.clicked() {
            ctx.tree.toggle_track_check(track_ref);
        }
        let newly_enabled =
            chk_resp.clicked() && matches!(check, CheckState::Off | CheckState::Mixed);
        let arrow = expand_arrow(is_expanded);
        let dist = gt_fmt::format_distance(track.metadata.distance_km);
        let dur = gt_fmt::format_human_terse_duration(track.metadata.duration);
        let label = format!("{arrow} #{}  {dist}  {dur}", track.metadata.index);
        let mut text = egui::RichText::new(label);
        if !passes {
            text = text.weak();
        }
        if panel_hovered {
            text = text.color(gt_ui_theme::HIGHLIGHT_BLUE);
        }
        let is_selected = ctx.tree.selection.contains(&key);
        let resp = ui.selectable_label(is_selected, text);
        let time_header = gt_fmt::format_time_range(
            track.metadata.time_range.start,
            track.metadata.time_range.end,
        );
        let fix_stats = track.metadata.fix_stats;
        let resp = resp.on_hover_ui(|ui| {
            ui.label(egui::RichText::new(&time_header).strong());
            match fix_stats {
                Some(stats) => fix_stats_tooltip_row(ui, stats),
                None => {
                    ui.label("No satellite data");
                }
            }
        });
        (resp, newly_enabled)
    });

    if map_hovered {
        paint_map_hover_bg(ui, row_response.response.rect, map_hover_bg);
    }
    let (response, newly_enabled) = row_response.inner;
    if newly_enabled && was_all_hidden {
        *ctx.zoom_to_visible_request = true;
    }
    if response.hovered() {
        ctx.highlight.hover = Some(HighlightScope::Track(track_ref));
    }
    let modifiers = ui.ctx().input(|i| i.modifiers);
    if response.double_clicked() {
        let bb = track.metadata.bounding_box;
        let center_lat = (bb.min().y + bb.max().y) / 2.0;
        let center_lon = (bb.min().x + bb.max().x) / 2.0;
        *ctx.map_center_request = Some((center_lat, center_lon));
    } else if response.clicked() {
        if modifiers.ctrl || modifiers.shift {
            ctx.tree.apply_click(key, modifiers.ctrl, modifiers.shift);
        } else {
            ctx.tree.toggle_expand_track(track_ref);
            ctx.tree.apply_click(key, false, false);
        }
    }
    response.context_menu(|ui| {
        if ui.button("Show only this track").clicked() {
            ctx.tree.show_only_track(track_ref);
            *ctx.zoom_to_visible_request = true;
            ui.close();
        }
        ui.separator();
        if ui.button("Unload").clicked() {
            ctx.tree.pending_unload = Some(vec![key]);
            ui.close();
        }
        if ctx.tree.selection.len() >= 2 && ui.button("Unload selected").clicked() {
            ctx.tree.pending_unload = Some(ctx.tree.selection.iter().cloned().collect());
            ui.close();
        }
    });

    if is_expanded {
        ui.indent(format!("track_{fi}_{ti}"), |ui| {
            render_track_categories(ui, track_ref, &track, ctx);
        });
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "all arguments are distinct; extracting a context struct avoids re-borrowing tree mid-render"
)]
fn render_category_section(
    ui: &mut egui::Ui,
    track_ref: TrackRef,
    cat: DataCategory,
    count: usize,
    label: &str,
    visible: bool,
    expanded: bool,
    tree: &mut TreeState,
    highlight: &mut MapHighlight,
    render_items: impl FnOnce(&mut egui::Ui, &mut MapHighlight),
) {
    if count == 0 {
        return;
    }
    let header = ui.horizontal(|ui| {
        let chk = tri_checkbox(
            ui,
            if visible {
                CheckState::On
            } else {
                CheckState::Off
            },
        );
        if chk.clicked() {
            tree.set_category_visible(track_ref, cat, !visible);
        }
        let arrow = expand_arrow(expanded);
        let resp = ui.selectable_label(expanded, format!("{arrow} {label}  {count}"));
        if resp.clicked() {
            tree.toggle_category_expanded(track_ref, cat);
        }
        resp
    });
    if header.inner.hovered() {
        highlight.hover = Some(HighlightScope::TrackCategory {
            track: track_ref,
            category: cat,
        });
    }
    if expanded {
        ui.indent((cat, track_ref), |ui| {
            render_items(ui, highlight);
        });
    }
}

fn render_track_categories(
    ui: &mut egui::Ui,
    track_ref: TrackRef,
    track: &LoadedTrack,
    ctx: &mut PanelContext<'_>,
) {
    let Some(track_node) = ctx.tree.track_node(track_ref) else {
        return;
    };
    let track_visible = track_node.track_visible;
    let tpv_visible = track_node.tpv_visible;
    let sat_visible = track_node.satellites_visible;
    let cm_visible = track_node.custom_markers_visible;
    let tpv_expanded = track_node.categories_expanded.contains(&DataCategory::Tpv);
    let sat_expanded = track_node
        .categories_expanded
        .contains(&DataCategory::SatelliteReport);
    let cm_expanded = track_node
        .categories_expanded
        .contains(&DataCategory::CustomMarker);
    let em_expanded = track_node
        .categories_expanded
        .contains(&DataCategory::EventMarker);
    let em_agg = track_node.event_paths.aggregate();
    let event_filter = track_node.event_filter.clone();

    let track_resp = ui.horizontal(|ui| {
        let chk = tri_checkbox(
            ui,
            if track_visible {
                CheckState::On
            } else {
                CheckState::Off
            },
        );
        if chk.clicked() {
            ctx.tree
                .set_category_visible(track_ref, DataCategory::Track, !track_visible);
        }
        ui.label("Track polyline")
    });
    if track_resp.inner.hovered() {
        ctx.highlight.hover = Some(HighlightScope::TrackCategory {
            track: track_ref,
            category: DataCategory::Track,
        });
    }

    render_category_section(
        ui,
        track_ref,
        DataCategory::Tpv,
        track.points.len(),
        "Track points",
        tpv_visible,
        tpv_expanded,
        ctx.tree,
        ctx.highlight,
        |ui, highlight| {
            render_tpv_items(
                ui,
                track_ref,
                track,
                highlight,
                ctx.map_center_request,
                ctx.popup_pos_request,
            );
        },
    );

    let sat_count = track
        .points
        .iter()
        .filter(|p| p.satellites.is_some())
        .count();
    render_category_section(
        ui,
        track_ref,
        DataCategory::SatelliteReport,
        sat_count,
        "Satellite reports",
        sat_visible,
        sat_expanded,
        ctx.tree,
        ctx.highlight,
        |ui, highlight| {
            render_satellite_report_items(
                ui,
                track_ref,
                track,
                highlight,
                ctx.map_center_request,
                ctx.popup_pos_request,
            );
        },
    );

    render_category_section(
        ui,
        track_ref,
        DataCategory::CustomMarker,
        track.custom_markers.len(),
        "Custom markers",
        cm_visible,
        cm_expanded,
        ctx.tree,
        ctx.highlight,
        |ui, highlight| {
            render_custom_marker_items(
                ui,
                track_ref,
                track,
                highlight,
                ctx.map_center_request,
                ctx.popup_pos_request,
            );
        },
    );

    render_generated_markers_section(ui, track_ref, track, ctx);

    if !track.event_markers.is_empty() {
        render_event_markers_section(
            ui,
            track_ref,
            track,
            em_agg,
            em_expanded,
            &event_filter,
            ctx,
        );
    }
}

fn render_event_markers_section(
    ui: &mut egui::Ui,
    track_ref: TrackRef,
    track: &LoadedTrack,
    em_agg: CheckState,
    is_open: bool,
    filter_text: &str,
    ctx: &mut PanelContext<'_>,
) {
    let count = track.event_markers.len();
    let header_response = ui.horizontal(|ui| {
        let chk_resp = tri_checkbox(ui, em_agg);
        if chk_resp.clicked() {
            ctx.tree.toggle_all_event_paths(track_ref);
        }
        let arrow = expand_arrow(is_open);
        let label = format!("{arrow} Events  {count}");
        ui.selectable_label(false, label)
    });

    if header_response.inner.clicked() {
        ctx.tree
            .toggle_category_expanded(track_ref, DataCategory::EventMarker);
    }
    if header_response.inner.hovered() {
        ctx.highlight.hover = Some(HighlightScope::TrackCategory {
            track: track_ref,
            category: DataCategory::EventMarker,
        });
    }

    if !is_open {
        return;
    }

    let header_id = egui::Id::new(("events_section", track_ref));

    ui.horizontal(|ui| {
        ui.add_space(16.0);
        let mut text = filter_text.to_owned();
        let resp = ui.add(
            egui::TextEdit::singleline(&mut text)
                .hint_text("Filter…")
                .desired_width(120.0)
                .id(egui::Id::new(("event_filter", header_id))),
        );
        if resp.changed()
            && let Some(track_node) = ctx.tree.track_node_mut(track_ref)
        {
            track_node.event_filter = text.clone();
        }
        if !text.is_empty()
            && ui.small_button("×").clicked()
            && let Some(track_node) = ctx.tree.track_node_mut(track_ref)
        {
            track_node.event_filter.clear();
        }
    });

    let current_filter = ctx
        .tree
        .track_node(track_ref)
        .map_or("", |t| t.event_filter.as_str());

    let mut paths: Vec<&str> = track
        .event_markers
        .iter()
        .map(|m| m.variant_path.as_str())
        .collect();
    paths.sort_unstable();
    paths.dedup();

    let filtered: Vec<&str> = if current_filter.is_empty() {
        paths
    } else {
        paths
            .into_iter()
            .filter(|p| p.contains(current_filter))
            .collect()
    };

    let mut prefix_set: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for path in &filtered {
        let segments: Vec<&str> = path.split('/').collect();
        for depth in 1..=segments.len() {
            if let Some(slice) = segments.get(..depth) {
                prefix_set.insert(slice.join("/"));
            }
        }
    }

    // No max_height cap - expands inline with the track's content.
    for prefix in &prefix_set {
        let depth = prefix.chars().filter(|&c| c == '/').count();
        let segment = prefix.split('/').next_back().unwrap_or(prefix.as_str());
        let marker_count = track
            .event_markers
            .iter()
            .filter(|m| {
                m.variant_path == *prefix || m.variant_path.starts_with(&format!("{prefix}/"))
            })
            .count();

        let node_check = ctx
            .tree
            .track_node(track_ref)
            .and_then(|t| t.event_paths.nodes.get(prefix.as_str()).copied())
            .unwrap_or(CheckState::On);

        ui.horizontal(|ui| {
            ui.add_space(16.0 + depth as f32 * 12.0);
            let chk_resp = tri_checkbox(ui, node_check);
            if chk_resp.clicked() {
                ctx.tree.toggle_event_path(track_ref, prefix);
            }
            ui.label(format!("{segment}  {marker_count}"));
        });
    }
}

fn render_tpv_items(
    ui: &mut egui::Ui,
    track_ref: TrackRef,
    track: &LoadedTrack,
    highlight: &mut MapHighlight,
    map_center_request: &mut Option<(f64, f64)>,
    popup_pos_request: &mut Option<egui::Pos2>,
) {
    for (pi, point) in track.points.iter().enumerate() {
        let point_ref = DataPointRef {
            track: track_ref,
            category: DataCategory::Tpv,
            point_index: PointIdx::new(pi),
        };
        let label = point.tpv.time().utc().format("%H:%M:%S").to_string();
        let lat_lon = (point.tpv.lat().as_degrees(), point.tpv.lon().as_degrees());
        point_item_row(
            ui,
            point_ref,
            label,
            lat_lon,
            highlight,
            map_center_request,
            popup_pos_request,
        );
    }
}

fn render_satellite_report_items(
    ui: &mut egui::Ui,
    track_ref: TrackRef,
    track: &LoadedTrack,
    highlight: &mut MapHighlight,
    map_center_request: &mut Option<(f64, f64)>,
    popup_pos_request: &mut Option<egui::Pos2>,
) {
    for (pi, point) in track.points.iter().enumerate() {
        let Some(sats) = &point.satellites else {
            continue;
        };
        let point_ref = DataPointRef {
            track: track_ref,
            category: DataCategory::SatelliteReport,
            point_index: PointIdx::new(pi),
        };
        let time_str = sats.best_time().map_or_else(
            || gt_ui_theme::EM_DASH.to_string(),
            |t| t.format("%H:%M:%S").to_string(),
        );
        let label = format!(
            "{time_str}  {}/{}",
            sats.fix_count(),
            sats.satellite_count()
        );
        let lat_lon = (point.tpv.lat().as_degrees(), point.tpv.lon().as_degrees());
        point_item_row(
            ui,
            point_ref,
            label,
            lat_lon,
            highlight,
            map_center_request,
            popup_pos_request,
        );
    }
}

fn render_custom_marker_items(
    ui: &mut egui::Ui,
    track_ref: TrackRef,
    track: &LoadedTrack,
    highlight: &mut MapHighlight,
    map_center_request: &mut Option<(f64, f64)>,
    popup_pos_request: &mut Option<egui::Pos2>,
) {
    for (pi, marker) in track.custom_markers.iter().enumerate() {
        let point_ref = DataPointRef {
            track: track_ref,
            category: DataCategory::CustomMarker,
            point_index: PointIdx::new(pi),
        };
        let label = format!("{}  {}", marker.time.format("%H:%M:%S"), marker.label);
        let lat_lon = (marker.lat.as_degrees(), marker.lon.as_degrees());
        point_item_row(
            ui,
            point_ref,
            label,
            lat_lon,
            highlight,
            map_center_request,
            popup_pos_request,
        );
    }
}

/// Render the "Generated markers" section as a tree: a category header (master
/// show/hide + expand) over one collapsible, individually toggleable group per
/// event type, with the markers of each type beneath their group.
fn render_generated_markers_section(
    ui: &mut egui::Ui,
    track_ref: TrackRef,
    track: &LoadedTrack,
    ctx: &mut PanelContext<'_>,
) {
    let count = track.generated_markers.len();
    if count == 0 {
        return;
    }
    let Some(node) = ctx.tree.track_node(track_ref) else {
        return;
    };
    let visible = node.generated_markers_visible;
    let expanded = node
        .categories_expanded
        .contains(&DataCategory::GeneratedMarker);

    let header = ui.horizontal(|ui| {
        let chk = tri_checkbox(
            ui,
            if visible {
                CheckState::On
            } else {
                CheckState::Off
            },
        );
        if chk.clicked() {
            ctx.tree
                .set_category_visible(track_ref, DataCategory::GeneratedMarker, !visible);
        }
        let arrow = expand_arrow(expanded);
        ui.selectable_label(expanded, format!("{arrow} Generated markers  {count}"))
    });
    if header.inner.clicked() {
        ctx.tree
            .toggle_category_expanded(track_ref, DataCategory::GeneratedMarker);
    }
    if header.inner.hovered() {
        ctx.highlight.hover = Some(HighlightScope::TrackCategory {
            track: track_ref,
            category: DataCategory::GeneratedMarker,
        });
    }
    if !expanded {
        return;
    }

    // Group markers by event type, ordered by the tag's variant order.
    let mut groups: std::collections::BTreeMap<gt_types::GeneratedMarkerKindTag, Vec<usize>> =
        std::collections::BTreeMap::new();
    for (pi, marker) in track.generated_markers.iter().enumerate() {
        groups.entry(marker.kind.tag()).or_default().push(pi);
    }

    ui.indent((DataCategory::GeneratedMarker, track_ref), |ui| {
        for (&tag, indices) in &groups {
            let tag_count = indices.len();
            let tag_hidden = !ctx.tree.generated_kind_visible(track_ref, tag);
            let tag_expanded = ctx.tree.generated_kind_expanded(track_ref, tag);

            let row = ui.horizontal(|ui| {
                let chk = tri_checkbox(
                    ui,
                    if tag_hidden {
                        CheckState::Off
                    } else {
                        CheckState::On
                    },
                );
                if chk.clicked() {
                    ctx.tree.toggle_generated_kind_hidden(track_ref, tag);
                }
                let arrow = expand_arrow(tag_expanded);
                ui.selectable_label(
                    tag_expanded,
                    format!("{arrow} {}  {tag_count}", tag.label()),
                )
            });
            if row.inner.clicked() {
                ctx.tree.toggle_generated_kind_expanded(track_ref, tag);
            }
            if !tag_expanded {
                continue;
            }
            ui.indent((tag, track_ref), |ui| {
                for &pi in indices {
                    let Some(marker) = track.generated_markers.get(pi) else {
                        continue;
                    };
                    let point_ref = DataPointRef {
                        track: track_ref,
                        category: DataCategory::GeneratedMarker,
                        point_index: PointIdx::new(pi),
                    };
                    // A multi-satellite slip shows its satellite count; the
                    // others need no per-marker detail beyond the time.
                    let detail = match &marker.kind {
                        GeneratedMarkerKind::Slip(event) if event.slips.len() > 1 => {
                            format!("  ({})", event.slips.len())
                        }
                        _ => String::new(),
                    };
                    let label = format!("{}{detail}", marker.time.format("%H:%M:%S"));
                    let lat_lon = (marker.lat.as_degrees(), marker.lon.as_degrees());
                    point_item_row(
                        ui,
                        point_ref,
                        label,
                        lat_lon,
                        ctx.highlight,
                        ctx.map_center_request,
                        ctx.popup_pos_request,
                    );
                }
            });
        }
    });
}

fn file_bounding_center(file: Option<&LoadedFile>) -> Option<(f64, f64)> {
    let tracks = &file?.tracks;
    if tracks.is_empty() {
        return None;
    }
    let min_lat = tracks
        .iter()
        .map(|t| t.metadata.bounding_box.min().y)
        .fold(f64::INFINITY, f64::min);
    let max_lat = tracks
        .iter()
        .map(|t| t.metadata.bounding_box.max().y)
        .fold(f64::NEG_INFINITY, f64::max);
    let min_lon = tracks
        .iter()
        .map(|t| t.metadata.bounding_box.min().x)
        .fold(f64::INFINITY, f64::min);
    let max_lon = tracks
        .iter()
        .map(|t| t.metadata.bounding_box.max().x)
        .fold(f64::NEG_INFINITY, f64::max);
    Some(((min_lat + max_lat) / 2.0, (min_lon + max_lon) / 2.0))
}

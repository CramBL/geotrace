use egui::WidgetText;
use egui_phosphor::regular::CARET_DOWN as ICON_CARET_DOWN;
use egui_tiles::{SimplificationOptions, TileId, UiResponse};
use gt_loaded_files::RecordingNames;
use gt_map::{MapContextAction, MapDrawContext, NavMap};
use gt_types::LoadedFile;
use gt_ui_types::{HighlightScope, TrackDataVisibility};

use super::SharedAppState;

/// Pane variants for the central area tiles tree.
pub(super) enum MainPane {
    Map,
    Plot,
}

/// Behavior implementation that renders each pane of the central tiles tree.
pub(super) struct MainBehavior<'a> {
    pub(super) map: &'a mut NavMap,
    pub(super) state: &'a mut SharedAppState,
    pub(super) plot_hover_scope: Option<HighlightScope>,
    pub(super) map_hover_time: Option<chrono::DateTime<chrono::Utc>>,
    /// Time span of the match hovered in the query results table, shaded on
    /// the plot (one frame behind the query window, like `query_matches`).
    pub(super) match_hover_time_range:
        Option<(chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>)>,
    pub(super) toggle_plot_request: bool,
    /// Matches of the last query run, drawn as halos (one frame behind the
    /// query window, which renders after the tiles tree).
    pub(super) query_matches: Option<&'a gt_ui_types::QueryMatches>,
    /// Snapped-track geometry of completed, shown snap runs.
    pub(super) snapped_tracks: &'a gt_ui_types::SnappedTracks,
    /// The interference cells the overlay draws, for the shown day.
    pub(super) jamming_dataset: Option<&'a gt_jam::dataset::JamDataset>,
    /// Which day the overlay shows, driven by the stepper in the eye popup.
    pub(super) jamming_day: &'a mut gt_jam::day_selection::DaySelection,
    /// Why the overlay is drawing nothing, for the popup's legend line.
    pub(super) jamming_empty: Option<gt_jam::day_selection::EmptyReason>,
    /// The archived TEC grid the heatmap draws, at the shown instant.
    pub(super) tec_snapshot: Option<gt_map::TecHeatmapSnapshot<'a>>,
    /// Which instant the heatmap shows, driven by the stepper in the eye popup
    /// and by the hovered or selected fix.
    pub(super) tec_instant: &'a mut gt_ionex::TecInstantSelection,
    /// Why the heatmap is drawing nothing, for the popup's row.
    pub(super) tec_empty: Option<gt_ionex::TecEmptyReason>,
    /// Snap error per track of completed snap runs, for the plot.
    pub(super) snap_error: &'a gt_ui_types::SnapErrorSeries,
    /// Interference per fix, resolved from the archive.
    pub(super) jamming_series: &'a gt_ui_types::JammingSeries,
    /// Geomagnetic index values per fix, resolved from the archive.
    pub(super) geomagnetic_series: &'a gt_ui_types::GeomagneticSeries,
    /// TEC per fix, resolved from the archive.
    pub(super) tec_series: &'a gt_ui_types::TecSeries,
    /// The context metric lines, resolved over the span the plot last drew.
    pub(super) context_lines: &'a gt_ui_types::ContextLines,
    /// The flares of the archived days in that span.
    pub(super) solar_flares: &'a [gt_flare::SolarFlare],
}

impl egui_tiles::Behavior<MainPane> for MainBehavior<'_> {
    fn pane_ui(&mut self, ui: &mut egui::Ui, _tile_id: TileId, pane: &mut MainPane) -> UiResponse {
        match pane {
            MainPane::Map => {
                let s = &mut *self.state;
                let center_req = s.map_center_request.take();
                let popup_pos = s.popup_pos_request.take();
                let zoom_to_visible = std::mem::replace(&mut s.zoom_to_visible_request, false);
                let recording_names =
                    RecordingNames::resolve(s.loaded_files.view(), &s.recording_name_template);
                if let Some(action) = self.map.draw(
                    ui,
                    MapDrawContext {
                        files: &s.loaded_files,
                        recording_names: &recording_names,
                        snapped_tracks: Some(self.snapped_tracks),
                        jamming_dataset: self.jamming_dataset,
                        tec: gt_map::TecLayer {
                            snapshot: self.tec_snapshot,
                            instant: &mut *self.tec_instant,
                            empty_reason: self.tec_empty,
                        },
                        query_matches: self.query_matches,
                        empty_reason: self.jamming_empty,
                        filter: &s.filter,
                        visibility: s.tree.visibility(),
                        event_marker_visibility: s.tree.event_marker_visibility(),
                        generated_marker_visibility: s.tree.generated_marker_visibility(),
                        display_mask: &mut s.display_mask,
                        day_selection: &mut *self.jamming_day,
                        highlight: &mut s.highlight,
                        sky_glyph_variant: &mut s.sky_glyph_variant,
                        point_window_folds: &mut s.point_window_folds,
                        center_request: center_req,
                        zoom_to_visible,
                        sticky_pos_override: popup_pos,
                    },
                ) {
                    match action {
                        MapContextAction::ShowOnlyTrack(track) => {
                            s.tree.show_only_track(track);
                        }
                        MapContextAction::ShowOnlyFile(fi) => {
                            s.tree.show_only_file(fi);
                        }
                        MapContextAction::ShowSkyTrails(request) => {
                            s.sky_trails_request = Some(request);
                        }
                    }
                }
            }
            MainPane::Plot => {
                egui::Panel::top("plot_header").show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui
                                .button(ICON_CARET_DOWN)
                                .on_hover_text("Hide plot")
                                .clicked()
                            {
                                self.toggle_plot_request = true;
                            }
                        });
                    });
                });
                let s = &mut *self.state;
                let map_sync_x_range = if s.plot_state.sync_to_map {
                    self.map.viewport_geo_bounds().and_then(|b| {
                        tpv_time_range_in_bounds(&s.loaded_files, s.tree.visibility(), b)
                    })
                } else {
                    None
                };
                let names =
                    RecordingNames::resolve(s.loaded_files.view(), &s.recording_name_template);
                gt_plot::show_track_plot(
                    ui,
                    &s.loaded_files,
                    &names,
                    s.tree.visibility(),
                    &s.filter,
                    self.plot_hover_scope,
                    self.map_hover_time,
                    self.match_hover_time_range,
                    map_sync_x_range,
                    self.snap_error,
                    self.jamming_series,
                    self.geomagnetic_series,
                    self.tec_series,
                    gt_plot::ArchiveOverlays {
                        context_lines: self.context_lines,
                        solar_flares: self.solar_flares,
                    },
                    &mut s.plot_state,
                );
            }
        }
        UiResponse::None
    }

    fn tab_title_for_pane(&mut self, pane: &MainPane) -> WidgetText {
        match pane {
            MainPane::Map => "Map".into(),
            MainPane::Plot => "Plot".into(),
        }
    }

    fn simplification_options(&self) -> SimplificationOptions {
        // Do not auto-prune single-child or empty containers - this keeps the
        // root Linear alive when the plot is hidden so the plot tile can be
        // re-added to children without rebuilding the whole tree.
        SimplificationOptions {
            prune_single_child_containers: false,
            prune_empty_containers: false,
            ..Default::default()
        }
    }
}

/// Find the Unix-second time range of TPV points that lie within the given map
/// geographic bounds, considering only files/tracks currently enabled in `visibility`.
///
/// Returns `None` when no visible TPV points fall in the viewport.
fn tpv_time_range_in_bounds(
    files: &[LoadedFile],
    visibility: &TrackDataVisibility,
    bounds: gt_map::GeoBounds,
) -> Option<(f64, f64)> {
    let mut t_min = f64::INFINITY;
    let mut t_max = f64::NEG_INFINITY;

    for (fi, file) in files.iter().enumerate() {
        let Some(fv) = visibility.files.get(fi) else {
            continue;
        };
        if !fv.enabled {
            continue;
        }
        for (ti, track) in file.tracks.iter().enumerate() {
            let Some(tv) = fv.tracks.get(ti) else {
                continue;
            };
            if !tv.enabled {
                continue;
            }
            for point in &track.points {
                let lat = point.tpv.lat().as_degrees();
                let lon = point.tpv.lon().as_degrees();
                if lat < bounds.lat_min
                    || lat > bounds.lat_max
                    || lon < bounds.lon_min
                    || lon > bounds.lon_max
                {
                    continue;
                }
                let t = point.tpv.time().utc().timestamp() as f64;
                t_min = t_min.min(t);
                t_max = t_max.max(t);
            }
        }
    }

    if t_min.is_finite() && t_max.is_finite() {
        Some((t_min, t_max))
    } else {
        None
    }
}

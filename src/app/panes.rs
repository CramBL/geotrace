use egui::{Align, Align2, Area, Frame, Layout, Role, WidgetInfo, WidgetText};
use egui_notify::Toasts;
use egui_phosphor::regular::CARET_DOWN as ICON_CARET_DOWN;
use egui_tiles::{SimplificationOptions, TileId, UiResponse};
use gt_loaded_files::RecordingNames;
use gt_map::{MapAction, MapDrawContext, MapFiles, MapPresentationInput, NavMap};
use gt_plot::PlotViewOutcome;
use gt_session_bridge::ErrorCode;
use gt_types::{Generation, LoadedFile};
use gt_ui_types::{HighlightScope, TrackDataVisibility};

use super::SharedAppState;
use super::pane_presentation::PresentationControls;
use super::query_run::BackgroundQuery;
use super::session_map::GuiCameraRequests;

/// Pane variants for the central area tiles tree.
pub(super) enum MainPane {
    Map,
    Plot,
}

/// Behavior implementation that renders each pane of the central tiles tree.
pub(super) struct MainBehavior<'a> {
    pub(super) map: &'a mut NavMap,
    pub(super) queries: &'a BackgroundQuery,
    pub(super) checked_plot_visibility: Generation<TrackDataVisibility>,
    pub(super) state: &'a mut SharedAppState,
    pub(super) toasts: &'a mut Toasts,
    /// The display names of the loaded recordings, resolved once for the frame.
    pub(super) recording_names: &'a RecordingNames,
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
    /// What the loaded logs' filters selected onto the map.
    pub(super) log_matches: &'a gt_ui_types::LogMatches,
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
    pub(super) solar_flares: &'a [gt_flare::MarkedFlare],
    /// What the map's environment warning indicator shows: the lines from
    /// [`SpaceWeatherWarning`](crate::app::space_weather_warning::SpaceWeatherWarning)
    /// and the levels behind them.
    pub(super) space_weather: gt_map::SpaceWeatherIndicator<'a>,
}

impl egui_tiles::Behavior<MainPane> for MainBehavior<'_> {
    fn pane_ui(&mut self, ui: &mut egui::Ui, _tile_id: TileId, pane: &mut MainPane) -> UiResponse {
        match pane {
            MainPane::Map => {
                let s = &mut *self.state;
                let map_viewport = ui.max_rect().intersect(ui.clip_rect());
                let can_draw_map = map_viewport.is_finite() && map_viewport.is_positive();
                let map_input = if can_draw_map && s.pending_session_map.is_some() {
                    self.map.presentation_input(ui, s.highlight.sticky())
                } else {
                    MapPresentationInput::Ready
                };
                let presentation =
                    if can_draw_map && matches!(map_input, MapPresentationInput::Ready) {
                        s.apply_session_map(self.map, self.query_matches, self.queries)
                    } else {
                        None
                    };
                let GuiCameraRequests {
                    center,
                    zoom_to_visible,
                    reveal,
                } = if can_draw_map {
                    s.take_gui_camera_requests_for_draw()
                } else {
                    GuiCameraRequests::default()
                };
                let popup_pos = s.popup_pos_request.take();
                let frame = self.map.draw(
                    ui,
                    MapDrawContext {
                        files: MapFiles::Loaded(s.loaded_files.view()),
                        recording_names: self.recording_names,
                        snapped_tracks: Some(self.snapped_tracks),
                        jamming_dataset: self.jamming_dataset,
                        tec: gt_map::TecLayer {
                            snapshot: self.tec_snapshot,
                            instant: &mut *self.tec_instant,
                            empty_reason: self.tec_empty,
                        },
                        query_matches: self.query_matches,
                        log_matches: self.log_matches,
                        log_hover: &mut s.log_hover,
                        clicked_log_glyph: &mut s.clicked_log_glyph,
                        empty_reason: self.jamming_empty,
                        space_weather: self.space_weather,
                        filter: &s.filter,
                        visibility: s.tree.visibility(),
                        event_marker_visibility: s.tree.event_marker_visibility(),
                        generated_marker_visibility: s.tree.generated_marker_visibility(),
                        display_mask: &mut s.display_mask,
                        day_selection: &mut *self.jamming_day,
                        highlight: &mut s.highlight,
                        sky_glyph_variant: &mut s.sky_glyph_variant,
                        point_window_folds: &mut s.point_window_folds,
                        center_request: center,
                        zoom_to_visible,
                        reveal_query_matches: reveal,
                        sticky_pos_override: popup_pos,
                        presentation_camera: presentation.as_ref().map(|draw| draw.camera),
                    },
                );
                if let Some(action) = frame.action {
                    match action {
                        MapAction::ShowOnlyTrack(track) => {
                            s.tree.show_only_track(track);
                        }
                        MapAction::ShowOnlyFile(fi) => {
                            s.tree.show_only_file(fi);
                        }
                        MapAction::ShowSkyTrails(request) => {
                            s.sky_trails_request = Some(request);
                        }
                        MapAction::OpenReferenceDocument(document) => {
                            s.reference_document_request = Some(document);
                        }
                    }
                }
                s.finish_session_map_input(ui.ctx(), map_input, frame.interaction);
                if let Some(draw) = presentation {
                    draw.confirm(self.toasts);
                }
                if map_viewport.is_positive()
                    && let Some(presentation) = s.presented_session_map
                    && let Some(caption) = presentation
                        .target()
                        .label(self.recording_names, s.loaded_files.files())
                    && self::map_presentation_controls(
                        ui,
                        map_viewport,
                        PresentationControls {
                            id: presentation.id(),
                            caption: &caption,
                            dismiss_label: "Dismiss map presentation",
                        },
                    )
                {
                    s.clear_session_map(self.map);
                    ui.ctx().request_repaint();
                }
            }
            MainPane::Plot => {
                let s = &mut *self.state;
                let header_action = self::plot_header(ui, s, self.recording_names);
                self.toggle_plot_request = matches!(header_action, PlotHeaderAction::Hide);
                if self.toggle_plot_request {
                    s.reject_pending_session_plot(ErrorCode::PlotPresentationSuperseded);
                }
                let map_sync_x_range = if s.plot_state.sync_to_map {
                    self.map.viewport_geo_bounds().and_then(|bounds| {
                        s.map_synced_plot_range.range(
                            bounds,
                            s.tree.visibility(),
                            &s.loaded_files,
                            tpv_time_range_in_bounds,
                        )
                    })
                } else {
                    None
                };
                let plot_ctx = ui.ctx().clone();
                let waiting_view = || {
                    if matches!(header_action, PlotHeaderAction::HidePressed) {
                        return None;
                    }
                    let pending = s.pending_session_plot.as_ref()?;
                    if self.checked_plot_visibility != s.tree.visibility_generation() {
                        plot_ctx.request_repaint();
                        return None;
                    }
                    pending.waiting_view()
                };
                let outcome = gt_plot::show_track_plot(
                    ui,
                    &s.loaded_files,
                    self.recording_names,
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
                    waiting_view,
                    &mut s.plot_state,
                );
                match outcome {
                    PlotViewOutcome::Framed => s.confirm_session_plot(self.toasts),
                    PlotViewOutcome::Superseded => {
                        s.reject_pending_session_plot(ErrorCode::PlotPresentationSuperseded);
                    }
                    PlotViewOutcome::Unchanged => {}
                }
                if s.presented_session_plot
                    .as_ref()
                    .is_some_and(|presentation| {
                        s.plot_state.visible_x_range() != Some(presentation.view.framed_x_range())
                    })
                {
                    s.presented_session_plot = None;
                    ui.ctx().request_repaint();
                }
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

enum PlotHeaderAction {
    Hide,
    HidePressed,
    None,
}

fn plot_header(
    ui: &mut egui::Ui,
    state: &mut SharedAppState,
    names: &RecordingNames,
) -> PlotHeaderAction {
    egui::Panel::top("plot_header")
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let hide = ui.button(ICON_CARET_DOWN).on_hover_text("Hide plot");
                    hide.widget_info(|| {
                        WidgetInfo::labeled(Role::Button, hide.enabled(), "Hide plot")
                    });
                    if let Some(presentation) = state.presented_session_plot.as_ref()
                        && let Some(caption) =
                            presentation.target.label(names, state.loaded_files.files())
                        && (PresentationControls {
                            id: presentation.id,
                            caption: &caption,
                            dismiss_label: "Dismiss plot presentation",
                        })
                        .show(ui)
                    {
                        state.presented_session_plot = None;
                    }
                    if hide.clicked() {
                        PlotHeaderAction::Hide
                    } else if hide.is_pointer_button_down_on() {
                        PlotHeaderAction::HidePressed
                    } else {
                        PlotHeaderAction::None
                    }
                })
                .inner
            })
            .inner
        })
        .inner
}

fn map_presentation_controls(
    ui: &egui::Ui,
    viewport: egui::Rect,
    controls: PresentationControls<'_>,
) -> bool {
    let inset = ui.spacing().item_spacing;
    let width = (viewport.width() - 2.0 * inset.x).min(MAP_PRESENTATION_MAX_WIDTH);
    Area::new(ui.scope_id().with("map_presentation"))
        .enabled(ui.is_enabled())
        .fixed_pos(viewport.center_top() + egui::vec2(0.0, inset.y))
        .pivot(Align2::CENTER_TOP)
        .constrain_to(viewport)
        .show(ui.ctx(), |ui| {
            ui.set_max_width(width.max(0.0));
            ui.set_clip_rect(viewport);
            Frame::popup(ui.style())
                .show(ui, |ui| ui.horizontal(|ui| controls.show(ui)).inner)
                .inner
        })
        .inner
}

const MAP_PRESENTATION_MAX_WIDTH: f32 = 370.0;

/// Find the Unix-second time range of TPV points that lie within `bounds`,
/// considering only files/tracks currently enabled in `visibility`.
///
/// Returns `None` when no visible TPV points fall in the viewport.
fn tpv_time_range_in_bounds(
    files: &[LoadedFile],
    visibility: &TrackDataVisibility,
    bounds: gt_map::ViewportBounds,
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
            // A track with no geometry is drawn nowhere, so no map area
            // covers any of its fixes.
            let Some(placed) = track.placed_points() else {
                continue;
            };
            for point in placed.iter() {
                let (latitude, longitude) = point.resolved_position();
                let lat = latitude.as_degrees();
                let lon = longitude.as_degrees();
                if lat < bounds.lat_min
                    || lat > bounds.lat_max
                    || lon < bounds.lon_min
                    || lon > bounds.lon_max
                {
                    continue;
                }
                let t = point.fix.tpv.time().utc().timestamp() as f64;
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

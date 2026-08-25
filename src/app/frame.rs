use std::sync::Arc;

use egui::{Button, CentralPanel, Label, MenuBar, ProgressBar, RichText, Sides, Window};
use egui_phosphor::regular::ARTICLE as ICON_ARTICLE;
use egui_phosphor::regular::CHART_LINE_UP as ICON_CHART_LINE_UP;
use egui_phosphor::regular::CHECK as ICON_CHECK;
use egui_phosphor::regular::CLOCK_COUNTER_CLOCKWISE as ICON_CLOCK_COUNTER_CLOCKWISE;
use egui_phosphor::regular::GEAR as ICON_GEAR;
use egui_phosphor::regular::TERMINAL_WINDOW as ICON_TERMINAL_WINDOW;
use egui_phosphor::regular::TRASH as ICON_TRASH;
use egui_phosphor::regular::WARNING as ICON_WARNING;
use egui_phosphor::regular::X as ICON_X;
use gt_ionex::quiet_time::QuietTimeDeviation;
use gt_loaded_files::RecordingNames;
use gt_map::MapLayer;
use gt_query_run::RunInputs;
use gt_side_panel::{PanelContext, SnapCostingTarget, SnapPanelView, show_side_panel};
use gt_track_builder::SegmentationConfig;
use gt_types::{DataCategory, FileIdx, LoadedFile, TrackIdx, TrackRef};
use gt_ui_types::{
    ArcIdentity, ContextLines, GeomagneticSeries, HighlightScope, JammingSeries, MapHighlight,
    TecSeries,
};
use rustc_hash::FxHashMap;

use super::context_line::ContextSpan;
use super::fix_positions::FixPositionTimeline;
use super::loader::{
    CompletedLoad, FINISHED_JOB_EXPIRE_SECS, FINISHED_JOB_FADE_START_SECS, LoadJobs,
};
use super::log_viewer::{self, LogViewerContext};
use super::modals::{
    SnapAutoChoice, SnapConsentChoice, SnapReplaceChoice, SnapScopeChoice, show_about_dialog,
    show_delete_confirmation, show_load_warnings_dialog, show_mapbox_token_dialog,
    show_orphaned_event_markers_popup, show_recording_details_dialog, show_snap_auto_prompt,
    show_snap_consent_dialog, show_snap_replace_dialog, show_snap_scope_dialog,
};
use super::panes::MainBehavior;
use super::shutdown::FrameContents;
use super::snap_state::PendingSnapRequest;
use super::space_weather_warning::{self, RecordingSeries, RecordingUnderAssessment};
use super::storage::{DatabasesPending, OPENING_DATABASES, QueuedLoad};
#[cfg(feature = "self-update")]
use super::update;
use super::{App, SharedAppState, modals};

const DROP_OVERLAY_SCRIM_OPACITY: f32 = 0.85;

/// Width of the loading-progress overlay: enough for a progress bar and its
/// stage, and no more than a recording name needs before it truncates.
const LOADING_OVERLAY_MIN_WIDTH: f32 = 260.0;
const LOADING_OVERLAY_MAX_WIDTH: f32 = 340.0;

impl eframe::App for App {
    fn save(&mut self, _storage: &mut dyn eframe::Storage) {
        self.flush_settings();
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        if self.intercept_close_request(ui) == FrameContents::ShutdownWindow {
            return;
        }

        // Kick off the one-shot startup update check (no-op after the first
        // frame, and only when enabled / release build / not offline).
        #[cfg(feature = "self-update")]
        if self.should_check_for_updates() {
            self.update_checker.start(ui.ctx());
        }

        self.apply_finished_background_work(ui);
        self.wait_for_the_data_directory(ui);
        self.show_interrupted_delete_prompts(ui);
        self.retry_marking_the_data_directory_in_the_background(ui.ctx());
        self.load_files_from_dialog_drops_and_paste(ui);
        self.unload_selection_on_delete_key(ui);
        self.show_top_menu_bar(ui);
        self.forward_legend_hover_to_map_highlight();
        self.show_track_data_panel(ui);
        self.show_central_area(ui);
        self.apply_log_viewer_requests();
        self.store_attached_log_filter_edits();

        let apply_resegment = self.show_settings_window(ui);
        if apply_resegment {
            self.apply_resegmentation();
        }
        self.reference_window.show(ui.ctx());

        if self.map.layer() == MapLayer::Satellite && !self.map.has_mapbox_token() {
            show_mapbox_token_dialog(ui, &mut self.map, &mut self.mapbox_token_field);
        }

        show_about_dialog(ui, &mut self.about_open, self.app_version);

        self.show_snap_prompts(ui);
        self.show_loading_progress_overlay(ui);
        show_drop_hint_overlay(ui.ctx());
        self.show_load_error_bar(ui);
        self.apply_pending_unload_and_removal(ui);

        show_orphaned_event_markers_popup(ui, &mut self.orphaned_event_markers);
        show_load_warnings_dialog(ui, &mut self.shared.borrow_mut().warnings_popup);
        show_recording_details_dialog(ui, &mut self.shared.borrow_mut().metadata_popup);

        self.show_history_window(ui);
        self.show_history_failure_prompt(ui);
        self.show_resegment_prompt(ui);
        self.show_auto_prune_prompt(ui);
        self.show_environment_prune_prompt(ui);
        self.show_log_association_dialog(ui);

        // Show the self-update prompt (if an in-place update was found and not
        // skipped). Package-manager/manual builds show the menu-bar badge instead.
        #[cfg(feature = "self-update")]
        if let Some(event) = self
            .update_checker
            .ui(ui.ctx(), self.skipped_version.as_deref())
        {
            match event {
                update::UpdateEvent::Skip(version) => self.skipped_version = Some(version),
            }
        }

        self.toasts.show(ui.ctx());

        // Detect settings changes and trigger a debounced write-through.
        let snapshot = self.collect_snapshot();
        self.config.sync(snapshot);
        if self.config.take_flush() {
            self.flush_settings();
        }
    }
}

impl App {
    pub(in crate::app) fn apply_finished_background_work(&mut self, ui: &egui::Ui) {
        // Adopt the databases first: what waited on them starts in the frame
        // they arrive.
        self.adopt_finished_archive_inspection();
        self.adopt_finished_storage_open();

        // Drain background load results first so newly loaded data is
        // visible in the same frame that it arrives.
        let completed_loads: Vec<CompletedLoad> = self.loader.drain();
        let frame_time = ui.ctx().input(|i| i.time);
        for completed in completed_loads {
            self.handle_completed_load(completed, frame_time);
        }

        // Apply any results the history worker has finished since last frame.
        for resp in self.history.poll() {
            self.handle_history_response(resp);
        }
        self.jamming.poll();
        self.geomagnetic_indices.poll();
        self.tec_maps.poll();
        self.solar_flares.poll();
        self.poll_environment_prune();

        // Apply finished snap runs and progress updates, persist completed
        // runs of history-stored files, and let the queue react to
        // visibility changes (parked entries may become eligible).
        let completed_snaps = self.snap.poll();
        if !completed_snaps.is_empty() {
            self.persist_snap_runs(&completed_snaps);
        }
        self.snap
            .set_visibility(self.shared.borrow().tree.visibility());
        if std::mem::take(&mut self.snap_auto_sweep) && !self.shutdown.has_begun() {
            self.queue_auto_snaps();
        }
    }

    fn load_files_from_dialog_drops_and_paste(&mut self, ui: &egui::Ui) {
        // Consume a pending file-picker result and dispatch the chosen path.
        if let Some(path) = self.loader.drain_file_dialog() {
            self.spawn_load_path(path);
        }

        let dropped = ui.ctx().input(|i| i.raw.dropped_files.clone());
        for file in dropped {
            let path = file.path();
            // Native drops carry an absolute path, web drops only a relative
            // file name plus bytes read through the handle.
            if path.is_absolute() {
                self.spawn_load_path(path.to_path_buf());
            } else if let Ok(bytes) = file.bytes() {
                let name = path.file_name().map_or_else(
                    || "dropped file".to_owned(),
                    |n| n.to_string_lossy().into_owned(),
                );
                self.handle_dropped_bytes(bytes.into(), &name);
            }
        }

        self.load_pasted_log_text(ui.ctx());
    }

    /// Reads a Ctrl+V anywhere in the app as log text, unless a widget holds
    /// keyboard focus: a paste into a text field belongs to that field.
    ///
    /// Only text arrives as [`egui::Event::Paste`], so other clipboard content
    /// never reaches this.
    fn load_pasted_log_text(&mut self, ctx: &egui::Context) {
        if ctx.memory(|memory| memory.focused()).is_some() {
            return;
        }
        // Taken out of the frame's events: nothing else may act on a paste the
        // log loader has claimed.
        let pasted = ctx.input_mut(|input| {
            let mut pasted: Vec<String> = Vec::new();
            input.events.retain_mut(|event| match event {
                egui::Event::Paste(text) => {
                    pasted.push(std::mem::take(text));
                    false
                }
                _ => true,
            });
            pasted
        });
        for text in pasted {
            if text.is_empty() {
                continue;
            }
            if let Some(queued_loads) = self.storage_open.queued_loads_mut() {
                queued_loads.push(QueuedLoad::PastedText(text));
                continue;
            }
            self.loader.spawn_pasted_log_text(text);
        }
    }

    fn unload_selection_on_delete_key(&self, ui: &egui::Ui) {
        let mut s = self.shared.borrow_mut();
        let delete_pressed = ui
            .ctx()
            .input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Delete));
        if delete_pressed && !s.tree.selection.is_empty() && s.tree.pending_unload.is_none() {
            // Delete key unloads the selection from the view (non-destructive,
            // recordings stay in history).
            s.tree.pending_unload = Some(s.tree.selection.iter().cloned().collect());
        }
    }

    fn show_top_menu_bar(&mut self, ui: &mut egui::Ui) {
        egui::Panel::top("top_panel").show(ui, |ui| {
            MenuBar::new().ui(ui, |ui| {
                // Left zone - the File menu
                ui.menu_button("File", |ui| {
                    if ui.button("Open…").clicked() {
                        self.loader.open_file_dialog();
                        ui.close();
                    }
                    ui.separator();
                    if ui.button("About GeoTrace").clicked() {
                        self.about_open = true;
                        ui.close();
                    }
                });

                // Right zone - utility windows and preferences, trailing-aligned
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    egui::widgets::global_theme_preference_buttons(ui);

                    ui.separator();

                    if ui
                        .selectable_label(self.settings_open, ICON_GEAR)
                        .on_hover_text("Settings")
                        .clicked()
                    {
                        self.settings_open = !self.settings_open;
                    }

                    if ui
                        .selectable_label(self.history_window.open, ICON_CLOCK_COUNTER_CLOCKWISE)
                        .on_hover_text("Browse and re-open previously recorded sessions")
                        .clicked()
                    {
                        self.history_window.open = !self.history_window.open;
                        self.history_window.invalidate();
                    }

                    if ui
                        .selectable_label(self.log_viewer.open, ICON_ARTICLE)
                        .on_hover_text("Read the loaded logs against the recordings")
                        .clicked()
                    {
                        self.log_viewer.open = !self.log_viewer.open;
                    }

                    // While a query is filtering the map but its window is
                    // closed, the button turns amber with a "!" so the active
                    // filter is not forgotten. Right-click clears it. The amber
                    // is dimmed in light mode, where the bright tone glares.
                    let query_active = self.query_window.filter_active();
                    let show_alert = query_active && !self.query_window.open;
                    let query_label = if show_alert {
                        RichText::new(format!("{ICON_TERMINAL_WINDOW} !"))
                            .color(gt_ui_theme::warning_amber(ui.visuals().dark_mode))
                    } else {
                        RichText::new(ICON_TERMINAL_WINDOW)
                    };
                    let query_button = ui.selectable_label(self.query_window.open, query_label);
                    let query_button = if query_active {
                        query_button.on_hover_text(format!(
                            "Query the loaded data. A query filter is active {} \
                             right-click to clear it.",
                            gt_ui_theme::EM_DASH
                        ))
                    } else {
                        query_button.on_hover_text("Query the loaded data")
                    };
                    if query_button.clicked() {
                        self.query_window.open = !self.query_window.open;
                    }
                    if query_active {
                        query_button.context_menu(|ui| {
                            if ui
                                .button(format!("{ICON_TRASH} Clear query filter"))
                                .clicked()
                            {
                                self.query_window.clear_filter();
                                ui.close();
                            }
                        });
                    }

                    // A subtle "update available" hint for builds that can't
                    // self-update (Homebrew, MSI, manual download). Self-updatable
                    // installs get the prompt instead of this badge.
                    #[cfg(feature = "self-update")]
                    if let Some(new_version) = self.update_checker.badge_version() {
                        ui.separator();
                        let text = RichText::new("Update available")
                            .color(gt_ui_theme::warning_amber(ui.visuals().dark_mode));
                        if ui
                            .add(Label::new(text).sense(egui::Sense::click()))
                            .on_hover_text(format!(
                                "GeoTrace {new_version} is available (current: {}). Update \
                                 through your package manager, or open the releases page.",
                                self.app_version
                            ))
                            .clicked()
                        {
                            ui.ctx()
                                .open_url(egui::OpenUrl::new_tab(update::RELEASES_URL));
                        }
                    }
                });
            });
        });
    }

    fn forward_legend_hover_to_map_highlight(&self) {
        // Forward the previous frame's plot-legend hover so NavMap::draw's
        // track layers highlight the matching file on the map this frame.
        // NavMap overwrites `highlight.hover` with its own pointer-hover at
        // the end of draw(), which is fine: the plot re-derives its line
        // highlight from `legend_hover_file` directly, so this write only
        // needs to survive until the map has rendered.
        let mut s = self.shared.borrow_mut();
        if let Some(fi) = s.plot_state.legend_hover_file {
            s.highlight.hover = Some(HighlightScope::File {
                file_index: FileIdx::new(fi),
            });
        }
    }

    fn show_track_data_panel(&mut self, ui: &mut egui::Ui) {
        // Snap view for the side panel, resolved once per frame and shared by
        // the docked and detached call sites. The trigger's request is drained
        // after the panel, mirroring the other panel requests.
        let snap_rows = self.snap_row_views();
        let snap_costing_choices = Self::costing_choices();
        let snap_progress = {
            let progress = self.snap.progress();
            gt_side_panel::SnapProgressView {
                in_flight: progress
                    .in_flight
                    .map(|run| gt_side_panel::SnapInFlightView {
                        track: run.track,
                        completed_chunks: run.completed_chunks,
                        total_chunks: run.total_chunks,
                    }),
                queued: progress.queued,
            }
        };
        let snap_view = SnapPanelView {
            offline: self.offline,
            consent_pending: !self.snap_settings.consent_granted(),
            rows: &snap_rows,
            costing_choices: &snap_costing_choices,
            progress: &snap_progress,
        };
        let mut snap_request: Option<TrackRef> = None;
        let mut snap_visibility_request: Option<TrackRef> = None;
        let mut snap_costing_request: Option<(SnapCostingTarget, gt_ui_types::SnapCosting)> = None;
        let mut sky_trails_request: Option<gt_ui_types::SkyTrailsRequest> = None;

        let detached = self.shared.borrow().tree.detached;
        if !detached {
            egui::Panel::left("track_data_panel")
                .min_size(240.0)
                .show(ui, |ui| {
                    let mut refmut = self.shared.borrow_mut();
                    let s = &mut *refmut;
                    let loaded_files = s.loaded_files.view();
                    let recording_names =
                        RecordingNames::resolve(loaded_files, &s.recording_name_template);
                    show_side_panel(
                        ui,
                        &mut PanelContext {
                            loaded_files,
                            tree: &mut s.tree,
                            highlight: &mut s.highlight,
                            filter: &mut s.filter,
                            filter_state: &mut s.filter_state,
                            map_center_request: &mut s.map_center_request,
                            popup_pos_request: &mut s.popup_pos_request,
                            query_matches: self.query_window.matches(),
                            zoom_to_visible_request: &mut s.zoom_to_visible_request,
                            warnings_request: &mut s.warnings_popup,
                            clear_query_request: &mut s.clear_query_request,
                            display_mask: s.display_mask,
                            recording_names: &recording_names,
                            metadata_request: &mut s.metadata_popup,
                            snap: snap_view,
                            snap_request: &mut snap_request,
                            snap_visibility_request: &mut snap_visibility_request,
                            snap_costing_request: &mut snap_costing_request,
                            sky_trails_request: &mut sky_trails_request,
                        },
                    );
                });
        } else {
            // Render the panel as a floating egui Window inside the same OS window
            // as the map. A separate OS viewport caused Wayland compositors to
            // suspend event delivery when the child was minimised or occluded,
            // freezing both windows. The floating-window approach is fully
            // platform-independent.
            let mut is_open = !ui
                .ctx()
                .input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
            Window::new("Track data")
                .id(egui::Id::new("detached_panel"))
                .open(&mut is_open)
                .default_pos(egui::pos2(10.0, 30.0))
                .default_width(320.0)
                .min_width(240.0)
                .resizable(true)
                .show(ui.ctx(), |ui| {
                    let mut refmut = self.shared.borrow_mut();
                    let s = &mut *refmut;
                    let loaded_files = s.loaded_files.view();
                    let recording_names =
                        RecordingNames::resolve(loaded_files, &s.recording_name_template);
                    show_side_panel(
                        ui,
                        &mut PanelContext {
                            loaded_files,
                            tree: &mut s.tree,
                            highlight: &mut s.highlight,
                            filter: &mut s.filter,
                            filter_state: &mut s.filter_state,
                            map_center_request: &mut s.map_center_request,
                            popup_pos_request: &mut s.popup_pos_request,
                            query_matches: self.query_window.matches(),
                            zoom_to_visible_request: &mut s.zoom_to_visible_request,
                            warnings_request: &mut s.warnings_popup,
                            clear_query_request: &mut s.clear_query_request,
                            display_mask: s.display_mask,
                            recording_names: &recording_names,
                            metadata_request: &mut s.metadata_popup,
                            snap: snap_view,
                            snap_request: &mut snap_request,
                            snap_visibility_request: &mut snap_visibility_request,
                            snap_costing_request: &mut snap_costing_request,
                            sky_trails_request: &mut sky_trails_request,
                        },
                    );
                });
            if !is_open {
                self.shared.borrow_mut().tree.detached = false;
            }
        }

        if let Some(track_ref) = snap_request {
            self.handle_snap_request(vec![track_ref]);
        }
        if let Some((target, choice)) = snap_costing_request {
            self.handle_snap_costing_request(target, choice);
        }
        if let Some(track_ref) = snap_visibility_request
            && !self.hidden_snapped.remove(&track_ref)
        {
            self.hidden_snapped.insert(track_ref);
        }

        // "Show sky trails" from either the side panel or the map context
        // menu (the latter routed through shared state) opens the window.
        let map_trails_request = self.shared.borrow_mut().sky_trails_request.take();
        if let Some(request) = sky_trails_request.or(map_trails_request) {
            self.sky_trails_window.open(request);
        }

        let reference_request = self.shared.borrow_mut().reference_document_request.take();
        if let Some(document) = reference_request {
            self.reference_window.open(document);
        }

        // "Reset filters" also drops the query filter so the map fully clears.
        if std::mem::take(&mut self.shared.borrow_mut().clear_query_request) {
            self.query_window.clear_filter();
        }
    }

    /// The span the context metric lines are resolved over: the one the plot
    /// drew last frame, or [`None`] before it has drawn at all.
    fn context_span_of(plot_state: &gt_plot::PlotState) -> Option<ContextSpan> {
        plot_state.visible_x_range().map(ContextSpan::covering)
    }

    /// Where the receiver was over time, rebuilt when the loaded recordings
    /// change.
    fn fix_position_timeline(&mut self) -> Arc<FixPositionTimeline> {
        let shared = self.shared.borrow();
        Arc::clone(self.fix_positions.timeline(&shared.loaded_files))
    }

    /// The context metric lines over `span`, read from the archives at the
    /// receiver's positions.
    fn context_lines(
        &mut self,
        span: Option<ContextSpan>,
        positions: &Arc<FixPositionTimeline>,
    ) -> ContextLines {
        let Some(span) = span else {
            return ContextLines::default();
        };
        ContextLines {
            jamming: self.jamming.context_line(span, positions),
            geomagnetic: self.geomagnetic_indices.context_lines(span),
            tec: self.tec_maps.context_line(span, positions),
        }
    }

    /// Assess every loaded recording against the archived environment values,
    /// and toast the ones a disturbance reaches for the first time.
    ///
    /// Runs every frame: a fetch worker archiving a day replaces the series it
    /// reaches, which is what warns about a recording loaded before its days
    /// arrived.
    fn assess_space_weather(
        &mut self,
        jamming: &JammingSeries,
        geomagnetic: &GeomagneticSeries,
        tec: &TecSeries,
        tec_deviations: &FxHashMap<TrackRef, QuietTimeDeviation>,
        positions: &Arc<FixPositionTimeline>,
    ) {
        let newly_warned = {
            let shared = self.shared.borrow();
            let recordings: Vec<RecordingUnderAssessment<'_>> = shared
                .loaded_files
                .view()
                .entries()
                .enumerate()
                .map(|(index, entry)| {
                    let file = entry.file();
                    let span = file.metadata.time_range;
                    let fi = FileIdx::new(index);
                    let tracks =
                        (0..file.tracks.len()).map(move |ti| TrackRef::new(fi, TrackIdx::new(ti)));
                    RecordingUnderAssessment {
                        id: entry.id(),
                        span,
                        series: RecordingSeries::of(
                            tracks,
                            jamming,
                            geomagnetic,
                            tec,
                            tec_deviations,
                        ),
                        archived_flare_days: self.solar_flares.archived_days_for(span),
                        positions: ArcIdentity::of(positions),
                    }
                })
                .collect();
            self.space_weather_warning.reassess(&recordings, |span| {
                self.solar_flares.flares_peaking_in(span, positions)
            })
        };
        for _ in 0..newly_warned {
            self.toasts.warning(space_weather_warning::LOAD_WARNING);
        }
    }

    fn show_central_area(&mut self, ui: &mut egui::Ui) {
        // Assembled after the panel so a visibility toggle takes effect in
        // the same frame's map render.
        let snapped_tracks = self.snapped_tracks_view();
        let snap_error = self.snap_error_view();
        let snap_error_values = self.snap_error_values();
        let jamming_series = {
            let shared = self.shared.borrow();
            self.jamming.plot_series(&shared.loaded_files)
        };
        let jamming_query_values = self.jamming.query_values();
        let geomagnetic_series = {
            let shared = self.shared.borrow();
            self.geomagnetic_indices.plot_series(&shared.loaded_files)
        };
        let tec_series = {
            let shared = self.shared.borrow();
            self.tec_maps.plot_series(&shared.loaded_files)
        };
        let tec_deviations = {
            let shared = self.shared.borrow();
            self.tec_maps.quiet_time_deviations(&shared.loaded_files)
        };
        // Resolved over the span the plot reported when it last drew, so a
        // pan or zoom reaches the lines on the frame after it.
        let context_span = Self::context_span_of(&self.shared.borrow().plot_state);
        let fix_positions = self.fix_position_timeline();
        let context_lines = self.context_lines(context_span, &fix_positions);
        let solar_flares = context_span
            .map(|span| self.solar_flares.markers(span, &fix_positions))
            .unwrap_or_default();
        self.assess_space_weather(
            &jamming_series,
            &geomagnetic_series,
            &tec_series,
            &tec_deviations,
            &fix_positions,
        );

        CentralPanel::default().show(ui, |ui| {
            let panel_rect = ui.max_rect();
            let mut s = self.shared.borrow_mut();
            let plot_hover_scope = match s.highlight.hover {
                Some(HighlightScope::File { .. })
                | Some(HighlightScope::Track(_))
                | Some(HighlightScope::TrackCategory { .. }) => s.highlight.hover,
                Some(HighlightScope::Point(_)) | None => None,
            };
            // Falls back to the sky-trails scrubber's instant (written by that
            // window last frame) so playback draws the same plot time line a
            // track-point hover does.
            let map_hover_time =
                extract_map_hover_time(&s.loaded_files, &s.highlight).or(s.highlight.scrub_time);
            let match_hover_time_range =
                extract_match_hover_time_range(&s.loaded_files, &s.highlight);

            // Render the tiles tree (map on top, optional plot on bottom).
            // Borrow tiles_tree and map explicitly so the borrow checker can see
            // they are disjoint from s (which comes from self.shared).
            let toggle_plot_request;
            {
                let map = &mut self.map;
                let tiles_tree = &mut self.tiles_tree;
                let jamming_empty = self.jamming.empty_reason();
                let (jamming_dataset, jamming_day) = self.jamming.overlay_state();
                // The heatmap follows the fix the pointer is on or the one
                // clicked, and falls back to the plot cursor's own time, which
                // the plot wrote when it last drew.
                self.tec_maps
                    .follow_instant(map_hover_time.or(s.plot_state.hovered_time));
                let gt_map::TecLayer {
                    snapshot: tec_snapshot,
                    instant: tec_instant,
                    empty_reason: tec_empty,
                } = self.tec_maps.overlay_layer();
                let log_matches = self.logs.map_matches();
                let space_weather = gt_map::SpaceWeatherIndicator {
                    warning_lines: self.space_weather_warning.lines(),
                    levels: &space_weather_warning::WARNING_LEVELS,
                };
                let mut behavior = MainBehavior {
                    map,
                    state: &mut s,
                    plot_hover_scope,
                    map_hover_time,
                    match_hover_time_range,
                    toggle_plot_request: false,
                    query_matches: self.query_window.matches(),
                    snapped_tracks: &snapped_tracks,
                    log_matches,
                    snap_error: &snap_error,
                    jamming_series: &jamming_series,
                    geomagnetic_series: &geomagnetic_series,
                    tec_series: &tec_series,
                    context_lines: &context_lines,
                    solar_flares: &solar_flares,
                    space_weather,
                    jamming_dataset,
                    jamming_day,
                    jamming_empty,
                    tec_snapshot,
                    tec_instant,
                    tec_empty,
                };
                tiles_tree.ui(&mut behavior, ui);
                toggle_plot_request = behavior.toggle_plot_request;
            }
            if toggle_plot_request {
                self.tiles_tree.tiles.toggle_visibility(self.plot_tile_id);
            }
            // The plot reports the span it drew only after it has drawn, so a
            // view that moved out of the resolved span needs one more frame.
            if Self::context_span_of(&s.plot_state) != context_span {
                ui.ctx().request_repaint();
            }

            // Forward plot hover → map highlight (must happen after the tree renders
            // so that show_track_plot has already written the current hovered_time).
            // The pre-computed `plot_hover_point` lets TpvRenderer look up the
            // closest point in O(1).
            let plot_visible = self.plot_is_visible();
            if plot_visible {
                if let Some(cursor_time) = s.plot_state.hovered_time {
                    let closest = gt_plot::find_closest_tpv(
                        &s.loaded_files,
                        s.tree.visibility(),
                        &s.filter,
                        cursor_time,
                    );
                    s.highlight.plot_hover_time = closest.map(|_| cursor_time);
                    s.highlight.plot_hover_point = closest;
                    // `plot_cursor_snapped` is computed inside `show_track_plot`
                    // using a 2-D screen-space check (both time and metric value)
                    // so the overlay only triggers when egui_plot would also
                    // show a hover label.
                    s.highlight.plot_hover_snapped = s.plot_state.plot_cursor_snapped;
                } else {
                    s.highlight.plot_hover_time = None;
                    s.highlight.plot_hover_point = None;
                    s.highlight.plot_hover_snapped = false;
                }
            } else {
                s.plot_state.hovered_time = None;
                s.highlight.plot_hover_time = None;
                s.highlight.plot_hover_point = None;
                s.highlight.plot_hover_snapped = false;

                let btn_size = egui::vec2(28.0, 22.0);
                let btn_rect = egui::Rect::from_min_size(
                    egui::pos2(panel_rect.min.x + 8.0, panel_rect.max.y - btn_size.y - 8.0),
                    btn_size,
                );
                if ui
                    .put(btn_rect, Button::new(ICON_CHART_LINE_UP).small())
                    .on_hover_text("Show plot")
                    .clicked()
                {
                    self.tiles_tree.tiles.toggle_visibility(self.plot_tile_id);
                }
            }

            // After the plot-hover forwarding: match-table row hover writes
            // the same cross-highlight fields and must win for the frame.
            let SharedAppState {
                loaded_files,
                tree,
                highlight,
                filter,
                map_center_request,
                popup_pos_request,
                reveal_query_matches_request,
                plot_state,
                log_hover,
                display_mask,
                recording_name_template,
                ..
            } = &mut *s;
            // The map and plot consumed last frame's hovered match above.
            // Clearing here keeps it set only while a header is hovered.
            highlight.hover_match = None;
            // Likewise the scrub line: cleared here (after the plot read last
            // frame's value) so it stays set only while the sky-trails window
            // below is driving a scrub, and vanishes as soon as it closes.
            highlight.scrub_time = None;
            self.query_window.show(
                ui.ctx(),
                RunInputs {
                    jamming: &jamming_query_values,
                    geomagnetic: &geomagnetic_series,
                    tec: &tec_series,
                    loaded_files: loaded_files.view(),
                    visibility: tree.visibility(),
                    filter,
                    snap_errors: &snap_error_values,
                },
                *display_mask,
                highlight,
                &mut gt_side_panel::widgets::PointClickRequests {
                    map_center: map_center_request,
                    popup_pos: popup_pos_request,
                },
                reveal_query_matches_request,
            );
            self.sky_trails_window.show(
                ui.ctx(),
                loaded_files.files(),
                plot_state.analysis.elevation_mask_deg,
                highlight,
            );
            let recording_names =
                RecordingNames::resolve(loaded_files.view(), recording_name_template);
            self.log_viewer.show(
                ui.ctx(),
                &mut self.logs,
                LogViewerContext {
                    recordings: loaded_files.view(),
                    recording_names: &recording_names,
                    map_center_request,
                    log_hover,
                    requests: &mut self.log_viewer_requests,
                    write_access: self.pending_writes.write_access(),
                    dialog_open: self.association_dialog.is_some(),
                },
            );
        });
    }

    fn show_snap_prompts(&mut self, ui: &egui::Ui) {
        // Auto mode armed without acknowledged uploads (the checkbox was
        // enabled, or the server host changed): consent is asked on the
        // first load with a snappable track, before anything is sent.
        if self.snap_settings.auto_snap == Some(true)
            && !self.snap_settings.consent_granted()
            && !self.snap_consent_prompt
            && self.any_snappable_track()
        {
            self.snap_consent_prompt = true;
        }

        if let Some(prompt) = self.snap_replace_prompt {
            let costing_name = Self::costing_from_choice(prompt.choice).display_name();
            match show_snap_replace_dialog(ui, costing_name) {
                Some(SnapReplaceChoice::SnapAgain) => {
                    self.snap_replace_prompt = None;
                    self.snap_tracks_as(vec![prompt.track_ref], prompt.choice);
                }
                Some(SnapReplaceChoice::Cancel) => self.snap_replace_prompt = None,
                None => {}
            }
        }

        if let Some(prompt) = self.snap_scope_prompt {
            let costing_name = Self::costing_from_choice(prompt.choice).display_name();
            let counts = self.snap_scope_counts(prompt);
            match show_snap_scope_dialog(ui, costing_name, counts) {
                Some(SnapScopeChoice::Snap(scope)) => {
                    self.snap_scope_prompt = None;
                    let track_refs = self.scoped_track_refs(prompt.fi, scope);
                    self.snap_tracks_as(track_refs, prompt.choice);
                }
                Some(SnapScopeChoice::Cancel) => self.snap_scope_prompt = None,
                None => {}
            }
        }

        if self.snap_consent_prompt {
            let ask_auto = self.snap_settings.auto_snap.is_none();
            match show_snap_consent_dialog(ui, &self.snap_settings.server_url, ask_auto) {
                Some(SnapConsentChoice::Accepted { auto_snap }) => {
                    self.snap_settings.acknowledge_consent();
                    if let Some(auto) = auto_snap {
                        self.snap_settings.auto_snap = Some(auto);
                    }
                    self.snap_consent_prompt = false;
                    // Run the request parked while consent was pending.
                    let parked = std::mem::take(&mut self.pending_snap);
                    self.run_snap_request(parked);
                    self.snap_auto_sweep = true;
                }
                Some(SnapConsentChoice::Declined) => {
                    // The acknowledgment stays unset (the next manual
                    // trigger re-prompts), but the auto choice persists as
                    // off: declined consent never leaves auto uploads armed.
                    self.snap_settings.auto_snap = Some(false);
                    self.snap_consent_prompt = false;
                    self.pending_snap = PendingSnapRequest::default();
                }
                None => {}
            }
        } else if self.snap_settings.auto_snap.is_none()
            && self.snap_settings.consent_granted()
            && self.any_snappable_track()
        {
            // Uploads were acknowledged before auto mode existed: ask the
            // mode choice once, before anything would auto-upload.
            if let Some(choice) = show_snap_auto_prompt(ui, &self.snap_settings.server_url) {
                self.snap_settings.auto_snap = Some(choice == SnapAutoChoice::Automatic);
                self.snap_auto_sweep = true;
            }
        }
    }

    fn show_loading_progress_overlay(&mut self, ui: &egui::Ui) {
        // Loading progress overlay in the bottom-right corner. Shows in-flight
        // jobs with a live elapsed timer, plus recently completed jobs that fade
        // out over ~3 seconds so the user can see how long it took.
        let now = ui.ctx().input(|i| i.time);
        let any_finishing = !self.loader.finishing_jobs.is_empty();
        let opening_databases =
            self.storage_open.databases_pending() == Some(DatabasesPending::Opening);
        self.loader.expire_finished(now);

        if !self.loader.loading_jobs.is_empty() || any_finishing || opening_databases {
            // Keep repainting while jobs are active or fading.
            ui.ctx().request_repaint();

            Window::new("##loading_progress")
                .title_bar(false)
                .resizable(false)
                .anchor(egui::Align2::RIGHT_BOTTOM, [-8.0, -8.0])
                .show(ui.ctx(), |ui| {
                    ui.set_min_width(LOADING_OVERLAY_MIN_WIDTH);
                    // Cap the width so a long recording name truncates.
                    ui.set_max_width(LOADING_OVERLAY_MAX_WIDTH);

                    // The overlay grows upward from the corner, and many
                    // concurrent loads reach past the top of the screen.
                    egui::ScrollArea::vertical()
                    .id_salt("loading_progress_jobs")
                    .show(ui, |ui| {
                    if opening_databases {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label(RichText::new(OPENING_DATABASES).strong());
                        });
                        ui.add_space(2.0);
                    }

                    for job in &self.loader.loading_jobs {
                        let elapsed = job.started_at.elapsed().as_secs_f32();
                        Sides::new().shrink_left().truncate().show(
                            ui,
                            |ui| {
                                ui.spinner();
                                ui.add(
                                    Label::new(RichText::new(&job.filename).strong()).truncate(),
                                );
                            },
                            |ui| {
                                ui.label(RichText::new(format!("{elapsed:.1}s")).small().weak());
                            },
                        );
                        ui.add(
                            ProgressBar::new(job.progress)
                                .animate(true)
                                .desired_width(240.0)
                                .text(job.stage),
                        );
                        ui.add_space(2.0);
                    }

                    for job in &self.loader.finishing_jobs {
                        let since = (now - job.completed_at) as f32;
                        #[expect(
                            clippy::cast_sign_loss,
                            reason = "fade_frac is clamped to [0, 1] before multiplying by 255"
                        )]
                        let alpha = if since < FINISHED_JOB_FADE_START_SECS {
                            255_u8
                        } else {
                            let fade_secs = FINISHED_JOB_EXPIRE_SECS - FINISHED_JOB_FADE_START_SECS;
                            let fade =
                                1.0 - ((since - FINISHED_JOB_FADE_START_SECS) / fade_secs).min(1.0);
                            (fade * 255.0) as u8
                        };
                        let color = egui::Color32::from_rgba_unmultiplied(140, 210, 140, alpha);
                        let weak_color =
                            egui::Color32::from_rgba_unmultiplied(120, 170, 120, alpha);
                        Sides::new().shrink_left().truncate().show(
                            ui,
                            |ui| {
                                ui.label(RichText::new(ICON_CHECK).color(color).small());
                                ui.add(
                                    Label::new(RichText::new(&job.filename).color(color).strong())
                                        .truncate(),
                                );
                            },
                            |ui| {
                                ui.label(
                                    RichText::new(format!("{:.1}s", job.elapsed_secs))
                                        .color(weak_color)
                                        .small(),
                                );
                            },
                        );
                        ui.add_space(2.0);
                    }
                    });
                });
        }
    }

    fn show_load_error_bar(&mut self, ui: &mut egui::Ui) {
        let mut dismiss = false;
        ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
            egui::warn_if_debug_build(ui);
            self.show_read_only_marker(ui);
            if let Some(error) = &self.load_error {
                ui.horizontal(|ui| {
                    ui.colored_label(
                        gt_ui_theme::error_indicator(ui.visuals().dark_mode),
                        format!("{ICON_WARNING} {error}"),
                    );
                    dismiss = ui.small_button(ICON_X).clicked();
                });
            }
        });
        if dismiss {
            self.load_error = None;
        }
    }

    fn apply_pending_unload_and_removal(&mut self, ui: &egui::Ui) {
        // Unload (context menu / Delete key): remove items from the view only.
        // The recordings stay in history, so no confirmation is needed.
        let unloaded = {
            let mut refmut = self.shared.borrow_mut();
            let s = &mut *refmut;
            if let Some(items) = s.tree.pending_unload.take() {
                modals::execute_delete(&items, &mut s.loaded_files, &mut s.tree);
                s.plot_state.rebuild_all(&s.loaded_files);
                Some(items.len())
            } else {
                None
            }
        };
        if let Some(count) = unloaded {
            self.on_track_indices_changed();
            log::info!("Unloaded {count} item(s) from view");
        }

        let remove_outcome = {
            let write_access = self.pending_writes.write_access();
            let mut refmut = self.shared.borrow_mut();
            let s = &mut *refmut;
            // Resolved only while the dialog is up. The map and plot resolve
            // their own names each frame.
            let outcome = s.tree.delete_confirm.is_some().then(|| {
                let recording_names =
                    RecordingNames::resolve(s.loaded_files.view(), &s.recording_name_template);
                show_delete_confirmation(
                    ui,
                    &mut s.tree,
                    &mut s.loaded_files,
                    &recording_names,
                    write_access,
                )
            });
            let outcome = outcome.flatten();
            if outcome.is_some() {
                s.plot_state.rebuild_all(&s.loaded_files);
            }
            outcome
        };
        if let Some(outcome) = remove_outcome {
            self.on_track_indices_changed();
            self.apply_remove_outcome(&outcome);
        }
    }

    /// Load a drop or a paste, once the databases it is stored in and
    /// resolved against are open.
    pub(in crate::app) fn handle_dropped_bytes(&mut self, bytes: Arc<[u8]>, name: &str) {
        if let Some(queued_loads) = self.storage_open.queued_loads_mut() {
            queued_loads.push(QueuedLoad::Bytes {
                bytes,
                name: name.to_owned(),
            });
            return;
        }
        handle_dropped_bytes_dispatch(&mut self.loader, bytes, name, self.processing_config);
    }
}

/// Sends dropped bytes to the recording loader or the log parser, deciding by
/// content: anything that is not HDF5 is read as a log, lossily decoded where
/// it is not UTF-8.
fn handle_dropped_bytes_dispatch(
    loader: &mut LoadJobs,
    bytes: Arc<[u8]>,
    name: &str,
    config: SegmentationConfig,
) {
    const HDF5_MAGIC: &[u8] = b"\x89HDF\r\n\x1a\n";
    if bytes.starts_with(HDF5_MAGIC) {
        let filename = if name.is_empty() {
            "dropped.gtd".to_owned()
        } else {
            name.to_owned()
        };
        loader.spawn_gtd_bytes(bytes, filename, config);
    } else {
        // The log takes its name from its first entry when the drop carries
        // no file name, as pasted text does.
        let filename = (!name.is_empty()).then(|| name.to_owned());
        loader.spawn_log_bytes(bytes, filename);
    }
}

/// Covers the window with the hint stating every way a log gets in, shown
/// while a file is dragged over the app.
fn show_drop_hint_overlay(ctx: &egui::Context) {
    if ctx.input(|input| input.raw.hovered_files.is_empty()) {
        return;
    }
    let window = ctx.content_rect();
    egui::Area::new(egui::Id::new("drop_hint_overlay"))
        .order(egui::Order::Foreground)
        .fixed_pos(window.min)
        .show(ctx, |ui| {
            ui.set_min_size(window.size());
            ui.painter().rect_filled(
                window,
                0.0,
                ui.visuals()
                    .window_fill
                    .gamma_multiply(DROP_OVERLAY_SCRIM_OPACITY),
            );
            ui.centered_and_justified(|ui| {
                ui.heading(log_viewer::LOG_LOAD_HINT);
            });
        });
}

/// Extract the GPS timestamp of the map-hovered TPV point (if any) so the plot
/// can draw a vertical cursor at the corresponding time.
fn extract_map_hover_time(
    files: &[LoadedFile],
    highlight: &MapHighlight,
) -> Option<chrono::DateTime<chrono::Utc>> {
    let HighlightScope::Point(point_ref) = highlight.hover? else {
        return None;
    };
    if point_ref.category != DataCategory::Tpv {
        return None;
    }
    point_ref
        .track
        .fi
        .get(files)
        .and_then(|f| point_ref.track.index.get(&f.tracks))
        .and_then(|t| point_ref.point_index.get(&t.points))
        .map(|p| p.tpv.time().utc())
}

/// The time span of the match hovered in the query results table (first to
/// last matched point), for the plot's shaded band.
fn extract_match_hover_time_range(
    files: &[LoadedFile],
    highlight: &MapHighlight,
) -> Option<(chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>)> {
    let hm = highlight.hover_match?;
    let track = hm
        .track
        .fi
        .get(files)
        .and_then(|f| hm.track.index.get(&f.tracks))?;
    let time = |pi: usize| track.points.get(pi).map(|p| p.tpv.time().utc());
    Some((time(hm.start)?, time(hm.end.checked_sub(1)?)?))
}

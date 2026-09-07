//! The harness every map test draws through: the scene a case builds, the
//! per-frame state it draws from, and the recordings and overlays it puts on
//! the map.
//!
//! The crate's own tests reach it as `crate::test_util`. The integration test
//! binaries reach it as `gt_map::test_util`, through the `test-util` feature
//! gt-map's dev-dependency on itself enables.

#![expect(
    clippy::expect_used,
    reason = "the harness is not covered by clippy's in-test relaxations"
)]

use std::ops::Range;
use std::sync::Arc;

use chrono::{DateTime, Duration, Utc};
use gt_filter::GlobalFilter;
use gt_ionex::TecInstantSelection;
use gt_ionex::maps::GlobalIonosphereMaps;
use gt_jam::dataset::JamDataset;
use gt_jam::day_selection::DaySelection;
use gt_jam::wire::HexObservation;
use gt_loaded_files::RecordingNames;
use gt_logfile::{LogText, ParsedLog};
use gt_test_utils::{
    By, HarnessInteraction as _, NodeT as _, Queryable as _, TestHarness, TestHarnessBuilder,
};
use gt_types::{
    DataCategory, EventMarker, FileIdx, FileSource, FixRef, Latitude, LoadedFile, LoadedTrack,
    Longitude, MercPoint, NavPoint, PointIdx, TimeRange, TrackIdx, TrackRef,
};
use gt_ui_types::{
    DataPointRef, DisplayCategory, DisplayMask, DrawLayer, EventMarkerVisibility,
    GeneratedMarkerVisibility, LoadedLogId, LogMatch, LogMatchColor, LogMatchGlyph, LogMatchHover,
    LogMatchLayer, LogMatchSource, LogMatches, MapHighlight, MatchRevealTarget, PointWindowFolds,
    QueryMatches, SkyGlyphVariant, SnappedEdgeInfo, SnappedEdgeSpan, SnappedSegment,
    SnappedTrackGeometry, SnappedTracks, TrackDataVisibility, TrackRanges,
    TrackSpaceWeatherWarning, WarningLevelExplanation,
};

use crate::{
    MapAction, MapDrawContext, NavMap, SpaceWeatherIndicator, TecHeatmapSnapshot, TecLayer,
    TileAccess, ViewportBounds, icon_mesh,
};

/// The viewport every case draws into, in logical pixels.
pub const VIEWPORT: egui::Vec2 = egui::vec2(800.0, 600.0);

/// Frames a case runs before it reads what the map drew: the first one frames
/// the recording, and the rest settle the load animation.
pub const FRAMES_TO_SETTLE: usize = 8;

/// Frames a case runs after it moves the pointer, past egui's hover delay and
/// the layout of the tooltip that opens.
pub const TOOLTIP_SETTLE_FRAMES: usize = 60;

/// [`TestHarness::builder`] plus the GPU icon pipeline the application
/// installs at startup.
pub fn harness_builder<'a>() -> TestHarnessBuilder<'a> {
    TestHarness::builder()
        .render_state_hook(icon_mesh::gpu::install_embedded_library_without_dithering)
}

/// The middle of the viewport, where the pointer reaches what the camera is
/// centred on.
pub fn viewport_center() -> egui::Pos2 {
    egui::Rect::from_min_size(egui::Pos2::ZERO, VIEWPORT).center()
}

/// The map's own default centre, which it keeps while no fit runs.
const CENTER_LAT: f64 = 55.676;
pub const CENTER_LON: f64 = 12.565;

/// Longitude between consecutive fixes of a walking track, about 63 m at this
/// latitude. The whole track draws without a fit: thirty of these steps span a
/// third of the viewport at the map's default zoom of 16.
pub const WALKING_STEP_DEGREES: f64 = 0.001;

/// The instant every recording here starts at.
pub fn epoch() -> DateTime<Utc> {
    DateTime::UNIX_EPOCH + Duration::seconds(1_700_000_000)
}

pub fn track0() -> TrackRef {
    TrackRef::new(FileIdx::new(0), TrackIdx::new(0))
}

/// The element at `point_index` of the first track of the first recording.
pub fn point_ref(category: DataCategory, point_index: usize) -> DataPointRef {
    point_ref_in(FileIdx::new(0), category, point_index)
}

/// The element at `point_index` of the first track of the recording at `file`.
pub fn point_ref_in(file: FileIdx, category: DataCategory, point_index: usize) -> DataPointRef {
    DataPointRef {
        track: TrackRef::new(file, TrackIdx::new(0)),
        category,
        point_index: PointIdx::new(point_index),
    }
}

/// One completed run whose single draw layer covers `points` of `track`.
pub fn a_run_drawing(track: TrackRef, points: Range<usize>) -> QueryMatches {
    QueryMatches {
        draws: vec![DrawLayer {
            color: 0,
            ranges: TrackRanges::from_iter([(track, vec![points])]),
        }],
        run: 1,
        ..QueryMatches::default()
    }
}

/// A fix `index` steps east of the map's default centre, recorded `index`
/// minutes after the epoch.
fn fix_at(index: usize, step_degrees: f64) -> NavPoint {
    let time = epoch() + Duration::minutes(index as i64);
    let tpv = gt_types::TimePositionVelocity::builder()
        .time(gt_types::GpsTime::from_utc(time))
        .lat(Latitude::new(CENTER_LAT))
        .lon(Longitude::new(CENTER_LON + index as f64 * step_degrees))
        .build();
    NavPoint::new(tpv, None)
}

/// One file over one track of `count` fixes, a minute apart, walking east in
/// steps of `step_degrees`. The metadata has the time range and the duration
/// the track filter reads.
pub fn a_recording_of(count: usize, step_degrees: f64) -> Vec<LoadedFile> {
    let points: Vec<NavPoint> = (0..count).map(|i| fix_at(i, step_degrees)).collect();
    let first = points.first().map_or_else(epoch, |p| p.tpv.time().utc());
    let last = points.last().map_or_else(epoch, |p| p.tpv.time().utc());
    let track = LoadedTrack {
        metadata: gt_types::TrackMetadata {
            duration: last - first,
            time_range: TimeRange::new(first, last),
            tpv_count: points.len(),
            ..gt_test_utils::empty_track_metadata()
        },
        ..gt_test_utils::loaded_track_with_points(points)
    };
    vec![LoadedFile {
        metadata: gt_test_utils::empty_file_metadata(),
        tracks: vec![track],
        event_marker_styles: rustc_hash::FxHashMap::default(),
        orphaned_event_markers: Vec::new(),
        source: FileSource::GtdPath(std::path::PathBuf::from("recording.gtd")),
        load_warnings: Vec::new(),
    }]
}

/// A window that keeps the fixes up to and including `index`.
pub fn window_ending_at(index: usize) -> GlobalFilter {
    GlobalFilter {
        time_end: Some(epoch() + Duration::minutes(index as i64)),
        ..GlobalFilter::default()
    }
}

/// The per-frame state a [`MapDrawContext`] borrows, owned so a case spells
/// out only the inputs it is about and takes the defaults for the rest.
pub struct DrawState {
    pub recording_names: RecordingNames,
    pub filter: GlobalFilter,
    pub event_marker_visibility: EventMarkerVisibility,
    pub generated_marker_visibility: GeneratedMarkerVisibility,
    pub display_mask: DisplayMask,
    pub sky_glyph_variant: SkyGlyphVariant,
    pub point_window_folds: PointWindowFolds,
    pub highlight: MapHighlight,
    pub day_selection: DaySelection,
    pub tec_instant: TecInstantSelection,

    /// The archived maps and the instant the heatmap draws them at, `None`
    /// while the map draws no heatmap.
    pub tec_snapshot: Option<(GlobalIonosphereMaps, DateTime<Utc>)>,

    pub log_matches: LogMatches,
    pub log_hover: LogMatchHover,
    pub clicked_log_glyph: Option<LogMatchGlyph>,
    pub space_weather_warnings: Vec<TrackSpaceWeatherWarning>,
    pub space_weather_levels: Vec<WarningLevelExplanation>,

    /// The position the camera is held on, `None` while it stays where the
    /// frames so far put it.
    pub center_request: Option<(f64, f64)>,
}

impl Default for DrawState {
    fn default() -> Self {
        Self {
            recording_names: RecordingNames::default(),
            filter: GlobalFilter::default(),
            event_marker_visibility: EventMarkerVisibility::default(),
            generated_marker_visibility: GeneratedMarkerVisibility::default(),
            display_mask: DisplayMask::default(),
            sky_glyph_variant: SkyGlyphVariant::default(),
            point_window_folds: PointWindowFolds::default(),
            highlight: MapHighlight::default(),
            day_selection: DaySelection::new(None, gt_jam::calendar::today_utc()),
            tec_instant: TecInstantSelection::new(None, epoch().date_naive()),
            tec_snapshot: None,
            log_matches: LogMatches::default(),
            log_hover: LogMatchHover::default(),
            clicked_log_glyph: None,
            space_weather_warnings: Vec::new(),
            space_weather_levels: Vec::new(),
            center_request: None,
        }
    }
}

impl DrawState {
    /// The context for one [`NavMap::draw`] call, with every overlay absent.
    /// A caller sets what its case is about with struct update syntax.
    pub fn context<'a>(
        &'a mut self,
        files: &'a [LoadedFile],
        visibility: &'a TrackDataVisibility,
    ) -> MapDrawContext<'a> {
        MapDrawContext {
            files,
            recording_names: &self.recording_names,
            snapped_tracks: None,
            jamming_dataset: None,
            tec: TecLayer {
                snapshot: self
                    .tec_snapshot
                    .as_ref()
                    .map(|(maps, instant)| TecHeatmapSnapshot {
                        maps,
                        instant: *instant,
                    }),
                instant: &mut self.tec_instant,
                empty_reason: None,
            },
            query_matches: None,
            log_matches: &self.log_matches,
            log_hover: &mut self.log_hover,
            clicked_log_glyph: &mut self.clicked_log_glyph,
            empty_reason: None,
            space_weather: SpaceWeatherIndicator {
                track_warnings: &self.space_weather_warnings,
                levels: &self.space_weather_levels,
                tec_deviation_caveat: &gt_ionex::text::DEVIATION_REFERENCE_CAVEAT,
            },
            filter: &self.filter,
            visibility,
            event_marker_visibility: &self.event_marker_visibility,
            generated_marker_visibility: &self.generated_marker_visibility,
            display_mask: &mut self.display_mask,
            day_selection: &mut self.day_selection,
            highlight: &mut self.highlight,
            sky_glyph_variant: &mut self.sky_glyph_variant,
            point_window_folds: &mut self.point_window_folds,
            center_request: self.center_request,
            zoom_to_visible: false,
            reveal_query_matches: None,
            sticky_pos_override: None,
        }
    }
}

/// What a frame draws over the recordings, each layer as the application hands
/// it to the map.
#[derive(Default)]
pub struct Overlays {
    pub snapped_tracks: Option<SnappedTracks>,
    pub jamming_dataset: Option<JamDataset>,
    pub query_matches: Option<QueryMatches>,
    pub reveal: Option<MatchRevealTarget>,
}

/// The recordings and overlays one case draws, the canvas it draws them on,
/// and where its camera is held.
pub struct MapScene {
    files: Vec<LoadedFile>,
    tiles: TileAccess,
    size: egui::Vec2,
    dark_mode: Option<bool>,
    zoom: Option<f64>,
    draw: DrawState,
    overlays: Overlays,
}

impl MapScene {
    /// `files` on a map reaching no tile server, with no overlay and the
    /// camera on the fit the load gives it.
    pub fn of(files: Vec<LoadedFile>) -> Self {
        Self {
            files,
            tiles: TileAccess::Offline,
            size: VIEWPORT,
            dark_mode: None,
            zoom: None,
            draw: DrawState::default(),
            overlays: Overlays::default(),
        }
    }

    /// The base layer the map draws under the recordings.
    pub fn tiles(mut self, tiles: TileAccess) -> Self {
        self.tiles = tiles;
        self
    }

    pub fn size(mut self, size: egui::Vec2) -> Self {
        self.size = size;
        self
    }

    /// Draws under dark or light visuals, rather than egui's default.
    pub fn theme(mut self, dark_mode: bool) -> Self {
        self.dark_mode = Some(dark_mode);
        self
    }

    /// Holds the camera at `zoom`, which overrides the fit on every frame.
    pub fn zoomed_to(mut self, zoom: f64) -> Self {
        self.zoom = Some(zoom);
        self
    }

    /// Puts what is drawn at `position`, in degrees, at [`viewport_center`]:
    /// the camera is held there from the first frame on.
    pub fn centred_on(mut self, position: (f64, f64)) -> Self {
        self.draw.center_request = Some(position);
        self
    }

    /// Draws `dataset` and shows the interference category, which a fresh
    /// install hides.
    pub fn showing_the_interference_layer(mut self, dataset: JamDataset) -> Self {
        self.overlays.jamming_dataset = Some(dataset);
        self.draw
            .display_mask
            .set_visible(DisplayCategory::JammingHexes, true);
        self
    }

    /// Hides the fix icons, which takes the fixes off the map and out of the
    /// hit test.
    pub fn hiding_the_fix_icons(mut self) -> Self {
        self.draw
            .display_mask
            .set_visible(DisplayCategory::TrackPoints, false);
        self
    }

    pub fn draw_state(mut self, set: impl FnOnce(&mut DrawState)) -> Self {
        set(&mut self.draw);
        self
    }

    pub fn overlays(mut self, set: impl FnOnce(&mut Overlays)) -> Self {
        set(&mut self.overlays);
        self
    }

    /// Renders the scene over [`FRAMES_TO_SETTLE`] frames, which frame the
    /// recordings and settle the load animation.
    pub fn render(self) -> RenderedMap {
        let mut rendered = self.build();
        for _ in 0..FRAMES_TO_SETTLE {
            rendered.harness.run();
        }
        rendered
    }

    /// Renders the single frame the scene loads on, which is where a case
    /// reads an animation at its start.
    pub fn render_one_frame(self) -> RenderedMap {
        let mut rendered = self.build();
        rendered.harness.step();
        rendered
    }

    fn build(self) -> RenderedMap {
        let visibility = TrackDataVisibility::from_loaded(&self.files);
        let mut builder = harness_builder().size(self.size);
        if let Some(dark_mode) = self.dark_mode {
            builder = builder.theme(dark_mode);
        }
        let harness = builder.ui_state(
            |ui,
             MapSceneState {
                 map,
                 tiles,
                 zoom,
                 draw,
                 overlays,
                 files,
                 visibility,
                 returned_action,
             }: &mut MapSceneState| {
                let map = map.get_or_insert_with(|| NavMap::new(ui.ctx().clone(), tiles.clone()));
                if let Some(zoom) = *zoom {
                    map.map_memory
                        .set_zoom(zoom)
                        .expect("a zoom inside the map's range");
                }
                let returned = map.draw(
                    ui,
                    MapDrawContext {
                        snapped_tracks: overlays.snapped_tracks.as_ref(),
                        jamming_dataset: overlays.jamming_dataset.as_ref(),
                        query_matches: overlays.query_matches.as_ref(),
                        reveal_query_matches: overlays.reveal.clone(),
                        ..draw.context(files, visibility)
                    },
                );
                if returned.is_some() {
                    *returned_action = returned;
                }
            },
            MapSceneState {
                map: None,
                tiles: self.tiles,
                zoom: self.zoom,
                draw: self.draw,
                overlays: self.overlays,
                files: self.files,
                visibility,
                returned_action: None,
            },
        );
        RenderedMap { harness }
    }
}

/// Everything one rendered frame reads. It is carried across frames: the
/// renderers read the state the previous frame left.
pub struct MapSceneState {
    map: Option<NavMap>,
    tiles: TileAccess,
    zoom: Option<f64>,
    draw: DrawState,
    overlays: Overlays,
    files: Vec<LoadedFile>,
    visibility: TrackDataVisibility,
    returned_action: Option<MapAction>,
}

/// One hover label the map has open, and the screen rect it is drawn in.
pub struct HoverLabel {
    pub rect: egui::Rect,
    pub text: String,
}

/// A rendered map a case drives frame by frame, reading what each frame drew
/// and what it left open.
pub struct RenderedMap {
    pub harness: TestHarness<'static, MapSceneState>,
}

impl RenderedMap {
    /// Renders one more frame, with the pointer where the last one left it.
    pub fn render_one_more_frame(&mut self) {
        self.harness.step();
    }

    /// The map the frames drew, `None` before the first frame.
    pub fn map(&self) -> Option<&NavMap> {
        self.harness.state().map.as_ref()
    }

    pub fn map_mut(&mut self) -> Option<&mut NavMap> {
        self.harness.state_mut().map.as_mut()
    }

    /// The per-frame state the next frame draws from.
    pub fn draw_state(&mut self) -> &mut DrawState {
        &mut self.harness.state_mut().draw
    }

    /// The overlays the next frame draws.
    pub fn overlays(&mut self) -> &mut Overlays {
        &mut self.harness.state_mut().overlays
    }

    /// The geographic bounds of the last frame's viewport, `None` before the
    /// first frame.
    pub fn framed(&self) -> Option<ViewportBounds> {
        self.map().and_then(NavMap::viewport_geo_bounds)
    }

    /// How many shapes the last frame painted.
    ///
    /// A layer that must draw nothing shows up as a difference against the
    /// same frame without it: the count is the whole frame's.
    pub fn shapes_painted(&self) -> usize {
        self.harness.inner.output().shapes.len()
    }

    /// The action the map returned, which the application acts on. `None`
    /// while no frame has returned one.
    pub fn returned_action(&self) -> Option<MapAction> {
        self.harness.state().returned_action
    }

    /// The log hexagon the last frame published under the cursor.
    pub fn hovered_log_glyph(&self) -> Option<LogMatchGlyph> {
        self.harness.state().draw.log_hover.glyph.clone()
    }

    /// The log hexagon a click published for the viewer to open on.
    pub fn clicked_log_glyph(&self) -> Option<LogMatchGlyph> {
        self.harness.state().draw.clicked_log_glyph.clone()
    }

    /// Renders one frame that reads a pointer move to `target`.
    pub fn move_pointer_to(&mut self, target: egui::Pos2) {
        self.harness.inner.hover_at(target);
        self.harness.step();
    }

    /// Moves the pointer to `target` and runs past egui's hover delay, which
    /// is what a tooltip needs to open and lay itself out.
    pub fn hover_at_and_settle(&mut self, target: egui::Pos2) {
        self.harness.inner.hover_at(target);
        for _ in 0..TOOLTIP_SETTLE_FRAMES {
            self.harness.run();
        }
    }

    /// Presses and releases the primary button at `target`, one frame for the
    /// move and one for the click.
    pub fn click_at(&mut self, target: egui::Pos2) {
        self.harness.inner.click_at(target);
    }

    /// [`Self::click_at`] with the secondary button, which opens the map's
    /// context menu on the element under the pointer.
    pub fn secondary_click_at(&mut self, target: egui::Pos2) {
        self.harness.inner.secondary_click_at(target);
    }

    /// Renders one frame that reads a press of Escape, which closes the
    /// disambiguation popup and the context menu.
    pub fn press_escape(&mut self) {
        self.harness.inner.key_press(egui::Key::Escape);
        self.harness.step();
    }

    /// The labels the last frame left open, in text order.
    ///
    /// The lines of one label are joined the way the tooltip stacks them. The
    /// order is the text's and not the screen's: a label's rect is not settled
    /// on the frame it opens.
    pub fn hover_labels(&self) -> Vec<HoverLabel> {
        let mut labels: Vec<HoverLabel> = self
            .open_tooltip_layers()
            .into_iter()
            .map(|(id, rect)| HoverLabel {
                rect,
                text: self.lines_under(id).join("\n"),
            })
            .collect();
        labels.sort_by(|left, right| left.text.cmp(&right.text));
        labels
    }

    /// Whether an egui popup is open, which is the flag
    /// [`MapHighlight::shows_hover_label`] reads. The map's context menu is
    /// one. The disambiguation popup is an [`egui::Area`] and is not.
    pub fn any_popup_is_open(&self) -> bool {
        self.harness.inner.ctx.any_popup_open()
    }

    pub fn disambiguation_popup_is_open(&self) -> bool {
        self.map().is_some_and(NavMap::disambiguation_is_open)
    }

    /// [`Self::hover_labels`] as their texts alone.
    pub fn hover_label_texts(&self) -> Vec<String> {
        self.hover_labels()
            .into_iter()
            .map(|label| label.text)
            .collect()
    }

    /// The texts of the labels the last frame left open, the topmost on the
    /// screen first, which is the order the map stacks them in.
    ///
    /// Only a frame that had every one of those labels open already reads
    /// this way: a label of a tooltip that opened this frame is laid out away
    /// from where the tooltip will be drawn.
    pub fn hover_label_texts_top_to_bottom(&self) -> Vec<String> {
        let mut labels = self.hover_labels();
        labels.sort_by(|left, right| left.rect.top().total_cmp(&right.rect.top()));
        labels.into_iter().map(|label| label.text).collect()
    }

    pub fn snapshot(&mut self, name: &str) {
        self.harness.snapshot_loose(name);
    }

    /// The id and rect of every layer the last frame left open at
    /// [`egui::Order::Tooltip`], which is where all three of the map's label
    /// mechanisms draw.
    fn open_tooltip_layers(&self) -> Vec<(egui::Id, egui::Rect)> {
        self.harness.inner.ctx.memory(|memory| {
            memory
                .areas()
                .visible_layer_ids()
                .into_iter()
                .filter(|layer| layer.order == egui::Order::Tooltip)
                .filter_map(|layer| Some((layer.id, memory.area_rect(layer.id)?)))
                .collect()
        })
    }

    /// The label lines drawn under the accesskit node of the layer `id`,
    /// which resolves them by the tree and not by their rects: a label of a
    /// tooltip that opened this frame is laid out away from where the tooltip
    /// will be drawn.
    fn lines_under(&self, id: egui::Id) -> Vec<String> {
        let accesskit_id = id.accesskit_id();
        self.harness
            .inner
            .query(By::new().predicate(move |node| node.locate().0 == accesskit_id))
            .map(|root| {
                root.children_recursive()
                    .filter(|node| node.accesskit_node().role() == egui::accesskit::Role::Label)
                    .map(|node| node.accesskit_node().value().unwrap_or_default())
                    .collect()
            })
            .unwrap_or_default()
    }
}

/// The moment each fix of the recording was made, in fix order.
fn fix_times(files: &[LoadedFile]) -> Vec<DateTime<Utc>> {
    files
        .first()
        .and_then(|file| file.tracks.first())
        .map(|track| {
            track
                .points
                .iter()
                .map(|point| point.tpv.time().utc())
                .collect()
        })
        .unwrap_or_default()
}

/// Where the map draws the fixes of track 0 of the first recording, in
/// normalized Mercator.
pub fn drawn_positions(files: &[LoadedFile]) -> Vec<MercPoint> {
    files
        .first()
        .and_then(|file| file.tracks.first())
        .and_then(LoadedTrack::placed_points)
        .map(|placed| placed.iter().map(|point| point.merc()).collect())
        .unwrap_or_default()
}

/// The position of the fix at `index`, in degrees. A case centres the camera
/// on it, which draws what sits at that fix in the middle of the viewport.
pub fn fix_position(files: &[LoadedFile], index: usize) -> (f64, f64) {
    files
        .first()
        .and_then(|file| file.tracks.first())
        .and_then(LoadedTrack::placed_points)
        .and_then(|placed| placed.get(index))
        .map(|point| {
            let (lat, lon) = point.resolved_position();
            (lat.as_degrees(), lon.as_degrees())
        })
        .unwrap_or_default()
}

/// A log of one line per fix of the recording, each timestamped at that fix.
pub fn a_log_over(files: &[LoadedFile]) -> Arc<ParsedLog> {
    let text: String = fix_times(files)
        .into_iter()
        .map(|time| {
            format!(
                "{} tracklogd[311]: heading hold engaged\n",
                time.format("%Y-%m-%d %H:%M:%S")
            )
        })
        .collect();
    let parsed = gt_logfile::parse_log(LogText::decode_lossy(text.as_bytes()), epoch())
        .expect("the generated lines carry ISO 8601 timestamps");
    Arc::new(parsed)
}

/// One layer of hexagons over the entries in `entries`, each at the position of
/// the fix it was recorded at, as a layer chip puts them on the map.
pub fn matches_over(
    files: &[LoadedFile],
    log: &Arc<ParsedLog>,
    entries: Range<usize>,
) -> LogMatches {
    let positions = drawn_positions(files);
    let matches: Vec<LogMatch> = entries
        .filter_map(|entry_index| {
            Some(LogMatch {
                merc: *positions.get(entry_index)?,
                entry_index,
                fix: FixRef::new(track0(), PointIdx::new(entry_index)),
            })
        })
        .collect();
    LogMatches::from_layers(vec![LogMatchLayer {
        color: LogMatchColor::LayerSlot {
            index: 0,
            shared: false,
        },
        log: LogMatchSource {
            id: LoadedLogId::new(1),
            parsed: Arc::clone(log),
            display_name: None,
        },
        matches,
    }])
}

/// A click on the fix at `fix_index` reaches two elements, the fix and an
/// event marker: `files` with that marker added at the fix's position and
/// instant.
pub fn with_an_event_marker_on_a_fix(
    mut files: Vec<LoadedFile>,
    fix_index: usize,
) -> Vec<LoadedFile> {
    let marker = files
        .first()
        .and_then(|file| file.tracks.first())
        .and_then(|track| track.points.get(fix_index))
        .and_then(|fix| {
            let (latitude, longitude) = fix.tpv.position()?;
            Some(EventMarker::new(
                fix.tpv.time().utc(),
                "power/boot".to_owned(),
                None,
                latitude,
                longitude,
            ))
        });
    if let Some(marker) = marker
        && let Some(track) = files.first_mut().and_then(|file| file.tracks.first_mut())
    {
        track.event_markers.push(marker);
    }
    files
}

/// The interference layer a case draws: one cell around `position`, over which
/// 2 of 100 aircraft reported low navigation accuracy. The cell covers the
/// whole viewport at the zoom that frames a walking track: an H3 resolution 4
/// cell spans about 22 km.
pub fn an_interference_cell_around(position: (f64, f64)) -> JamDataset {
    let (latitude, longitude) = position;
    let cell = h3o::LatLng::new(latitude, longitude)
        .expect("a position on the globe")
        .to_cell(gt_jam::H3_RESOLUTION);
    JamDataset::new(
        epoch().date_naive(),
        vec![HexObservation {
            cell,
            good: 98,
            bad: 2,
        }],
    )
}

/// Half the length of the snapped edge, in normalized Mercator: about 2 km
/// each way, which crosses the whole viewport at the zoom that frames a
/// walking track.
const SNAPPED_EDGE_HALF_LENGTH_MERC: f64 = 1.0e-4;

/// One straight snapped edge running west to east through `position`, in
/// degrees, matched to a named road whose class, speed limit and surface the
/// edge's hover label states.
pub fn a_snapped_edge_through(position: (f64, f64)) -> SnappedTracks {
    let (latitude, longitude) = position;
    let merc = gt_types::mercator::normalize(Latitude::new(latitude), Longitude::new(longitude));
    let mut snapped = SnappedTracks::default();
    snapped.insert(
        track0(),
        Arc::new(SnappedTrackGeometry {
            segments: vec![SnappedSegment {
                points: vec![
                    MercPoint {
                        x: merc.x - SNAPPED_EDGE_HALF_LENGTH_MERC,
                        y: merc.y,
                    },
                    MercPoint {
                        x: merc.x + SNAPPED_EDGE_HALF_LENGTH_MERC,
                        y: merc.y,
                    },
                ],
                recorded_points: Vec::new(),
                edge_spans: vec![SnappedEdgeSpan {
                    start: 0,
                    end: 2,
                    edge: 0,
                }],
            }],
            edges: vec![SnappedEdgeInfo {
                name: Some("H.C. Andersens Boulevard".to_owned()),
                road_class: Some("Tertiary".to_owned()),
                speed_limit: Some("50 km/h".to_owned()),
                surface: Some("Paved smooth".to_owned()),
            }],
            whiskers: Vec::new(),
        }),
    );
    snapped
}

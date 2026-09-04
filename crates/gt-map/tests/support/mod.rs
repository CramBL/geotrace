//! Shared fixture construction for the gt-map integration test binaries: the
//! headless map they draw frames into, the rendered map a snapshot and a hover
//! label are read from, and the recordings and overlays they draw.

#![allow(dead_code, reason = "shared across binaries with different needs")]
#![expect(
    clippy::expect_used,
    reason = "the helpers beside the tests are not covered by clippy's in-test relaxations"
)]

use std::ops::Range;
use std::sync::Arc;

use chrono::{DateTime, Duration, Utc};
use gt_filter::GlobalFilter;
use gt_jam::dataset::JamDataset;
use gt_jam::day_selection::DaySelection;
use gt_jam::wire::HexObservation;
use gt_loaded_files::RecordingNames;
use gt_logfile::{LogText, ParsedLog};
use gt_map::{
    MapDrawContext, NavMap, SpaceWeatherIndicator, TecLayer, TileAccess, ViewportBounds, icon_mesh,
};
use gt_test_utils::{By, HarnessInteraction as _, NodeT as _, Queryable as _, TestHarness};
use gt_types::{
    EventMarker, FileIdx, FileSource, FixRef, Latitude, LoadedFile, LoadedTrack, Longitude,
    MercPoint, NavPoint, PointIdx, TimeRange, TrackIdx, TrackRef,
};
use gt_ui_types::{
    DisplayCategory, DisplayMask, EventMarkerVisibility, GeneratedMarkerVisibility, LoadedLogId,
    LogMatch, LogMatchColor, LogMatchGlyph, LogMatchHover, LogMatchLayer, LogMatchSource,
    LogMatches, MapHighlight, MatchRevealTarget, PointWindowFolds, QueryMatches, SkyGlyphVariant,
    SnappedEdgeInfo, SnappedEdgeSpan, SnappedSegment, SnappedTrackGeometry, SnappedTracks,
    TrackDataVisibility, TrackSpaceWeatherWarning, WarningLevelExplanation,
};

/// The viewport every case draws into, in logical pixels.
pub const VIEWPORT: egui::Vec2 = egui::vec2(800.0, 600.0);

/// Frames a case runs before it reads what the map drew: the first one frames
/// the recording, and the rest settle the load animation.
pub const FRAMES_TO_SETTLE: usize = 8;

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

/// Owned separately from [`MapDrawContext`] so a case spells out only the
/// per-frame state it is about.
struct DrawState {
    recording_names: RecordingNames,
    filter: GlobalFilter,
    event_marker_visibility: EventMarkerVisibility,
    generated_marker_visibility: GeneratedMarkerVisibility,
    display_mask: DisplayMask,
    sky_glyph_variant: SkyGlyphVariant,
    point_window_folds: PointWindowFolds,
    highlight: MapHighlight,
    day_selection: DaySelection,
    tec_instant: gt_ionex::TecInstantSelection,
    log_matches: LogMatches,
    log_hover: LogMatchHover,
    clicked_log_glyph: Option<LogMatchGlyph>,
    space_weather_warnings: Vec<TrackSpaceWeatherWarning>,
    space_weather_levels: Vec<WarningLevelExplanation>,

    /// The position the camera is held on, `None` while it stays where the
    /// frames so far put it.
    center_request: Option<(f64, f64)>,
}

impl DrawState {
    fn new(filter: GlobalFilter) -> Self {
        Self {
            recording_names: RecordingNames::default(),
            filter,
            event_marker_visibility: EventMarkerVisibility::default(),
            generated_marker_visibility: GeneratedMarkerVisibility::default(),
            display_mask: DisplayMask::default(),
            sky_glyph_variant: SkyGlyphVariant::default(),
            point_window_folds: PointWindowFolds::default(),
            highlight: MapHighlight::default(),
            day_selection: DaySelection::new(None, gt_jam::calendar::today_utc()),
            tec_instant: gt_ionex::TecInstantSelection::new(None, epoch().date_naive()),
            log_matches: LogMatches::default(),
            log_hover: LogMatchHover::default(),
            clicked_log_glyph: None,
            space_weather_warnings: Vec::new(),
            space_weather_levels: Vec::new(),
            center_request: None,
        }
    }

    fn context<'a>(
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
                snapshot: None,
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

/// What one frame hands the map besides the recordings and the filter.
#[derive(Default)]
pub struct Frame<'a> {
    pub snapped_tracks: Option<&'a SnappedTracks>,
    pub query_matches: Option<&'a QueryMatches>,
    pub reveal: Option<MatchRevealTarget>,

    /// The input this frame is read with: a pointer move, a button press.
    pub events: Vec<egui::Event>,
}

/// A headless map over one set of recordings, drawn frame by frame. The map
/// keeps its camera across frames, which lets a case draw once and then
/// request a reveal, the way the query window does.
pub struct HeadlessMap<'a> {
    egui_ctx: egui::Context,
    map: NavMap,
    state: DrawState,
    files: &'a [LoadedFile],
    visibility: TrackDataVisibility,
}

impl<'a> HeadlessMap<'a> {
    pub fn new(files: &'a [LoadedFile], filter: GlobalFilter) -> Self {
        let egui_ctx = egui::Context::default();
        let map = NavMap::new(egui_ctx.clone(), TileAccess::Offline);
        Self {
            egui_ctx,
            map,
            state: DrawState::new(filter),
            files,
            visibility: TrackDataVisibility::from_loaded(files),
        }
    }

    /// Draw one frame, returning how many shapes it painted.
    ///
    /// A layer that must draw nothing shows up as a difference against the
    /// same frame without it: the count is the whole frame's.
    pub fn draw(&mut self, frame: &Frame<'_>) -> usize {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, VIEWPORT)),
            events: frame.events.clone(),
            ..egui::RawInput::default()
        };
        let Self {
            egui_ctx,
            map,
            state,
            files,
            visibility,
        } = self;
        let mut output = egui_ctx.run_ui(input, |ui| {
            map.draw(
                ui,
                MapDrawContext {
                    snapped_tracks: frame.snapped_tracks,
                    query_matches: frame.query_matches,
                    reveal_query_matches: frame.reveal.clone(),
                    ..state.context(files, visibility)
                },
            );
        });
        // The font-texture deltas egui hands out are dropped: nothing here
        // paints to a GPU to apply them to.
        output.textures_delta.clear();
        output.shapes.len()
    }

    /// The geographic bounds of the last frame's viewport, `None` before the
    /// first frame.
    pub fn framed(&self) -> Option<ViewportBounds> {
        self.map.viewport_geo_bounds()
    }

    /// Narrows what the map draws from the next frame on, the way the side
    /// panel's filter does.
    pub fn set_filter(&mut self, filter: GlobalFilter) {
        self.state.filter = filter;
    }

    /// Puts `matches` on the map from the next frame on, the way a log's
    /// layer chips do.
    pub fn set_log_matches(&mut self, matches: LogMatches) {
        self.state.log_matches = matches;
    }

    /// Holds the camera on `position`, in degrees, from the next frame on.
    pub fn center_on(&mut self, position: (f64, f64)) {
        self.state.center_request = Some(position);
    }

    /// The log hexagon the last frame published under the cursor.
    pub fn hovered_log_glyph(&self) -> Option<LogMatchGlyph> {
        self.state.log_hover.glyph.clone()
    }

    /// The log hexagon a click published for the viewer to open on.
    pub fn clicked_log_glyph(&self) -> Option<LogMatchGlyph> {
        self.state.clicked_log_glyph.clone()
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

/// Where the map draws the fixes of the recording, in normalized Mercator.
fn drawn_positions(files: &[LoadedFile]) -> Vec<MercPoint> {
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

/// The recordings and overlays one rendered case draws, and where its camera
/// is held.
pub struct RenderedMapScene {
    files: Vec<LoadedFile>,
    snapped_tracks: Option<SnappedTracks>,
    jamming_dataset: Option<JamDataset>,
    log_matches: LogMatches,
    display_mask: DisplayMask,
    center: Option<(f64, f64)>,
}

impl RenderedMapScene {
    /// `files` with no overlay on the map and the camera on the fit the load
    /// gives it.
    pub fn of(files: Vec<LoadedFile>) -> Self {
        Self {
            files,
            snapped_tracks: None,
            jamming_dataset: None,
            log_matches: LogMatches::default(),
            display_mask: DisplayMask::default(),
            center: None,
        }
    }

    /// Draws `dataset` and shows the interference category, which a fresh
    /// install hides.
    pub fn showing_the_interference_layer(mut self, dataset: JamDataset) -> Self {
        self.jamming_dataset = Some(dataset);
        self.display_mask
            .set_visible(DisplayCategory::JammingHexes, true);
        self
    }

    pub fn with_snapped_tracks(mut self, snapped: SnappedTracks) -> Self {
        self.snapped_tracks = Some(snapped);
        self
    }

    pub fn with_log_matches(mut self, matches: LogMatches) -> Self {
        self.log_matches = matches;
        self
    }

    /// Hides the fix icons, which takes the fixes off the map and out of the
    /// hit test.
    pub fn hiding_the_fix_icons(mut self) -> Self {
        self.display_mask
            .set_visible(DisplayCategory::TrackPoints, false);
        self
    }

    /// Puts what is drawn at `position`, in degrees, at [`viewport_center`]:
    /// the camera is held there from the first frame on.
    pub fn centred_on(mut self, position: (f64, f64)) -> Self {
        self.center = Some(position);
        self
    }

    /// Renders the scene into a map reaching no tile server, run until the
    /// camera and the load animation settle.
    pub fn draw(self) -> RenderedMap {
        let Self {
            files,
            snapped_tracks,
            jamming_dataset,
            log_matches,
            display_mask,
            center,
        } = self;
        let visibility = TrackDataVisibility::from_loaded(&files);
        let mut draw = DrawState::new(GlobalFilter::default());
        draw.display_mask = display_mask;
        draw.log_matches = log_matches;
        draw.center_request = center;
        let mut harness = TestHarness::builder()
            .size(VIEWPORT)
            .render_state_hook(icon_mesh::gpu::install_embedded_library_without_dithering)
            .ui_state(
                |ui,
                 RenderedMapState {
                     map,
                     draw,
                     snapped_tracks,
                     jamming_dataset,
                     files,
                     visibility,
                 }: &mut RenderedMapState| {
                    let map = map
                        .get_or_insert_with(|| NavMap::new(ui.ctx().clone(), TileAccess::Offline));
                    map.draw(
                        ui,
                        MapDrawContext {
                            snapped_tracks: snapped_tracks.as_ref(),
                            jamming_dataset: jamming_dataset.as_ref(),
                            ..draw.context(files, visibility)
                        },
                    );
                },
                RenderedMapState {
                    map: None,
                    draw,
                    snapped_tracks,
                    jamming_dataset,
                    files,
                    visibility,
                },
            );
        for _ in 0..FRAMES_TO_SETTLE {
            harness.run();
        }
        RenderedMap { harness }
    }
}

/// Everything one rendered frame reads. It is carried across frames: the
/// renderers read the state the previous frame left.
struct RenderedMapState {
    map: Option<NavMap>,
    draw: DrawState,
    snapped_tracks: Option<SnappedTracks>,
    jamming_dataset: Option<JamDataset>,
    files: Vec<LoadedFile>,
    visibility: TrackDataVisibility,
}

/// One hover label the map has open, and the screen rect it is drawn in.
pub struct HoverLabel {
    pub rect: egui::Rect,
    pub text: String,
}

/// A rendered map a case drives frame by frame, reading the hover labels each
/// frame left open.
pub struct RenderedMap {
    harness: TestHarness<'static, RenderedMapState>,
}

impl RenderedMap {
    /// Draws one frame that reads a pointer move to `target`.
    pub fn move_pointer_to(&mut self, target: egui::Pos2) {
        self.harness.inner.hover_at(target);
        self.harness.step();
    }

    /// Draws one frame with the pointer where the last one left it.
    pub fn draw_one_more_frame(&mut self) {
        self.harness.step();
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

    /// Draws one frame that reads a press of Escape, which closes the
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
        self.harness
            .state()
            .map
            .as_ref()
            .is_some_and(NavMap::disambiguation_is_open)
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

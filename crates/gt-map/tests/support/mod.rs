//! Shared fixture construction for the gt-map integration test binaries: the
//! headless map they draw frames into, the harness a rendered baseline comes
//! from, and the recordings they draw.

#![allow(dead_code, reason = "shared across binaries with different needs")]

use chrono::{DateTime, Duration, Utc};
use gt_filter::GlobalFilter;
use gt_jam::day_selection::DaySelection;
use gt_loaded_files::RecordingNames;
use gt_map::{
    MapDrawContext, NavMap, SpaceWeatherIndicator, TecLayer, TileAccess, ViewportBounds, icon_mesh,
};
use gt_test_utils::TestHarness;
use gt_types::{
    FileIdx, FileSource, Latitude, LoadedFile, LoadedTrack, Longitude, NavPoint, TimeRange,
    TrackIdx, TrackRef,
};
use gt_ui_types::{
    DisplayMask, EventMarkerVisibility, GeneratedMarkerVisibility, LogMatchGlyph, LogMatchHover,
    LogMatches, MapHighlight, MatchRevealTarget, PointWindowFolds, QueryMatches, SkyGlyphVariant,
    SnappedTracks, TrackDataVisibility, TrackSpaceWeatherWarning, WarningLevelExplanation,
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
/// keeps its camera across frames, which lets a case draw once and then ask
/// for a reveal, the way the query window does.
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

/// A harness that renders `files` into a map reaching no tile server, run
/// until the camera settles on the fit a newly loaded recording gets.
pub fn rendered_map(files: Vec<LoadedFile>) -> TestHarness<'static, Option<NavMap>> {
    let visibility = TrackDataVisibility::from_loaded(&files);
    let mut harness = TestHarness::builder()
        .size(VIEWPORT)
        .render_state_hook(icon_mesh::gpu::install_embedded_library_without_dithering)
        .ui_state(
            move |ui, map: &mut Option<NavMap>| {
                let map =
                    map.get_or_insert_with(|| NavMap::new(ui.ctx().clone(), TileAccess::Offline));
                let mut state = DrawState::new(GlobalFilter::default());
                map.draw(ui, state.context(&files, &visibility));
            },
            None,
        );
    for _ in 0..FRAMES_TO_SETTLE {
        harness.run();
    }
    harness
}

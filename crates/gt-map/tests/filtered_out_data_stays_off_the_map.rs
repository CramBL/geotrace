//! What the global filter keeps off the map: the snapped-track layer of a
//! recording the filter rejects, the snapped vertices and error whiskers its
//! time window hides, and the fixes a fit frames.

use std::sync::Arc;

use chrono::{DateTime, Duration, Utc};
use gt_filter::GlobalFilter;
use gt_jam::day_selection::DaySelection;
use gt_loaded_files::RecordingNames;
use gt_map::{MapDrawContext, NavMap, SpaceWeatherIndicator, TecLayer, TileAccess, ViewportBounds};
use gt_types::mercator::MercPoint;
use gt_types::{
    FileIdx, FileSource, Latitude, LoadedFile, LoadedTrack, Longitude, NavPoint, PointIdx,
    TimeRange, TrackIdx, TrackRef,
};
use gt_ui_types::{
    DisplayMask, EventMarkerVisibility, GeneratedMarkerVisibility, LogMatchHover, LogMatches,
    MapHighlight, PointWindowFolds, SkyGlyphVariant, SnappedSegment, SnappedTrackGeometry,
    SnappedTracks, TrackDataVisibility, WhiskerAnchor,
};

/// The viewport every case here draws into, in logical pixels.
const VIEWPORT: egui::Vec2 = egui::vec2(800.0, 600.0);

/// The map's own default centre, which it keeps while no fit runs.
const CENTER_LAT: f64 = 55.676;
const CENTER_LON: f64 = 12.565;

/// Longitude between consecutive fixes of a walking track, about 63 m at this
/// latitude. Thirty of them span a third of the viewport at the map's default
/// zoom of 16, so the whole track is drawn without a fit.
const WALKING_STEP_DEGREES: f64 = 0.001;

/// Longitude between consecutive fixes of a track recorded in one spot, about
/// 6 cm. A fit over such a track reaches the maximum zoom, which is where the
/// error whiskers draw.
const STANDING_STEP_DEGREES: f64 = 0.000_001;

/// The instant every recording here starts at.
fn epoch() -> DateTime<Utc> {
    DateTime::UNIX_EPOCH + Duration::seconds(1_700_000_000)
}

fn track0() -> TrackRef {
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
fn a_recording_of(count: usize, step_degrees: f64) -> Vec<LoadedFile> {
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
fn window_ending_at(index: usize) -> GlobalFilter {
    GlobalFilter {
        time_end: Some(epoch() + Duration::minutes(index as i64)),
        ..GlobalFilter::default()
    }
}

/// Ten metres north in normalized Mercator at this latitude, offsetting the
/// snapped geometry so it is its own ink beside the recorded track.
const SNAPPED_OFFSET_MERC_Y: f64 = -1.5e-6;

/// Where the map draws the fixes of track 0, in normalized Mercator.
fn drawn_positions(files: &[LoadedFile]) -> Vec<MercPoint> {
    files
        .first()
        .and_then(|file| file.tracks.first())
        .and_then(LoadedTrack::placed_points)
        .map(|placed| placed.iter().map(|point| point.merc()).collect())
        .unwrap_or_default()
}

/// Snapped road geometry for track 0: one polyline beside the recorded fixes
/// in `fixes`, one vertex per fix.
fn snapped_polyline_over(files: &[LoadedFile], fixes: std::ops::Range<usize>) -> SnappedTracks {
    let points: Vec<MercPoint> = drawn_positions(files)
        .get(fixes.clone())
        .unwrap_or_default()
        .iter()
        .map(|point| MercPoint {
            x: point.x,
            y: point.y + SNAPPED_OFFSET_MERC_Y,
        })
        .collect();
    let mut snapped = SnappedTracks::default();
    snapped.insert(
        track0(),
        Arc::new(SnappedTrackGeometry {
            segments: vec![SnappedSegment {
                points,
                recorded_points: fixes.map(PointIdx::new).collect(),
                edge_spans: Vec::new(),
            }],
            edges: Vec::new(),
            whiskers: Vec::new(),
        }),
    );
    snapped
}

/// Error whiskers for track 0: one per recorded fix in `fixes`, reaching from
/// the fix to a snapped position north of it.
fn whiskers_over(files: &[LoadedFile], fixes: std::ops::Range<usize>) -> SnappedTracks {
    let whiskers: Vec<WhiskerAnchor> = drawn_positions(files)
        .into_iter()
        .enumerate()
        .filter(|(index, _)| fixes.contains(index))
        .map(|(index, point)| WhiskerAnchor {
            point: PointIdx::new(index),
            snapped: MercPoint {
                x: point.x,
                y: point.y + SNAPPED_OFFSET_MERC_Y,
            },
        })
        .collect();
    let mut snapped = SnappedTracks::default();
    snapped.insert(
        track0(),
        Arc::new(SnappedTrackGeometry {
            segments: Vec::new(),
            edges: Vec::new(),
            whiskers,
        }),
    );
    snapped
}

/// The per-frame state a [`MapDrawContext`] borrows, owned so a case spells
/// out only what it is about.
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
    clicked_log_glyph: Option<gt_ui_types::LogMatchGlyph>,
    space_weather_warnings: Vec<gt_ui_types::TrackSpaceWeatherWarning>,
    space_weather_levels: Vec<gt_ui_types::WarningLevelExplanation>,
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
            center_request: None,
            zoom_to_visible: false,
            reveal_query_matches: None,
            sticky_pos_override: None,
        }
    }
}

/// A headless map over one set of recordings, drawn frame by frame.
struct HeadlessMap<'a> {
    egui_ctx: egui::Context,
    map: NavMap,
    state: DrawState,
    files: &'a [LoadedFile],
    visibility: TrackDataVisibility,
}

impl<'a> HeadlessMap<'a> {
    fn new(files: &'a [LoadedFile], filter: GlobalFilter) -> Self {
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
    /// The count is the whole frame's, so a layer that must draw nothing shows
    /// up as a difference against the same frame without it.
    fn draw(&mut self, snapped: Option<&SnappedTracks>) -> usize {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, VIEWPORT)),
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
                    snapped_tracks: snapped,
                    ..state.context(files, visibility)
                },
            );
        });
        // egui hands out font-texture deltas for a painter to apply. Nothing
        // here paints to a GPU, so they are dropped deliberately.
        output.textures_delta.clear();
        output.shapes.len()
    }

    /// The geographic bounds of the last frame's viewport, `None` before the
    /// first frame.
    fn framed(&self) -> Option<ViewportBounds> {
        self.map.viewport_geo_bounds()
    }
}

/// Shapes one frame paints over the recordings in `files`, under `filter`,
/// with `snapped` handed to the map.
fn shapes_with(
    files: &[LoadedFile],
    filter: GlobalFilter,
    snapped: Option<&SnappedTracks>,
) -> usize {
    HeadlessMap::new(files, filter).draw(snapped)
}

/// A recording the filter rejects puts nothing on the map, and the road
/// geometry it was snapped to is part of that nothing: it is the same
/// recording, drawn beside itself.
#[rstest::rstest]
#[case::the_time_window_is_disjoint_from_the_recording(GlobalFilter {
    time_start: Some(epoch() + Duration::hours(5)),
    ..GlobalFilter::default()
})]
#[case::the_recording_is_shorter_than_the_minimum_duration(GlobalFilter {
    min_duration: Some(Duration::hours(5)),
    ..GlobalFilter::default()
})]
fn a_snapped_track_of_a_filtered_out_recording_is_not_drawn(#[case] filter: GlobalFilter) {
    let files = a_recording_of(30, WALKING_STEP_DEGREES);
    let snapped = snapped_polyline_over(&files, 0..30);

    assert_eq!(
        shapes_with(&files, filter, Some(&snapped)),
        shapes_with(&files, filter, None),
        "the snapped track of a filtered-out recording put ink on the map"
    );
}

/// The time window ends the recorded track at the last fix it keeps, and the
/// snapped track beside it ends there too: the map draws the same ink for the
/// whole snapped geometry as for the stretch the window keeps.
#[test]
fn a_snapped_track_is_not_drawn_past_the_end_of_the_time_window() {
    let files = a_recording_of(30, WALKING_STEP_DEGREES);
    let filter = window_ending_at(14);

    assert_eq!(
        shapes_with(&files, filter, Some(&snapped_polyline_over(&files, 0..30))),
        shapes_with(&files, filter, Some(&snapped_polyline_over(&files, 0..15))),
        "the snapped track was drawn past the end of the time window"
    );
}

/// An error whisker reaches from a recorded fix to where that fix was snapped.
/// The fixes outside the time window are not drawn, so neither are their
/// whiskers.
#[test]
fn an_error_whisker_of_a_fix_outside_the_time_window_is_not_drawn() {
    let files = a_recording_of(30, STANDING_STEP_DEGREES);
    let filter = window_ending_at(14);

    assert_eq!(
        shapes_with(&files, filter, Some(&whiskers_over(&files, 0..30))),
        shapes_with(&files, filter, Some(&whiskers_over(&files, 0..15))),
        "a whisker was drawn at a fix the time window hides"
    );
}

/// The snapped track of a recording the filter keeps is drawn, so the cases
/// above fail for the reason they name.
#[test]
fn a_snapped_track_of_a_kept_recording_is_drawn() {
    let files = a_recording_of(30, WALKING_STEP_DEGREES);
    let snapped = snapped_polyline_over(&files, 0..30);

    assert!(
        shapes_with(&files, GlobalFilter::default(), Some(&snapped))
            > shapes_with(&files, GlobalFilter::default(), None),
        "the snapped track of a kept recording put no ink on the map"
    );
}

/// A fit frames the fixes the time window keeps: fix 4 is the last of them,
/// and fix 10 lies well outside it.
#[test]
fn a_fit_frames_the_fixes_inside_the_time_window() {
    let files = a_recording_of(30, WALKING_STEP_DEGREES);
    let mut map = HeadlessMap::new(&files, window_ending_at(4));

    map.draw(None);

    let framed = map.framed().expect("the map has drawn a frame");
    assert!(
        framed.lon_max > CENTER_LON + 4.0 * WALKING_STEP_DEGREES
            && framed.lon_max < CENTER_LON + 10.0 * WALKING_STEP_DEGREES,
        "the fit framed up to {}° E, and the window ends at fix 4",
        framed.lon_max
    );
}

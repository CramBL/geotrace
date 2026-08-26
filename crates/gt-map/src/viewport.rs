//! Viewport-level geometry and collection: geographic bounds of the
//! visible map, zoom-to-fit, hit-test visibility, and the per-frame
//! collection of spatial points the render plugins draw.

use std::ops::Range;

use gt_filter::{GlobalFilter, point_passes_time_filter, track_passes_filter};
use gt_types::{DataCategory, FileIdx, LoadedFile, NavPoint, SpatialPoint, TrackIdx, TrackRef};
use gt_ui_types::{
    DataPointRef, DisplayCategory, DisplayMask, MapScope, QueryMatches, TrackDataVisibility,
};
use rustc_hash::FxHashMap;
use smallvec::SmallVec;
use walkers::MapMemory;

use crate::tpv_renderer::{self, TrackIconFade};
use crate::transform::{MapScale, MercTransform};

/// Geographic bounding box of the currently visible map viewport.
#[derive(Debug, Clone, Copy)]
pub struct GeoBounds {
    pub lat_min: f64,
    pub lat_max: f64,
    pub lon_min: f64,
    pub lon_max: f64,
}

/// The spatial points inside the viewport, split by category, ready for
/// the render plugins. TPV fixes arrive pre-grouped per track (point
/// indices), which is the shape `TrackLayers` consumes.
///
/// Held across frames as reusable scratch (see [`crate::NavMap`]): [`Self::clear`]
/// empties every buffer while keeping its allocation, so a steady stream of
/// frames at similar zoom reuses the same memory instead of reallocating.
#[derive(Default)]
pub(crate) struct VisiblePoints {
    pub(crate) tpv_by_track: FxHashMap<TrackRef, Vec<usize>>,
    pub(crate) custom: Vec<SpatialPoint>,
    pub(crate) generated: Vec<SpatialPoint>,
    pub(crate) event: Vec<SpatialPoint>,
}

impl VisiblePoints {
    /// Empty every buffer for reuse, keeping capacity. TPV track keys are
    /// retained with emptied index lists so their inner allocations survive
    /// too; an empty list draws nothing, exactly as an absent key would.
    fn clear(&mut self) {
        for indices in self.tpv_by_track.values_mut() {
            indices.clear();
        }
        self.custom.clear();
        self.generated.clear();
        self.event.clear();
    }
}

/// Collect the spatial points inside the current viewport from the global
/// R-tree into `visible`, one list per category. The buffers are cleared
/// first, so a reused [`VisiblePoints`] keeps its allocations across frames.
///
/// TPV points are gated by the frame's [`TrackPlan`]: fix icons of tracks
/// that are disabled, filtered out, TPV-layer-hidden, or classified
/// [`TrackIconFade::AllHidden`] (the quality line stands in) are never
/// drawn, so collecting their viewport points - potentially the entire
/// recording when zoomed out - would be pure allocation waste.
pub(crate) fn collect_visible_points(
    visible: &mut VisiblePoints,
    tree: &rstar::RTree<SpatialPoint>,
    plan: &TrackPlan,
    transform: &MercTransform,
    map_rect: egui::Rect,
) {
    visible.clear();
    let lt = map_rect.left_top();
    let rb = map_rect.right_bottom();
    let aabb = rstar::AABB::from_corners(
        [
            transform.merc_x_from_screen(lt.x),
            transform.merc_y_from_screen(lt.y),
        ],
        [
            transform.merc_x_from_screen(rb.x),
            transform.merc_y_from_screen(rb.y),
        ],
    );
    for sp in tree.locate_in_envelope(aabb) {
        match sp.category {
            DataCategory::Tpv => {
                // Unknown tracks default to collectable, so a stale index
                // can only cost wasted collection, never hidden data.
                if plan
                    .entry(sp.track_ref())
                    .is_none_or(TrackEntry::tpv_collectable)
                {
                    visible
                        .tpv_by_track
                        .entry(sp.track_ref())
                        .or_default()
                        .push(sp.point_index.as_usize());
                }
            }
            DataCategory::CustomMarker => visible.custom.push(*sp),
            DataCategory::GeneratedMarker => visible.generated.push(*sp),
            DataCategory::EventMarker => visible.event.push(*sp),
            _ => {}
        }
    }
}

/// What the renderers will do for one track this frame, derived once per
/// frame in [`TrackPlan::compute`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TrackEntry {
    /// The plain trackline layer is drawn (file and track enabled, track
    /// layer visible, filter passed, category displayed).
    pub(crate) trackline: bool,
    /// Icon-fade classification of the TPV layer. `None` when that layer
    /// is hidden or the track is disabled or filtered out.
    pub(crate) fade: Option<TrackIconFade>,
    /// The track's satellite-label anchors are placement candidates.
    /// Rides the TPV tree toggle but has its own display category, so
    /// labels survive hiding the track points (and vice versa).
    pub(crate) sat_labels: bool,
    /// The track's report-bearing points are sky-glyph candidates. Rides the
    /// track's map visibility and its own display category, independent of
    /// the track-points toggle.
    pub(crate) sky_glyphs: bool,
}

impl TrackEntry {
    /// TPV viewport points are worth collecting only when icons can draw.
    fn tpv_collectable(self) -> bool {
        self.fade.is_some_and(|f| f != TrackIconFade::AllHidden)
    }

    /// No layer draws. The renderer can skip the track outright.
    pub(crate) fn draws_nothing(self) -> bool {
        !self.trackline && self.fade.is_none() && !self.sat_labels && !self.sky_glyphs
    }
}

/// Per-track drawing decisions for one frame, derived once from the zoom
/// level and the visibility/filter state, then shared by viewport
/// collection and the track-layer renderer - a single derivation point, so
/// the decisions cannot drift between consumers.
///
/// Entries are flattened into one inline buffer
/// (`entries[offsets[fi] + ti]`, with `offsets` carrying one trailing end
/// entry), so computing the plan allocates nothing for typical workspace
/// sizes.
pub(crate) struct TrackPlan {
    entries: SmallVec<[TrackEntry; 128]>,
    offsets: SmallVec<[usize; 9]>,
}

impl TrackPlan {
    pub(crate) fn compute(
        files: &[LoadedFile],
        visibility: &TrackDataVisibility,
        filter: &GlobalFilter,
        display_mask: DisplayMask,
        zoom: f64,
    ) -> Self {
        let scale = MapScale::from_zoom(zoom);
        let icon_size = tpv_renderer::base_arrow_size(zoom);
        let mut entries: SmallVec<[TrackEntry; 128]> = SmallVec::new();
        let mut offsets: SmallVec<[usize; 9]> = SmallVec::new();
        for (fi, file) in files.iter().enumerate() {
            offsets.push(entries.len());
            let file_vis = FileIdx::new(fi).get(&visibility.files);
            let file_enabled = file_vis.is_some_and(|fv| fv.enabled);
            for (ti, track) in file.tracks.iter().enumerate() {
                let track_vis = file_vis.and_then(|fv| TrackIdx::new(ti).get(&fv.tracks));
                let enabled = file_enabled
                    && track_vis.is_some_and(|tv| tv.enabled)
                    && track_passes_filter(&track.metadata, filter);
                let tpv_on =
                    enabled && track_vis.is_some_and(|tv| tv.category_visible(DataCategory::Tpv));
                // The fade classification runs last so it is skipped for
                // tracks that are hidden or filtered out anyway.
                let fade = (tpv_on && display_mask.is_visible(DisplayCategory::TrackPoints))
                    .then(|| tpv_renderer::classify_icon_fade(track, scale, icon_size));
                entries.push(TrackEntry {
                    trackline: enabled
                        && track_vis.is_some_and(|tv| tv.category_visible(DataCategory::Track))
                        && display_mask.is_visible(DisplayCategory::Tracks),
                    fade,
                    sat_labels: tpv_on && display_mask.is_visible(DisplayCategory::SatelliteLabels),
                    sky_glyphs: enabled && display_mask.is_visible(DisplayCategory::SkyGlyphs),
                });
            }
        }
        offsets.push(entries.len());
        Self { entries, offsets }
    }

    /// The decisions for `track`. `None` for indices outside the plan.
    pub(crate) fn entry(&self, track: TrackRef) -> Option<TrackEntry> {
        let fi = track.fi.as_usize();
        let (&start, &end) = (self.offsets.get(fi)?, self.offsets.get(fi + 1)?);
        let idx = start + track.index.as_usize();
        if idx >= end {
            return None;
        }
        self.entries.get(idx).copied()
    }
}

/// Bounding box over only the currently **visible** data: tracks with both their
/// file and track enabled, that pass the active filter, counting only the
/// points and markers inside the filter's time window.  Returns `None` if no
/// such data exists.
///
/// This matches what the renderers actually draw, so "zoom to fit" frames the
/// visible data rather than the whole recording. The display mask counts the
/// same way: point ink contributes only while some point-anchored category
/// (tracks, track points, satellite labels) is displayed, custom markers only
/// while theirs is. Snapped tracks deliberately never contribute: they
/// annotate a recorded track that is already framed.
pub(crate) fn compute_visible_bounding_box(
    files: &[LoadedFile],
    visibility: &TrackDataVisibility,
    filter: &GlobalFilter,
    display_mask: DisplayMask,
) -> Option<(f64, f64, f64, f64)> {
    let points_displayed = [
        DisplayCategory::Tracks,
        DisplayCategory::TrackPoints,
        DisplayCategory::SatelliteLabels,
    ]
    .into_iter()
    .any(|c| display_mask.is_visible(c));
    let custom_markers_displayed = display_mask.is_visible(DisplayCategory::CustomMarkers);
    if !points_displayed && !custom_markers_displayed {
        return None;
    }
    let mut min_lat = f64::MAX;
    let mut max_lat = f64::MIN;
    let mut min_lon = f64::MAX;
    let mut max_lon = f64::MIN;
    let mut any = false;

    for (fi, file) in files.iter().enumerate() {
        let Some(file_vis) = visibility.files.get(fi) else {
            continue;
        };
        if !file_vis.enabled {
            continue;
        }
        for (ti, track) in file.tracks.iter().enumerate() {
            let Some(track_vis) = file_vis.tracks.get(ti) else {
                continue;
            };
            if !track_vis.enabled {
                continue;
            }
            if !track_passes_filter(&track.metadata, filter) {
                continue;
            }
            if points_displayed {
                for point in &track.points {
                    if !point_passes_time_filter(point.tpv.time().utc(), filter) {
                        continue;
                    }
                    let lat = point.tpv.lat().as_degrees();
                    let lon = point.tpv.lon().as_degrees();
                    min_lat = min_lat.min(lat);
                    max_lat = max_lat.max(lat);
                    min_lon = min_lon.min(lon);
                    max_lon = max_lon.max(lon);
                    any = true;
                }
            }
            if custom_markers_displayed {
                for marker in &track.custom_markers {
                    if !point_passes_time_filter(marker.time, filter) {
                        continue;
                    }
                    let lat = marker.lat.as_degrees();
                    let lon = marker.lon.as_degrees();
                    min_lat = min_lat.min(lat);
                    max_lat = max_lat.max(lat);
                    min_lon = min_lon.min(lon);
                    max_lon = max_lon.max(lon);
                    any = true;
                }
            }
        }
    }

    if any {
        Some((min_lat, max_lat, min_lon, max_lon))
    } else {
        None
    }
}

/// Bounding box over the points every `draw` layer of `matches` covers, for
/// framing the map on what a query run drew. `None` when no draw layer covers
/// a point of a loaded track.
pub(crate) fn matched_bounding_box(
    files: &[LoadedFile],
    matches: &QueryMatches,
) -> Option<(f64, f64, f64, f64)> {
    let mut bounds = None;
    for layer in &matches.draws {
        for (track_ref, ranges) in &layer.ranges {
            let Some(track) = track_ref.resolve(files) else {
                continue;
            };
            for range in ranges {
                grow_bounds_over(
                    &mut bounds,
                    track.points.get(range.clone()).unwrap_or_default(),
                );
            }
        }
    }
    bounds
}

/// Bounding box over the points of one match, for framing the map on the match
/// a results row points at. `None` when its track is no longer loaded or the
/// range reaches past it.
pub(crate) fn match_bounding_box(
    files: &[LoadedFile],
    track_ref: TrackRef,
    points: &Range<usize>,
) -> Option<(f64, f64, f64, f64)> {
    let track = track_ref.resolve(files)?;
    let mut bounds = None;
    grow_bounds_over(&mut bounds, track.points.get(points.clone())?);
    bounds
}

/// Grows `bounds` over every point of `points`, starting it at the first one
/// when it is still `None`.
fn grow_bounds_over(bounds: &mut Option<(f64, f64, f64, f64)>, points: &[NavPoint]) {
    for point in points {
        let lat = point.tpv.lat().as_degrees();
        let lon = point.tpv.lon().as_degrees();
        *bounds = Some(match *bounds {
            None => (lat, lat, lon, lon),
            Some((min_lat, max_lat, min_lon, max_lon)) => (
                min_lat.min(lat),
                max_lat.max(lat),
                min_lon.min(lon),
                max_lon.max(lon),
            ),
        });
    }
}

/// Compute the geographic bounding box of the given map viewport rect.
///
/// Uses the walkers `Projector` to unproject the four corners of `map_rect`
/// into geographic positions and returns their bounding envelope.
pub(crate) fn compute_viewport_bounds(map_memory: &MapMemory, map_rect: egui::Rect) -> GeoBounds {
    // `my_position` is only used as a fallback when the map is in GPS-follow
    // mode. `center_at()` is always called explicitly, so `detached()` provides
    // the actual center. Fall back to (0, 0) if `detached()` is unset.
    let center = map_memory
        .detached()
        .unwrap_or_else(|| walkers::lat_lon(0.0, 0.0));
    let projector = walkers::Projector::new(map_rect, map_memory, center);

    let corners = [
        map_rect.left_top(),
        map_rect.right_top(),
        map_rect.left_bottom(),
        map_rect.right_bottom(),
    ];

    let mut lat_min = f64::INFINITY;
    let mut lat_max = f64::NEG_INFINITY;
    let mut lon_min = f64::INFINITY;
    let mut lon_max = f64::NEG_INFINITY;

    for corner in corners {
        let pos = projector.unproject(corner.to_vec2());
        let lat = pos.y();
        let lon = pos.x();
        lat_min = lat_min.min(lat);
        lat_max = lat_max.max(lat);
        lon_min = lon_min.min(lon);
        lon_max = lon_max.max(lon);
    }

    GeoBounds {
        lat_min,
        lat_max,
        lon_min,
        lon_max,
    }
}

/// Returns `true` when a spatial point should participate in hover and click
/// detection.
///
/// A trackline and a raw satellite report have no hover target of their own -
/// neither is ever inserted into the spatial index, and neither is clickable.
/// Everything else is hit-testable exactly while the map draws it, queried from
/// [`MapScope::draws`] so hit-testing cannot drift from what is on screen.
pub(crate) fn is_spatial_point_visible(sp: &SpatialPoint, scope: MapScope<'_>) -> bool {
    !matches!(
        sp.category,
        DataCategory::Track | DataCategory::SatelliteReport
    ) && scope.draws(DataPointRef {
        track: sp.track_ref(),
        category: sp.category,
        point_index: sp.point_index,
    })
}

/// Center the map and set the zoom so the given bounding box fills ~80 % of the
/// viewport. Respects walkers' valid zoom range [1, 18].
pub(crate) fn zoom_to_fit(
    map_memory: &mut MapMemory,
    viewport: egui::Rect,
    (min_lat, max_lat, min_lon, max_lon): (f64, f64, f64, f64),
) {
    let center_lat = (min_lat + max_lat) / 2.0;
    let center_lon = (min_lon + max_lon) / 2.0;
    map_memory.center_at(walkers::lat_lon(center_lat, center_lon));

    let lat_range = (max_lat - min_lat).max(0.001);
    let lon_range = (max_lon - min_lon).max(0.001);
    let vw = viewport.width() as f64;
    let vh = viewport.height() as f64;

    // At zoom z the world is 256·2^z pixels wide (equatorial Mercator).
    // Fill 80 % of the viewport with the bounding box:
    //   lon_range · (256·2^z / 360) = vw · 0.8
    //   → z = log2(vw · 0.8 · 360 / (256 · lon_range))
    let z_lon = (vw * 0.8 * 360.0 / (256.0 * lon_range)).log2();
    let z_lat = (vh * 0.8 * 360.0 / (256.0 * lat_range)).log2();
    let zoom = z_lon.min(z_lat).clamp(1.0, 18.0);
    // zoom is already clamped to [1, 18], so set_zoom can only fail if the
    // walkers library's valid range narrows further - ignore silently.
    let _ignored = map_memory.set_zoom(zoom);
}

#[cfg(test)]
mod zoom_to_fit_antimeridian {
    use std::path::PathBuf;

    use chrono::{TimeZone, Utc};
    use gt_types::{
        FileMetadata, FileSource, GpsTime, Latitude, LoadedTrack, Longitude, TimePositionVelocity,
        TrackLod,
    };
    use gt_ui_types::{FileVisibility, TrackVisibility};
    use rustc_hash::FxHashMap;
    use uom::si::angle::degree;
    use uom::si::f64::Angle;

    use super::*;

    /// The viewport every case here frames into, in logical pixels.
    const VIEWPORT: egui::Rect =
        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(800.0, 600.0));

    fn fix(seconds: i64, lat: f64, lon: f64) -> NavPoint {
        let time = GpsTime::from_utc(
            Utc.timestamp_opt(seconds, 0)
                .single()
                .expect("valid timestamp"),
        );
        let tpv = TimePositionVelocity::builder()
            .time(time)
            .lat(Latitude::new(lat))
            .lon(Longitude::new(lon))
            .heading(Angle::new::<degree>(90.0))
            .build();
        NavPoint::new(tpv, None)
    }

    /// A one-track file over `points`. Only the points matter here:
    /// [`compute_visible_bounding_box`] reads them, not the track metadata.
    fn file_over(points: Vec<NavPoint>) -> LoadedFile {
        LoadedFile {
            metadata: FileMetadata {
                filename: "track.gtd".to_owned(),
                ..gt_test_utils::empty_file_metadata()
            },
            tracks: vec![LoadedTrack {
                metadata: gt_test_utils::empty_track_metadata(),
                points,
                lod: TrackLod::default(),
                sat_label_anchors: Vec::new(),
                custom_markers: vec![],
                generated_markers: vec![],
                event_markers: vec![],
                channels: vec![],
            }],
            event_marker_styles: FxHashMap::default(),
            orphaned_event_markers: vec![],
            source: FileSource::GtdPath(PathBuf::from("track.gtd")),
            load_warnings: vec![],
        }
    }

    fn all_visible() -> TrackDataVisibility {
        TrackDataVisibility {
            files: vec![FileVisibility {
                enabled: true,
                tracks: vec![TrackVisibility::all_visible()],
            }],
        }
    }

    /// An eastbound equatorial crossing running 179.0° E to 180.5° E:
    /// 1.5° of longitude, 166.79 km long.
    fn antimeridian_file() -> Vec<LoadedFile> {
        vec![file_over(vec![
            fix(0, 0.0, 179.0),
            fix(60, 0.0, 179.5),
            fix(120, 0.0, -179.9),
            fix(180, 0.0, -179.5),
        ])]
    }

    fn visible_bounds(files: &[LoadedFile]) -> (f64, f64, f64, f64) {
        compute_visible_bounding_box(
            files,
            &all_visible(),
            &GlobalFilter::default(),
            DisplayMask::default(),
        )
        .expect("the file has visible points")
    }

    /// Opening a Pacific recording frames the map through `adopt_new_files`
    /// and [`zoom_to_fit`], which must center on the track's own center
    /// meridian, 179.75° E.
    #[test]
    fn zoom_to_fit_across_the_antimeridian_centers_on_the_track() {
        let files = antimeridian_file();
        let mut map_memory = MapMemory::default();
        zoom_to_fit(&mut map_memory, VIEWPORT, visible_bounds(&files));
        let center = map_memory.detached().expect("centered");
        assert!(
            (center.x() - 179.75).abs() < 0.5,
            "expected the map centered near 179.75° E, got {}",
            center.x()
        );
    }

    /// The same framing must zoom in on the 1.5° the track covers:
    /// `log2(800 · 0.8 · 360 / (256 · 1.5))` is 9.23, while the long way
    /// around the planet gives 1.32.
    #[test]
    fn zoom_to_fit_across_the_antimeridian_frames_the_track_not_the_globe() {
        let files = antimeridian_file();
        let mut map_memory = MapMemory::default();
        zoom_to_fit(&mut map_memory, VIEWPORT, visible_bounds(&files));
        assert!(
            map_memory.zoom() > 8.0,
            "a 166.79 km track was framed at zoom {}",
            map_memory.zoom()
        );
    }

    /// A track clear of the antimeridian is framed on itself: the same
    /// formula, at a longitude range of 0.04°, zooms all the way in.
    #[test]
    fn zoom_to_fit_over_a_local_track_frames_it() {
        let files = vec![file_over(vec![fix(0, 55.67, 12.55), fix(60, 55.69, 12.59)])];
        let mut map_memory = MapMemory::default();
        zoom_to_fit(&mut map_memory, VIEWPORT, visible_bounds(&files));
        let center = map_memory.detached().expect("centered");
        assert!((center.x() - 12.57).abs() < 1e-9, "lon {}", center.x());
        assert!((center.y() - 55.68).abs() < 1e-9, "lat {}", center.y());
        assert!(map_memory.zoom() > 12.0, "zoom {}", map_memory.zoom());
    }

    /// A single-fix track gives a zero-sized box; [`zoom_to_fit`]'s floor on
    /// the span keeps the zoom finite and clamped to the map's maximum.
    #[test]
    fn zoom_to_fit_over_a_single_fix_clamps_to_the_maximum_zoom() {
        let files = vec![file_over(vec![fix(0, 55.67, 12.55)])];
        let mut map_memory = MapMemory::default();
        zoom_to_fit(&mut map_memory, VIEWPORT, visible_bounds(&files));
        assert!(
            (map_memory.zoom() - 18.0).abs() < 1e-9,
            "{}",
            map_memory.zoom()
        );
    }
}

//! Viewport-level geometry and collection: geographic bounds of the
//! visible map, zoom-to-fit, hit-test visibility, and the per-frame
//! collection of spatial points the render plugins draw.

use gt_filter::{GlobalFilter, point_passes_time_filter, track_passes_filter};
use gt_types::{DataCategory, FileIdx, LoadedFile, SpatialPoint, TrackIdx, TrackRef};
use gt_ui_types::{DisplayCategory, DisplayMask, QueryMatches, TrackDataVisibility};
use smallvec::SmallVec;
use std::collections::HashMap;
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
pub(crate) struct VisiblePoints {
    pub(crate) tpv_by_track: HashMap<TrackRef, Vec<usize>>,
    pub(crate) custom: Vec<SpatialPoint>,
    pub(crate) generated: Vec<SpatialPoint>,
    pub(crate) event: Vec<SpatialPoint>,
}

/// Collect the spatial points inside the current viewport from the global
/// R-tree, one list per category.
///
/// TPV points are gated by the frame's [`TrackPlan`]: fix icons of tracks
/// that are disabled, filtered out, TPV-layer-hidden, or classified
/// [`TrackIconFade::AllHidden`] (the quality line stands in) are never
/// drawn, so collecting their viewport points - potentially the entire
/// recording when zoomed out - would be pure allocation waste.
pub(crate) fn collect_visible_points(
    tree: &rstar::RTree<SpatialPoint>,
    plan: &TrackPlan,
    transform: &MercTransform,
    map_rect: egui::Rect,
) -> VisiblePoints {
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
    let mut visible = VisiblePoints {
        tpv_by_track: HashMap::new(),
        custom: Vec::new(),
        generated: Vec::new(),
        event: Vec::new(),
    };
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
    visible
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
}

impl TrackEntry {
    /// TPV viewport points are worth collecting only when icons can draw.
    fn tpv_collectable(self) -> bool {
        self.fade.is_some_and(|f| f != TrackIconFade::AllHidden)
    }

    /// No layer draws. The renderer can skip the track outright.
    pub(crate) fn draws_nothing(self) -> bool {
        !self.trackline && self.fade.is_none() && !self.sat_labels
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
                let trip_vis = file_vis.and_then(|fv| TrackIdx::new(ti).get(&fv.tracks));
                let enabled = file_enabled
                    && trip_vis.is_some_and(|tv| tv.enabled)
                    && track_passes_filter(&track.metadata, filter);
                let tpv_on = enabled && trip_vis.is_some_and(|tv| tv.tpv_visible);
                // The fade classification runs last so it is skipped for
                // tracks that are hidden or filtered out anyway.
                let fade = (tpv_on && display_mask.is_visible(DisplayCategory::TrackPoints))
                    .then(|| tpv_renderer::classify_icon_fade(track, scale, icon_size));
                entries.push(TrackEntry {
                    trackline: enabled
                        && trip_vis.is_some_and(|tv| tv.track_visible)
                        && display_mask.is_visible(DisplayCategory::Tracks),
                    fade,
                    sat_labels: tpv_on && display_mask.is_visible(DisplayCategory::SatelliteLabels),
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
            let Some(trip_vis) = file_vis.tracks.get(ti) else {
                continue;
            };
            if !trip_vis.enabled {
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

/// Compute the geographic bounding box of the given map viewport rect.
///
/// Uses the walkers `Projector` to unproject the four corners of `map_rect`
/// into geographic positions and returns their bounding envelope.
pub(crate) fn compute_viewport_bounds(map_memory: &MapMemory, map_rect: egui::Rect) -> GeoBounds {
    // `my_position` is only used as a fallback when the map is in GPS-follow mode;
    // since we always call `center_at()` explicitly, `detached()` provides the
    // actual center.  Fall back to (0, 0) if `detached()` is unset.
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

/// Returns `true` when a spatial point should participate in hover and click detection.
///
/// The renderers suppress invisible elements from being drawn; this function
/// applies the *same* rules so a hidden element cannot be hovered or clicked.
/// That means matching the renderers on every axis: file/track enablement, the
/// per-category layer toggle, the track-level filter, the per-point time
/// window (so points of a partially-overlapping track outside the window are not
/// hit-testable either), and the points a `keep`/`hide` query removed.
pub(crate) fn is_spatial_point_visible(
    sp: &SpatialPoint,
    files: &[LoadedFile],
    visibility: &TrackDataVisibility,
    filter: &GlobalFilter,
    display_mask: DisplayMask,
    query_matches: Option<&QueryMatches>,
) -> bool {
    // Tracklines and raw satellite reports have no hover target of their own.
    if matches!(
        sp.category,
        DataCategory::Track | DataCategory::SatelliteReport
    ) {
        return false;
    }
    // The gating the renderers apply (enablement, tree toggle, filter),
    // plus the display category: an element hidden either way is not
    // drawn, so it must not be hoverable or clickable either.
    let Some(track) =
        crate::scope::category_in_scope(files, visibility, filter, sp.track_ref(), sp.category)
    else {
        return false;
    };
    if !display_mask.is_visible(DisplayCategory::from(sp.category)) {
        return false;
    }
    let pi = sp.point_index.as_usize();
    // A `keep`/`hide` query removes TPV points from the drawn line and icons;
    // markers stay drawn (the hidden ranges index TPV points, not the marker
    // arrays), so only the TPV category consults the mask.
    if sp.category == DataCategory::Tpv
        && query_matches.is_some_and(|m| m.is_hidden(sp.track_ref(), pi))
    {
        return false;
    }
    let time = match sp.category {
        DataCategory::Tpv => track.points.get(pi).map(|p| p.tpv.time().utc()),
        DataCategory::CustomMarker => track.custom_markers.get(pi).map(|m| m.time),
        DataCategory::GeneratedMarker => track.generated_markers.get(pi).map(|m| m.time),
        DataCategory::EventMarker => track.event_markers.get(pi).map(|m| m.time),
        DataCategory::Track | DataCategory::SatelliteReport => None,
    };
    time.is_some_and(|t| point_passes_time_filter(t, filter))
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

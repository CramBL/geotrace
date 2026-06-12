//! Viewport-level geometry and collection: geographic bounds of the
//! visible map, zoom-to-fit, hit-test visibility, and the per-frame
//! collection of spatial points the render plugins draw.

use gt_filter::{GlobalFilter, track_passes_filter};
use gt_types::{DataCategory, FileIdx, LoadedFile, SpatialPoint, TrackIdx, TrackRef};
use gt_ui_types::TrackDataVisibility;
use smallvec::SmallVec;
use std::collections::HashMap;
use walkers::MapMemory;

use crate::tpv_renderer::{self, TrackIconFade};
use crate::transform::MercTransform;

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
/// indices), which is the shape `TpvRenderer` consumes.
pub(crate) struct VisiblePoints {
    pub(crate) tpv_by_track: HashMap<TrackRef, Vec<usize>>,
    pub(crate) custom: Vec<SpatialPoint>,
    pub(crate) generated: Vec<SpatialPoint>,
    pub(crate) event: Vec<SpatialPoint>,
}

/// Collect the spatial points inside the current viewport from the global
/// R-tree, one list per category.
///
/// TPV points are additionally gated per track: fix icons of tracks that
/// are disabled, filtered out, TPV-layer-hidden, or classified
/// [`TrackIconFade::AllHidden`] (the quality line stands in) are never
/// drawn, so collecting their viewport points - potentially the entire
/// recording when zoomed out - would be pure allocation waste. The fade
/// classification matches `TpvRenderer::run` exactly: same zoom-derived
/// icon size, same per-frame map scale.
pub(crate) fn collect_visible_points(
    tree: &rstar::RTree<SpatialPoint>,
    files: &[LoadedFile],
    visibility: &TrackDataVisibility,
    filter: &GlobalFilter,
    transform: &MercTransform,
    map_rect: egui::Rect,
    zoom: f64,
) -> VisiblePoints {
    let collectable = TpvCollectable::compute(files, visibility, filter, transform, zoom);
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
                if collectable.is_collectable(sp.track_ref()) {
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

/// Per-track "may this track's TPV icons draw this frame" flags, flattened
/// into one inline buffer (`flags[offsets[fi] + ti]`, with `offsets`
/// carrying one trailing end entry), so computing them allocates nothing
/// for typical workspace sizes.
struct TpvCollectable {
    flags: SmallVec<[bool; 128]>,
    offsets: SmallVec<[usize; 9]>,
}

impl TpvCollectable {
    fn compute(
        files: &[LoadedFile],
        visibility: &TrackDataVisibility,
        filter: &GlobalFilter,
        transform: &MercTransform,
        zoom: f64,
    ) -> Self {
        let icon_size = tpv_renderer::base_arrow_size(zoom);
        let mut flags: SmallVec<[bool; 128]> = SmallVec::new();
        let mut offsets: SmallVec<[usize; 9]> = SmallVec::new();
        for (fi, file) in files.iter().enumerate() {
            offsets.push(flags.len());
            let file_vis = FileIdx::new(fi).get(&visibility.files);
            for (ti, track) in file.tracks.iter().enumerate() {
                let trip_vis = file_vis.and_then(|fv| TrackIdx::new(ti).get(&fv.tracks));
                // The fade classification runs last so it is skipped for
                // tracks that are hidden or filtered out anyway.
                let drawable = file_vis.is_some_and(|fv| fv.enabled)
                    && trip_vis.is_some_and(|tv| tv.enabled && tv.tpv_visible)
                    && track_passes_filter(&track.metadata, filter)
                    && tpv_renderer::classify_icon_fade(track, transform, icon_size)
                        != TrackIconFade::AllHidden;
                flags.push(drawable);
            }
        }
        offsets.push(flags.len());
        Self { flags, offsets }
    }

    /// Whether the track's icons may draw. Unknown tracks default to
    /// collectable, so a stale index can only cost wasted collection, never
    /// hidden data.
    fn is_collectable(&self, track: TrackRef) -> bool {
        let fi = track.fi.as_usize();
        let (Some(&start), Some(&end)) = (self.offsets.get(fi), self.offsets.get(fi + 1)) else {
            return true;
        };
        let idx = start + track.index.as_usize();
        if idx >= end {
            return true;
        }
        self.flags.get(idx).copied().unwrap_or(true)
    }
}

/// Bounding box over only the currently **visible** tracks (those with both their
/// file and track enabled). Returns `None` if no visible data exists.
pub(crate) fn compute_visible_bounding_box(
    files: &[LoadedFile],
    visibility: &TrackDataVisibility,
) -> Option<(f64, f64, f64, f64)> {
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
            for point in &track.points {
                let lat = point.tpv.lat().as_degrees();
                let lon = point.tpv.lon().as_degrees();
                min_lat = min_lat.min(lat);
                max_lat = max_lat.max(lat);
                min_lon = min_lon.min(lon);
                max_lon = max_lon.max(lon);
                any = true;
            }
            for marker in &track.custom_markers {
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

    if any {
        Some((min_lat, max_lat, min_lon, max_lon))
    } else {
        None
    }
}

/// Bounding box (min_lat, max_lat, min_lon, max_lon) over all GPS points and
/// custom markers in every loaded file. Returns `None` if there is no data.
pub(crate) fn compute_bounding_box(files: &[LoadedFile]) -> Option<(f64, f64, f64, f64)> {
    let mut min_lat = f64::MAX;
    let mut max_lat = f64::MIN;
    let mut min_lon = f64::MAX;
    let mut max_lon = f64::MIN;
    let mut any = false;

    for file in files {
        for track in &file.tracks {
            for point in &track.points {
                let lat = point.tpv.lat().as_degrees();
                let lon = point.tpv.lon().as_degrees();
                min_lat = min_lat.min(lat);
                max_lat = max_lat.max(lat);
                min_lon = min_lon.min(lon);
                max_lon = max_lon.max(lon);
                any = true;
            }
            for marker in &track.custom_markers {
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
/// The renderers already suppress invisible elements from being drawn; this function
/// ensures the hit-test layer applies the same rules so that hidden tracks cannot be
/// accidentally hovered or clicked.
pub(crate) fn is_spatial_point_visible(
    sp: &SpatialPoint,
    visibility: &TrackDataVisibility,
) -> bool {
    let Some(file_vis) = sp.file_index.get(&visibility.files) else {
        return false;
    };
    if !file_vis.enabled {
        return false;
    }
    let Some(trip_vis) = sp.track_index.get(&file_vis.tracks) else {
        return false;
    };
    if !trip_vis.enabled {
        return false;
    }
    match sp.category {
        DataCategory::Tpv => trip_vis.tpv_visible,
        DataCategory::CustomMarker => trip_vis.custom_markers_visible,
        DataCategory::GeneratedMarker => trip_vis.generated_markers_visible,
        DataCategory::EventMarker => trip_vis.event_markers_visible,
        DataCategory::Track | DataCategory::SatelliteReport => false,
    }
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

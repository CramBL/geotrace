//! Viewport-level geometry and collection: geographic bounds of the
//! visible map, zoom-to-fit, hit-test visibility, and the per-frame
//! collection of spatial points the render plugins draw.

use std::ops::Range;

use gt_filter::{GlobalFilter, point_passes_time_filter, track_passes_filter};
use gt_types::{
    DataCategory, FileIdx, GeoBounds, Latitude, LoadedFile, LoadedTrack, Longitude, PlacedPoint,
    PlacedPoints, PoleWinding, SpatialPoint, TrackIdx, TrackRef, mercator,
};
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
pub struct ViewportBounds {
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
                    && track_passes_filter(track, filter);
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
///
/// A track whose drawn fixes circle a pole contributes the cap around that
/// pole, the way its metadata's box does: each track is bounded on its own
/// before the boxes are united.
pub(crate) fn compute_visible_bounding_box(
    files: &[LoadedFile],
    visibility: &TrackDataVisibility,
    filter: &GlobalFilter,
    display_mask: DisplayMask,
) -> Option<GeoBounds> {
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

    files
        .iter()
        .zip(&visibility.files)
        .filter(|(_, file_vis)| file_vis.enabled)
        .flat_map(|(file, file_vis)| file.tracks.iter().zip(&file_vis.tracks))
        .filter(|(track, track_vis)| track_vis.enabled && track_passes_filter(track, filter))
        .filter_map(|(track, _)| {
            let fixes = || {
                points_displayed
                    .then(|| drawn_fix_positions(track.placed_points().unwrap_or_default(), filter))
                    .into_iter()
                    .flatten()
            };
            let markers = custom_markers_displayed
                .then(|| drawn_custom_marker_positions(track, filter))
                .into_iter()
                .flatten();
            let bounds = GeoBounds::from_positions(fixes().chain(markers))?;
            Some(bounds.extended_to_the_encircled_pole(PoleWinding::of_track(fixes())))
        })
        .reduce(GeoBounds::union)
}

/// Where the map draws the fixes of `placed` that pass the time filter. Empty
/// for the points of a track with no geometry, which is drawn nowhere.
fn drawn_fix_positions<'a>(
    placed: PlacedPoints<'a>,
    filter: &'a GlobalFilter,
) -> impl Iterator<Item = (Latitude, Longitude)> + 'a {
    placed
        .iter()
        .filter(|point| point_passes_time_filter(point.fix.tpv.time().utc(), filter))
        .map(PlacedPoint::resolved_position)
}

fn drawn_custom_marker_positions<'a>(
    track: &'a LoadedTrack,
    filter: &'a GlobalFilter,
) -> impl Iterator<Item = (Latitude, Longitude)> + 'a {
    track
        .custom_markers
        .iter()
        .filter(|marker| point_passes_time_filter(marker.time, filter))
        .map(|marker| (marker.lat, marker.lon))
}

/// Bounding box over the points every `draw` layer of `matches` covers that
/// the filter keeps, for framing the map on what a query run drew. `None` when
/// no draw layer covers a drawn point of a loaded track.
pub(crate) fn matched_bounding_box(
    files: &[LoadedFile],
    matches: &QueryMatches,
    filter: &GlobalFilter,
) -> Option<GeoBounds> {
    let mut matched_ranges: Vec<(TrackRef, &[Range<usize>])> = matches
        .draws
        .iter()
        .flat_map(|layer| &layer.ranges)
        .map(|(track_ref, ranges)| (*track_ref, ranges.as_slice()))
        .collect();
    // Sorting the ranges by TrackRef fixes the order the longitude extent
    // grows in: the ranges of a draw layer are keyed by a hash map, so an
    // unsorted fold would frame a different box from run to run.
    matched_ranges.sort_by_key(|(track_ref, _)| *track_ref);

    GeoBounds::from_positions(
        matched_ranges
            .into_iter()
            .filter_map(|(track_ref, ranges)| Some((track_ref.resolve(files)?, ranges)))
            .filter(|(track, _)| track_passes_filter(track, filter))
            .filter_map(|(track, ranges)| Some((track.placed_points()?, ranges)))
            .flat_map(|(placed, ranges)| {
                ranges.iter().flat_map(move |range| {
                    drawn_fix_positions(placed.range(range.clone()).unwrap_or_default(), filter)
                })
            }),
    )
}

/// Bounding box over the points of one match that the filter keeps, for
/// framing the map on the match a results row points at. `None` when its track
/// is no longer loaded, the range reaches past it, the filter rejects the
/// track, or the time window hides every point of the range.
pub(crate) fn match_bounding_box(
    files: &[LoadedFile],
    track_ref: TrackRef,
    points: &Range<usize>,
    filter: &GlobalFilter,
) -> Option<GeoBounds> {
    let track = track_ref.resolve(files)?;
    if !track_passes_filter(track, filter) {
        return None;
    }
    let matched = track.placed_points()?.range(points.clone())?;
    GeoBounds::from_positions(drawn_fix_positions(matched, filter))
}

/// Compute the geographic bounding box of the given map viewport rect.
///
/// Uses the walkers `Projector` to unproject the four corners of `map_rect`
/// into geographic positions and returns their bounding envelope.
pub(crate) fn compute_viewport_bounds(
    map_memory: &MapMemory,
    map_rect: egui::Rect,
) -> ViewportBounds {
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

    ViewportBounds {
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

/// Longitude the normalized Mercator world spans: dividing a longitude span
/// by it gives that span's x extent.
const WORLD_WIDTH_DEGREES: f64 = 360.0;

/// Floor on a bounding box's extent in normalized Mercator units, keeping the
/// fit of a single position finite. A thousandth of a degree of longitude,
/// which every viewport frames past the maximum zoom.
const MIN_FIT_EXTENT_MERC: f64 = 0.001 / WORLD_WIDTH_DEGREES;

/// The share of the viewport a fit fills with the bounding box.
const FIT_FILL: f64 = 0.8;

/// What a fit could put on the map.
///
/// Web Mercator ends at [`mercator::MAX_LATITUDE_DEGREES`]. No zoom draws a
/// fix past that parallel, and the fit centres on the limit instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FitOutcome {
    Framed,
    FixesPastTheNorthernLimit,
    FixesPastTheSouthernLimit,
    FixesPastBothLimits,
}

impl FitOutcome {
    /// What to tell the user about the fixes the projection leaves out.
    pub(crate) fn notice(self) -> Option<String> {
        let limit = mercator::MAX_LATITUDE_DEGREES;
        match self {
            Self::Framed => None,
            Self::FixesPastTheNorthernLimit => Some(format!(
                "The map projection ends at {limit:.0}° N: fixes above it are not drawn"
            )),
            Self::FixesPastTheSouthernLimit => Some(format!(
                "The map projection ends at {limit:.0}° S: fixes below it are not drawn"
            )),
            Self::FixesPastBothLimits => Some(format!(
                "The map projection ends at {limit:.0}° N and {limit:.0}° S: fixes past them are not drawn"
            )),
        }
    }
}

/// Center the map and set the zoom so `bounds` fills ~80 % of the viewport.
/// Respects walkers' valid zoom range [1, 18].
pub(crate) fn zoom_to_fit(
    map_memory: &mut MapMemory,
    viewport: egui::Rect,
    bounds: GeoBounds,
) -> FitOutcome {
    let (center_lat, center_lon) = bounds.center();
    let center_lat = center_lat.clamped_to_the_mercator_limit();
    map_memory.center_at(walkers::lat_lon(
        center_lat.as_degrees(),
        center_lon.as_degrees(),
    ));

    // A box over every meridian holds a polar cap, and what the fit has to
    // show of it is its diameter across the pole. Mercator draws that arc as
    // wide as the longitudes it covers on the parallel the map centres on.
    let lon_span_degrees = match bounds.lon.is_full_circle() {
        true => {
            bounds.lat.arc_across_the_pole_degrees() / center_lat.as_degrees().to_radians().cos()
        }
        false => bounds.lon.span_degrees(),
    };
    let x_extent = (lon_span_degrees / WORLD_WIDTH_DEGREES).max(MIN_FIT_EXTENT_MERC);

    // Mercator stretches latitude away from the equator: a degree at 80° N
    // stands 5.8 times as tall as one at the equator.
    let north_edge = mercator::normalize(
        bounds.lat.north().clamped_to_the_mercator_limit(),
        center_lon,
    );
    let south_edge = mercator::normalize(
        bounds.lat.south().clamped_to_the_mercator_limit(),
        center_lon,
    );
    let y_extent = (south_edge.y - north_edge.y).max(MIN_FIT_EXTENT_MERC);

    // The box fills FIT_FILL of the viewport once the world spans
    // viewport · FIT_FILL / extent pixels.
    let z_lon = MapScale::zoom_for_world_px(viewport.width() as f64 * FIT_FILL / x_extent);
    let z_lat = MapScale::zoom_for_world_px(viewport.height() as f64 * FIT_FILL / y_extent);
    let zoom = z_lon.min(z_lat).clamp(1.0, 18.0);
    // zoom is already clamped to [1, 18], so set_zoom can only fail if the
    // walkers library's valid range narrows further - ignore silently.
    let _ignored = map_memory.set_zoom(zoom);

    match (
        bounds.lat.north().as_degrees() > mercator::MAX_LATITUDE_DEGREES,
        bounds.lat.south().as_degrees() < -mercator::MAX_LATITUDE_DEGREES,
    ) {
        (false, false) => FitOutcome::Framed,
        (true, false) => FitOutcome::FixesPastTheNorthernLimit,
        (false, true) => FitOutcome::FixesPastTheSouthernLimit,
        (true, true) => FitOutcome::FixesPastBothLimits,
    }
}

#[cfg(test)]
mod zoom_to_fit {
    use chrono::{Duration, TimeZone, Utc};
    use gt_types::NavPoint;
    use rstest::rstest;

    use super::*;
    use crate::tests::{file_with_tracks, nav_at, track_over, vis_all_visible};

    /// The viewport every case here frames into, in logical pixels.
    const VIEWPORT: egui::Rect =
        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(800.0, 600.0));

    /// A receiver carried around the north pole a quarter turn at a time, on
    /// a ring 22.24 km across.
    const AROUND_THE_NORTH_POLE: &[(f64, f64)] =
        &[(89.9, 0.0), (89.9, 90.0), (89.9, 180.0), (89.9, -90.0)];

    /// The same lap around the south pole, walked the other way.
    const AROUND_THE_SOUTH_POLE: &[(f64, f64)] =
        &[(-89.9, 0.0), (-89.9, -90.0), (-89.9, 180.0), (-89.9, 90.0)];

    /// Fixes one second apart at `positions`, given as (latitude, longitude)
    /// in degrees.
    fn fixes_at(positions: &[(f64, f64)]) -> Vec<NavPoint> {
        let start = Utc.timestamp_opt(0, 0).single().expect("valid timestamp");
        positions
            .iter()
            .zip(0_i64..)
            .map(|(&(lat, lon), second)| nav_at(start + Duration::seconds(second), lat, lon))
            .collect()
    }

    fn file_over(positions: &[(f64, f64)]) -> Vec<LoadedFile> {
        vec![file_with_tracks(vec![track_over(fixes_at(positions))])]
    }

    /// An eastbound equatorial crossing running 179.0° E to 180.5° E:
    /// 1.5° of longitude, 166.79 km long.
    fn antimeridian_file() -> Vec<LoadedFile> {
        file_over(&[(0.0, 179.0), (0.0, 179.5), (0.0, -179.9), (0.0, -179.5)])
    }

    fn visible_bounds(files: &[LoadedFile]) -> GeoBounds {
        compute_visible_bounding_box(
            files,
            &vis_all_visible(),
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
        let files = file_over(&[(55.67, 12.55), (55.69, 12.59)]);
        let mut map_memory = MapMemory::default();
        zoom_to_fit(&mut map_memory, VIEWPORT, visible_bounds(&files));
        let center = map_memory.detached().expect("centered");
        assert!((center.x() - 12.57).abs() < 1e-9, "lon {}", center.x());
        assert!((center.y() - 55.68).abs() < 1e-9, "lat {}", center.y());
        assert!(map_memory.zoom() > 12.0, "zoom {}", map_memory.zoom());
    }

    /// A single-fix track gives a zero-sized box: [`zoom_to_fit`]'s floor on
    /// the span keeps the zoom finite and clamped to the map's maximum.
    #[test]
    fn zoom_to_fit_over_a_single_fix_clamps_to_the_maximum_zoom() {
        let files = file_over(&[(55.67, 12.55)]);
        let mut map_memory = MapMemory::default();
        zoom_to_fit(&mut map_memory, VIEWPORT, visible_bounds(&files));
        assert!(
            (map_memory.zoom() - 18.0).abs() < 1e-9,
            "{}",
            map_memory.zoom()
        );
    }

    /// A track that circles a pole is bounded by the cap over every meridian,
    /// so the fit must size from the cap's 22.24 km diameter. Read as 360°
    /// of longitude, the same box frames the whole globe at zoom 1.32.
    #[test]
    fn zoom_to_fit_around_the_pole_frames_the_track_not_the_globe() {
        let files = file_over(AROUND_THE_NORTH_POLE);
        let mut map_memory = MapMemory::default();
        zoom_to_fit(&mut map_memory, VIEWPORT, visible_bounds(&files));
        assert!(
            map_memory.zoom() > 8.0,
            "a 22.24 km track around the pole was framed at zoom {}",
            map_memory.zoom()
        );
    }

    /// The cap around a pole centres at 89.95°, which Web Mercator does not
    /// reach: the map has to stop at the parallel the projection ends on.
    #[test]
    fn zoom_to_fit_around_the_pole_centers_at_the_projection_limit() {
        let files = file_over(AROUND_THE_NORTH_POLE);
        let mut map_memory = MapMemory::default();
        zoom_to_fit(&mut map_memory, VIEWPORT, visible_bounds(&files));
        let center = map_memory.detached().expect("centered");
        assert!(
            (center.y() - mercator::MAX_LATITUDE_DEGREES).abs() < 1e-9,
            "centered on {}° N",
            center.y()
        );
    }

    /// A fit spans the ground the track covers, wherever on the globe it
    /// lies. The map's scale at the centre and zoom the fit chose turns the
    /// share of the viewport it fills back into metres.
    #[rstest]
    #[case::at_the_equator(&[(0.0, 0.0), (0.0, 0.3473)])]
    #[case::at_eighty_north(&[(80.0, 0.0), (80.0, 2.0)])]
    #[case::around_the_south_pole(AROUND_THE_SOUTH_POLE)]
    fn zoom_to_fit_frames_the_width_of_the_track(#[case] positions: &[(f64, f64)]) {
        let files = file_over(positions);
        let mut map_memory = MapMemory::default();
        zoom_to_fit(&mut map_memory, VIEWPORT, visible_bounds(&files));

        let framed_m =
            VIEWPORT.width() as f64 * FIT_FILL / pixels_per_meter_at_the_map_center(&map_memory);
        let track_positions: Vec<(Latitude, Longitude)> = positions
            .iter()
            .map(|&(lat, lon)| (Latitude::new(lat), Longitude::new(lon)))
            .collect();
        let track_m = gt_geo_math::point_set_diameter_m(&track_positions);
        assert!(
            (framed_m / track_m - 1.0).abs() < 0.02,
            "framed {framed_m:.0} m of a {track_m:.0} m wide track at zoom {}",
            map_memory.zoom()
        );
    }

    /// A track 1° of latitude tall and 0.1° of longitude wide, centred on
    /// `center_lat_degrees`.
    fn tall_narrow_track(center_lat_degrees: f64) -> Vec<(f64, f64)> {
        vec![
            (center_lat_degrees - 0.5, 0.0),
            (center_lat_degrees + 0.5, 0.1),
        ]
    }

    /// A fit spans the ground the track covers north to south, wherever on
    /// the globe it lies. Web Mercator draws a degree of latitude 1/cos(lat)
    /// as tall as one at the equator, so the further from the equator the
    /// track lies, the further out its fit has to sit.
    #[rstest]
    #[case::straddling_the_equator(0.0)]
    #[case::at_fifty_five_north(55.0)]
    #[case::at_eighty_north(80.0)]
    #[case::at_eighty_four_north(84.0)]
    fn zoom_to_fit_frames_the_height_of_the_track(#[case] center_lat_degrees: f64) {
        let files = file_over(&tall_narrow_track(center_lat_degrees));
        let bounds = visible_bounds(&files);
        let mut map_memory = MapMemory::default();
        zoom_to_fit(&mut map_memory, VIEWPORT, bounds);

        let framed_m =
            VIEWPORT.height() as f64 * FIT_FILL / pixels_per_meter_at_the_map_center(&map_memory);
        let track_m = gt_geo_math::haversine_m(
            bounds.lat.south(),
            bounds.lon.center(),
            bounds.lat.north(),
            bounds.lon.center(),
        );
        assert!(
            (framed_m / track_m - 1.0).abs() < 0.02,
            "framed {framed_m:.0} m of a {track_m:.0} m tall track at zoom {}, drawing it {:.1}× the height the fit frames",
            map_memory.zoom(),
            track_m / framed_m
        );
    }

    /// Mercator's vertical stretch is 1 at the equator, where a degree of
    /// latitude covers 1/360 of the world: a box 1° tall straddling it frames
    /// at `log2(600 · 0.8 · 360 / 256)`.
    #[test]
    fn zoom_to_fit_over_the_equator_sizes_from_the_degree_span() {
        let files = file_over(&tall_narrow_track(0.0));
        let mut map_memory = MapMemory::default();
        zoom_to_fit(&mut map_memory, VIEWPORT, visible_bounds(&files));
        assert!(
            (map_memory.zoom() - 9.399).abs() < 1e-3,
            "zoom {}",
            map_memory.zoom()
        );
    }

    /// The map's scale at the centre and zoom the fit left it at.
    fn pixels_per_meter_at_the_map_center(map_memory: &MapMemory) -> f64 {
        let center = map_memory.detached().expect("centered");
        MapScale::from_zoom(map_memory.zoom()).pixels_per_meter(Latitude::new(center.y()))
    }

    /// What the fit could not show is the caller's to pass on to the user.
    #[rstest]
    #[case::inside_the_projection(&[(55.67, 12.55), (55.69, 12.59)], FitOutcome::Framed)]
    #[case::past_the_northern_limit(
        &[(86.0, 10.0), (87.0, 11.0)],
        FitOutcome::FixesPastTheNorthernLimit
    )]
    #[case::past_the_southern_limit(
        &[(-87.0, 10.0), (-86.0, 11.0)],
        FitOutcome::FixesPastTheSouthernLimit
    )]
    #[case::past_both_limits(&[(-86.0, 10.0), (86.0, 11.0)], FitOutcome::FixesPastBothLimits)]
    fn zoom_to_fit_reports_the_fixes_the_projection_leaves_out(
        #[case] positions: &[(f64, f64)],
        #[case] expected: FitOutcome,
    ) {
        let files = file_over(positions);
        let mut map_memory = MapMemory::default();

        assert_eq!(
            zoom_to_fit(&mut map_memory, VIEWPORT, visible_bounds(&files)),
            expected
        );
    }
}

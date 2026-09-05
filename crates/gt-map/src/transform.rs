//! Per-frame projection from normalised Mercator coordinates to screen
//! pixels, and the LOD-aware point iteration built on top of it.

use gt_types::coordinates::{Latitude, Longitude};
use gt_types::{
    LOD_CHUNK_POINTS, LoadedTrack, MercBounds, MercPoint, PlacedPoint, PlacedPoints, mercator,
};
use walkers::MapMemory;

use crate::polyline::MAX_LOD_ERROR_PX;

/// Wrap a longitude in degrees into `Longitude`'s valid `[-180, 180]` range.
///
/// At low zoom the viewport can span more than 360° of longitude, so the
/// pixel column at the viewport centre may sit past the antimeridian wrap and
/// `Projector::unproject` returns e.g. 185°.
/// Longitude is periodic with period 360°, so the wrapped value identifies the
/// same meridian and is the correct input for [`Longitude::new`].
fn wrap_longitude_degrees(deg: f64) -> f64 {
    ((deg + 180.0).rem_euclid(360.0)) - 180.0
}

/// Per-frame transform context for projecting pre-computed normalised Mercator
/// coordinates to screen pixel positions with full f64 precision.
///
/// ## Why not `projector.project()`?
///
/// Walkers' `Projector::project()` computes the screen position of an arbitrary
/// geographic point by subtracting two large pixel values (the point's Mercator
/// pixel position and the map centre's) in f64, then calling `.to_vec2()` which
/// truncates the result to f32. At zoom ≥ 17 and for data far from the origin
/// (e.g. Denmark: lat 55° N, lon 12° E), the y-component of this difference is
/// ≈ 12 M px, where f32 ULP = 1 px. The anchor obtained this way has ≈ ±0.5 px
/// constant error per frame, which appears as snapping during smooth zoom
/// animations. Viewport culling arithmetic done in f32 before casting to f64
/// has the same issue.
///
/// ## Solution
///
/// This transform uses `projector.unproject(clip_center)` to obtain the map
/// centre's geographic coordinates. Walkers' `unproject` performs all arithmetic
/// in f64 and returns a high-precision `Position`. We then compute the normalised
/// Mercator coordinates of the map centre in f64, and express every point's
/// screen position as:
///
/// ```text
/// screen = clip_center + (merc_point − merc_center) × total_px
/// ```
///
/// `clip_center` is a small, exact f32 value (the pixel centre of the map
/// widget, typically around 800–1000 px). `merc_point − merc_center` is a
/// small number (≤ ~0.05 for any visible point), so no large-magnitude
/// arithmetic occurs anywhere. The final cast to f32 is applied only to the
/// already-small screen coordinate, where f32 ULP < 0.001 px.
pub(crate) struct MercTransform {
    clip_center_x: f64,
    clip_center_y: f64,
    merc_center: MercPoint,
    scale: MapScale,
}

/// The map's scale at a given zoom level: how many pixels the world spans.
///
/// Unlike [`MercTransform`] this carries no viewport anchoring, so it can be
/// derived before egui lays the map widget out - which makes it the right
/// input for per-frame decisions (icon-fade classification, LOD selection)
/// that must agree between the pre-layout planning pass and the renderers.
#[derive(Debug, Clone, Copy)]
pub(crate) struct MapScale {
    total_px: f64,
}

/// The pixel width of the world at zoom 0: a single tile.
const WORLD_PX_AT_ZOOM_0: f64 = 256.0;

impl MapScale {
    pub(crate) fn from_zoom(zoom: f64) -> Self {
        Self {
            total_px: 2_f64.powf(zoom) * WORLD_PX_AT_ZOOM_0,
        }
    }

    /// The zoom at which the world spans `total_px` pixels: the inverse of
    /// [`MapScale::from_zoom`]. Unclamped, so it can land outside walkers'
    /// [1, 18] range.
    pub(crate) fn zoom_for_world_px(total_px: f64) -> f64 {
        (total_px / WORLD_PX_AT_ZOOM_0).log2()
    }

    /// Pixels per metre at `lat`.
    ///
    /// Uses the Web Mercator scale factor: the equatorial circumference
    /// (≈ 40 030 km) shrinks by cos(lat) at a given latitude.
    #[inline]
    pub(crate) fn pixels_per_meter(self, lat: Latitude) -> f64 {
        // 1 Mercator tile column = Earth circumference / 2^z metres at the
        // equator, scaled by cos(lat) at higher latitudes.
        const EARTH_CIRCUMFERENCE_M: f64 = 40_030_173.0;
        self.total_px / (EARTH_CIRCUMFERENCE_M * lat.as_degrees().to_radians().cos())
    }

    /// Pixels per Mercator unit (the whole world spans one Mercator unit).
    /// This is the exact scale factor of [`MercTransform::to_screen`], so
    /// tolerances expressed in Mercator units convert losslessly to pixels.
    #[inline]
    pub(crate) fn px_per_merc(self) -> f64 {
        self.total_px
    }
}

impl MercTransform {
    /// Build the transform for the current frame.
    ///
    /// `clip_center` must be `ui.max_rect().center()` inside a plugin's
    /// `run()` method - walkers sets the child UI rect to the map widget rect,
    /// which is also `projector`'s clip rect, so `clip_center` equals
    /// `projector.clip_rect.center()`.
    pub(crate) fn new(
        projector: &walkers::Projector,
        map_memory: &MapMemory,
        clip_center: egui::Pos2,
    ) -> Self {
        let scale = MapScale::from_zoom(map_memory.zoom());
        // unproject(clip_center) returns the geographic position at the
        // viewport centre using f64 arithmetic throughout.
        // In walkers: Position.x() = longitude, Position.y() = latitude.
        let center_ll = projector.unproject(clip_center.to_vec2());
        // Latitude from `unproject` is always within ±90° (it comes from
        // `atan`), but longitude can land outside ±180° - see
        // `wrap_longitude_degrees`.
        //
        // The centre keeps its latitude unclamped past the projection's
        // limit: a drag over the top of the world puts it there, and this
        // anchor has to hold the same position walkers projects the tiles
        // from.
        let merc_center = mercator::normalize_past_the_projection_limit(
            Latitude::new(center_ll.y()),
            Longitude::new(wrap_longitude_degrees(center_ll.x())),
        );
        Self {
            clip_center_x: clip_center.x as f64,
            clip_center_y: clip_center.y as f64,
            merc_center,
            scale,
        }
    }

    /// A fixed transform for unit tests: viewport centred on the geographic
    /// origin, with the world `total_px` pixels wide.
    #[cfg(test)]
    pub(crate) fn for_test(total_px: f64) -> Self {
        Self::for_test_centered(total_px, Latitude::new(0.0))
    }

    /// Like [`MercTransform::for_test`], with the viewport centred on `lat`
    /// (for scale math that depends on the centre).
    #[cfg(test)]
    pub(crate) fn for_test_centered(total_px: f64, lat: Latitude) -> Self {
        Self {
            clip_center_x: 0.0,
            clip_center_y: 0.0,
            merc_center: mercator::normalize_past_the_projection_limit(lat, Longitude::new(0.0)),
            scale: MapScale { total_px },
        }
    }

    /// Like [`MercTransform::for_test_centered`], centred on a position and
    /// framed so that position lands at `clip_center` on screen.
    #[cfg(test)]
    pub(crate) fn for_test_view(
        total_px: f64,
        lat: Latitude,
        lon: Longitude,
        clip_center: egui::Pos2,
    ) -> Self {
        Self {
            clip_center_x: f64::from(clip_center.x),
            clip_center_y: f64::from(clip_center.y),
            merc_center: mercator::normalize_past_the_projection_limit(lat, lon),
            scale: MapScale { total_px },
        }
    }

    /// Project a pre-computed normalised Mercator coordinate to a screen position.
    #[inline]
    pub(crate) fn to_screen(&self, merc: MercPoint) -> egui::Pos2 {
        let total_px = self.scale.px_per_merc();
        egui::pos2(
            (self.clip_center_x + (merc.x - self.merc_center.x) * total_px) as f32,
            (self.clip_center_y + (merc.y - self.merc_center.y) * total_px) as f32,
        )
    }

    /// Convert a screen-space x-coordinate to a normalised Mercator x value.
    #[inline]
    pub(crate) fn merc_x_from_screen(&self, screen_x: f32) -> f64 {
        (screen_x as f64 - self.clip_center_x) / self.scale.px_per_merc() + self.merc_center.x
    }

    /// Convert a screen-space y-coordinate to a normalised Mercator y value.
    #[inline]
    pub(crate) fn merc_y_from_screen(&self, screen_y: f32) -> f64 {
        (screen_y as f64 - self.clip_center_y) / self.scale.px_per_merc() + self.merc_center.y
    }

    /// The screen rectangle's bounds in normalised Mercator space.
    pub(crate) fn viewport_merc_bounds(&self, rect: egui::Rect) -> gt_types::MercBounds {
        gt_types::MercBounds {
            x_min: self.merc_x_from_screen(rect.min.x),
            x_max: self.merc_x_from_screen(rect.max.x),
            y_min: self.merc_y_from_screen(rect.min.y),
            y_max: self.merc_y_from_screen(rect.max.y),
        }
    }

    /// The viewport-independent scale component of this transform. Lets
    /// tests derive a [`MapScale`] from a [`MercTransform::for_test`].
    #[cfg(test)]
    pub(crate) fn scale(&self) -> MapScale {
        self.scale
    }

    /// Pixels per metre at `lat`. See [`MapScale`].
    #[inline]
    pub(crate) fn pixels_per_meter(&self, lat: Latitude) -> f64 {
        self.scale.pixels_per_meter(lat)
    }

    /// Pixels per metre at the viewport centre's latitude - the scale gate
    /// for meter-sized annotations (the snap error whiskers). Inverts the
    /// Mercator y of the centre. One trigonometric round trip per frame.
    pub(crate) fn pixels_per_meter_at_center(&self) -> f64 {
        let lat_rad = (std::f64::consts::PI * (1.0 - 2.0 * self.merc_center.y))
            .sinh()
            .atan();
        self.pixels_per_meter(Latitude::new(lat_rad.to_degrees()))
    }

    /// Pixels per Mercator unit. See [`MapScale`].
    #[inline]
    pub(crate) fn px_per_merc(&self) -> f64 {
        self.scale.px_per_merc()
    }
}

/// How far the cull bounds reach past the cull rect the caller compares
/// screen positions against. The slack keeps a point outside those bounds
/// outside the rect once `to_screen` has rounded its f64 result to f32, a
/// rounding that moves a point by a fraction of a pixel.
const CULL_BOUNDS_SLACK_PX: f32 = 1.0;

/// Iterate `(index, point)` over the track's LOD level appropriate for the
/// current map scale, or over the full point list when no stored level is
/// fine enough (zoomed in, or no LOD built). Bounds polyline-pass iteration
/// by on-screen detail, and skips the stretches of the track that lie outside
/// `cull_rect` - the rect the caller then culls the segments against.
///
/// `placed` are `track`'s own points, which the caller has already gated on
/// the track having a geometry.
///
/// [`crate::polyline::segment_outside`] drops every segment between the
/// points of a chunk of [`LOD_CHUNK_POINTS`] points whose bounds miss
/// `cull_rect`, since such a chunk lies beyond one of the rect's edges. The
/// walk yields the chunk's first and last point, which keep the segments
/// entering and leaving it, and skips the rest. The walk keeps every point of
/// a chunk whose bounds cross the antimeridian, which covers two pieces of
/// the world that no single edge separates.
pub(crate) fn lod_points<'a>(
    track: &'a LoadedTrack,
    placed: PlacedPoints<'a>,
    transform: &MercTransform,
    cull_rect: egui::Rect,
) -> LodPoints<'a> {
    let cull_bounds = transform.viewport_merc_bounds(cull_rect.expand(CULL_BOUNDS_SLACK_PX));
    match track.lod.select(transform.px_per_merc(), MAX_LOD_ERROR_PX) {
        Some(level) => LodPoints::Level {
            indices: level.indices(),
            placed,
            walk: ChunkedWalk::new(level.chunk_bounds(), level.indices().len(), cull_bounds),
        },
        None => LodPoints::Full {
            placed,
            walk: ChunkedWalk::new(
                track.lod.full_point_chunk_bounds(),
                placed.len(),
                cull_bounds,
            ),
        },
    }
}

/// What [`lod_points`] walks: the entries of one stored [`LodLevel`], or the
/// track's full point list.
pub(crate) enum LodPoints<'a> {
    Level {
        indices: &'a [u32],
        placed: PlacedPoints<'a>,
        walk: ChunkedWalk<'a>,
    },
    Full {
        placed: PlacedPoints<'a>,
        walk: ChunkedWalk<'a>,
    },
}

impl<'a> Iterator for LodPoints<'a> {
    type Item = (usize, PlacedPoint<'a>);

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            match self {
                Self::Level {
                    indices,
                    placed,
                    walk,
                } => {
                    let slot = walk.next_slot()?;
                    if let Some(&i) = indices.get(slot)
                        && let Ok(pi) = usize::try_from(i)
                        && let Some(point) = placed.get(pi)
                    {
                        return Some((pi, point));
                    }
                }
                Self::Full { placed, walk } => {
                    let slot = walk.next_slot()?;
                    if let Some(point) = placed.get(slot) {
                        return Some((slot, point));
                    }
                }
            }
        }
    }
}

/// The slots of one chunked sequence of points a frame visits: every slot of
/// a chunk the cull bounds reach, and the first and the last slot of a chunk
/// they miss.
pub(crate) struct ChunkedWalk<'a> {
    chunk_bounds: &'a [MercBounds],
    slot_count: usize,
    cull_bounds: MercBounds,
    next_slot: usize,
}

impl<'a> ChunkedWalk<'a> {
    fn new(chunk_bounds: &'a [MercBounds], slot_count: usize, cull_bounds: MercBounds) -> Self {
        Self {
            chunk_bounds,
            slot_count,
            cull_bounds,
            next_slot: 0,
        }
    }

    /// The next slot to visit, `None` past the end of the sequence.
    fn next_slot(&mut self) -> Option<usize> {
        let slot = self.next_slot;
        if slot >= self.slot_count {
            return None;
        }
        let chunk = slot / LOD_CHUNK_POINTS;
        let opens_a_skipped_chunk =
            slot.is_multiple_of(LOD_CHUNK_POINTS) && self.chunk_lies_outside_the_cull_bounds(chunk);
        self.next_slot = if opens_a_skipped_chunk {
            let chunk_end = ((chunk + 1) * LOD_CHUNK_POINTS).min(self.slot_count);
            chunk_end.saturating_sub(1).max(slot + 1)
        } else {
            slot + 1
        };
        Some(slot)
    }

    /// Whether every point of `chunk` lies beyond one edge of the cull
    /// bounds. A chunk across the antimeridian never does, and a chunk with
    /// no stored bounds is walked whole.
    fn chunk_lies_outside_the_cull_bounds(&self, chunk: usize) -> bool {
        self.chunk_bounds.get(chunk).is_some_and(|bounds| {
            !bounds.crosses_the_antimeridian() && !bounds.intersects(self.cull_bounds)
        })
    }
}

#[cfg(test)]
mod tests {
    use gt_types::coordinates::Longitude;
    use gt_types::{LoadedTrack, PlacedPoint, PlacedPoints};
    use rstest::rstest;

    use super::{LOD_CHUNK_POINTS, Latitude, MercTransform, lod_points, wrap_longitude_degrees};
    use crate::polyline::{CULL_MARGIN_PX, MAX_LOD_ERROR_PX, VisiblePath, visible_path};

    /// Asserts `a` and `b` are within `1e-9` of each other - tight enough to
    /// catch a wrong wrap while tolerating ordinary `f64` rounding noise.
    fn assert_deg_close(a: f64, b: f64) {
        assert!((a - b).abs() < 1e-9, "expected {a} ≈ {b}");
    }

    /// Regression test: values already inside `Longitude`'s range must pass
    /// through unchanged (the wrap must be a no-op for ordinary positions).
    #[test]
    fn wrap_longitude_degrees_is_identity_in_range() {
        for deg in [-180.0, -179.999, -90.0, 0.0, 12.5638, 90.0, 179.999] {
            assert_deg_close(wrap_longitude_degrees(deg), deg);
        }
    }

    /// Regression test: longitudes past the antimeridian - as `unproject` can
    /// return at low zoom - must wrap to the equivalent meridian inside
    /// `Longitude`'s `[-180, 180]` range. An unwrapped value panics in
    /// `Longitude::new`.
    #[test]
    fn wrap_longitude_degrees_wraps_past_antimeridian() {
        assert_deg_close(
            wrap_longitude_degrees(195.925_437_518_683_45),
            -164.074_562_481_316_55,
        );
        assert_deg_close(
            wrap_longitude_degrees(184.015_191_562_275_4),
            -175.984_808_437_724_6,
        );
        // A full extra revolution must wrap back to the same meridian.
        assert_deg_close(wrap_longitude_degrees(540.0), wrap_longitude_degrees(180.0));
        assert_deg_close(
            wrap_longitude_degrees(-541.0),
            wrap_longitude_degrees(-181.0),
        );
    }

    /// The centre-latitude scale inverts `mercator::normalize`'s y exactly:
    /// for a viewport centred at a known latitude, the derived pixels per
    /// metre must match the direct computation at that latitude.
    #[rstest::rstest]
    #[case::equator(0.0)]
    #[case::mid_north(45.0)]
    #[case::copenhagen_ish(55.68)]
    #[case::mid_south(-55.0)]
    #[case::near_pole(84.0)]
    fn pixels_per_meter_at_center_inverts_the_projection(#[case] lat_deg: f64) {
        let lat = Latitude::new(lat_deg);
        let transform = MercTransform::for_test_centered(1_000_000.0, lat);
        let direct = transform.pixels_per_meter(lat);
        let derived = transform.pixels_per_meter_at_center();
        assert!(
            ((derived - direct) / direct).abs() < 1e-12,
            "lat {lat_deg}: derived {derived} vs direct {direct}"
        );
    }

    /// The map widget rect the walk tests frame their tracks in.
    const MAP_RECT: egui::Rect = egui::Rect {
        min: egui::pos2(0.0, 0.0),
        max: egui::pos2(800.0, 600.0),
    };

    /// A track through `positions`, with the LOD levels and chunk bounds the
    /// track builder computes for them.
    fn track_through(positions: &[(Latitude, Longitude)]) -> LoadedTrack {
        let points = gt_test_utils::nav_points_at_positions(
            chrono::DateTime::<chrono::Utc>::UNIX_EPOCH,
            positions,
        );
        let mut track = gt_test_utils::loaded_track_with_points(points);
        let lod = track.placed_points().map(gt_track_builder::build_track_lod);
        if let Some(lod) = lod {
            track.lod = lod;
        }
        track
    }

    /// 10 000 fixes running east along one parallel, about 22 m apart.
    fn straight_line() -> Vec<(Latitude, Longitude)> {
        (0..10_000)
            .map(|i| {
                (
                    Latitude::new(55.0),
                    Longitude::new(12.0 + f64::from(i) * 0.000_2),
                )
            })
            .collect()
    }

    /// 1200 fixes on twelve rows 0.002° apart, each run in the opposite
    /// direction to the one below it: a viewport over the middle of the rows
    /// sees the track leave and re-enter it once per row.
    fn rows_across_the_viewport() -> Vec<(Latitude, Longitude)> {
        (0..12)
            .flat_map(|row| {
                (0..100).map(move |i| {
                    let along = if row % 2 == 0 { i } else { 99 - i };
                    (
                        Latitude::new(55.0 + f64::from(row) * 0.002),
                        Longitude::new(12.0 + f64::from(along) * 0.000_2),
                    )
                })
            })
            .collect()
    }

    /// 300 fixes running east from 179°E over the antimeridian, 0.01° apart.
    fn across_the_antimeridian() -> Vec<(Latitude, Longitude)> {
        (0..300)
            .map(|i| {
                let degrees = 179.0 + f64::from(i) * 0.01;
                (
                    Latitude::new(0.0),
                    Longitude::new(wrap_longitude_degrees(degrees)),
                )
            })
            .collect()
    }

    /// `count` fixes of the straight line, for the tracks below one chunk.
    fn first_fixes(count: usize) -> Vec<(Latitude, Longitude)> {
        straight_line().into_iter().take(count).collect()
    }

    /// The viewports every shape is walked in: four zooms, from the whole
    /// world to a few metres across, over the track's first fix, over its
    /// middle fix, and over a position half a world away.
    fn viewports(positions: &[(Latitude, Longitude)]) -> Vec<MercTransform> {
        let anchors = [
            positions.first().copied(),
            positions.get(positions.len() / 2).copied(),
            Some((Latitude::new(-33.87), Longitude::new(151.21))),
        ];
        [1e4, 2_f64.powi(19), 3e7, 2_f64.powi(30)]
            .into_iter()
            .flat_map(|world_px| {
                anchors.into_iter().flatten().map(move |(lat, lon)| {
                    MercTransform::for_test_view(world_px, lat, lon, MAP_RECT.center())
                })
            })
            .collect()
    }

    /// The walk as it ran before the chunk bounds: every point of the
    /// selected level, or every point of the track.
    fn unbounded_lod_points<'a>(
        track: &'a LoadedTrack,
        placed: PlacedPoints<'a>,
        transform: &MercTransform,
    ) -> Vec<(usize, PlacedPoint<'a>)> {
        match track.lod.select(transform.px_per_merc(), MAX_LOD_ERROR_PX) {
            Some(level) => level
                .indices()
                .iter()
                .filter_map(|&i| {
                    let pi = usize::try_from(i).ok()?;
                    Some((pi, placed.get(pi)?))
                })
                .collect(),
            None => placed.iter().enumerate().collect(),
        }
    }

    /// The polyline the track renderer draws from `points`, keyed on the
    /// ghost flag as the renderer keys it.
    fn path_of<'a>(
        points: impl Iterator<Item = (usize, PlacedPoint<'a>)>,
        transform: &MercTransform,
        cull_rect: egui::Rect,
    ) -> VisiblePath<bool> {
        visible_path(
            points.map(|(_, point)| {
                (
                    point.fix.tpv.heading().is_none(),
                    transform.to_screen(point.merc()),
                )
            }),
            cull_rect,
        )
    }

    #[rstest]
    #[case::straight_line(straight_line())]
    #[case::rows_across_the_viewport(rows_across_the_viewport())]
    #[case::across_the_antimeridian(across_the_antimeridian())]
    #[case::shorter_than_one_chunk(first_fixes(40))]
    #[case::two_fixes(first_fixes(2))]
    #[case::one_fix(first_fixes(1))]
    fn the_bounded_walk_draws_the_path_the_unbounded_walk_draws(
        #[case] positions: Vec<(Latitude, Longitude)>,
    ) {
        let track = track_through(&positions);
        let placed = track
            .placed_points()
            .expect("every fixture fix has a position");
        let cull_rect = MAP_RECT.expand(CULL_MARGIN_PX);
        for transform in viewports(&positions) {
            let bounded = path_of(
                lod_points(&track, placed, &transform, cull_rect),
                &transform,
                cull_rect,
            );
            let unbounded = path_of(
                unbounded_lod_points(&track, placed, &transform).into_iter(),
                &transform,
                cull_rect,
            );
            assert_eq!(
                bounded,
                unbounded,
                "at {} px per world",
                transform.px_per_merc()
            );
        }
    }

    /// The path is the unbounded walk's wherever a chunk boundary falls
    /// relative to the viewport edge, since the walk keeps the segment
    /// entering a skipped chunk and the one leaving it.
    #[test]
    fn the_path_matches_at_every_chunk_phase_against_the_viewport_edge() {
        let positions = straight_line();
        let track = track_through(&positions);
        let placed = track
            .placed_points()
            .expect("every fixture fix has a position");
        let cull_rect = MAP_RECT.expand(CULL_MARGIN_PX);
        // Anchoring the viewport one fix further along moves the chunk
        // boundaries one fix across it: at this scale the fixes are about
        // 9 px apart.
        let world_px = 2_f64.powi(24);
        for anchor in 0..LOD_CHUNK_POINTS {
            let (lat, lon) = positions[positions.len() / 2 + anchor];
            let transform = MercTransform::for_test_view(world_px, lat, lon, MAP_RECT.center());
            let bounded = path_of(
                lod_points(&track, placed, &transform, cull_rect),
                &transform,
                cull_rect,
            );
            let unbounded = path_of(
                unbounded_lod_points(&track, placed, &transform).into_iter(),
                &transform,
                cull_rect,
            );
            assert_eq!(bounded, unbounded, "anchored on fix {anchor} of the chunk");
        }
    }

    /// A viewport over the middle of a long track holds a few of its fixes,
    /// and the walk stops short of the rest.
    #[rstest]
    #[case::from_a_stored_level(2_f64.powi(19), true)]
    #[case::from_the_full_point_list(2_f64.powi(30), false)]
    fn the_walk_skips_the_chunks_outside_the_cull_rect(
        #[case] world_px: f64,
        #[case] walks_a_stored_level: bool,
    ) {
        let positions = straight_line();
        let track = track_through(&positions);
        let placed = track
            .placed_points()
            .expect("every fixture fix has a position");
        let (lat, lon) = positions[positions.len() / 2];
        let transform = MercTransform::for_test_view(world_px, lat, lon, MAP_RECT.center());
        assert_eq!(
            track
                .lod
                .select(transform.px_per_merc(), MAX_LOD_ERROR_PX)
                .is_some(),
            walks_a_stored_level
        );

        let cull_rect = MAP_RECT.expand(CULL_MARGIN_PX);
        let walked = lod_points(&track, placed, &transform, cull_rect).count();
        let unbounded = unbounded_lod_points(&track, placed, &transform).len();
        assert!(
            walked < unbounded / 2,
            "walked {walked} of {unbounded} points"
        );
    }
}

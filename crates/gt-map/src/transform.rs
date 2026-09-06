//! Per-frame projection from normalised Mercator coordinates to screen
//! pixels, and the LOD-aware point iteration built on top of it.

use std::ops::Range;

use chrono::{DateTime, Utc};
use gt_filter::GlobalFilter;
use gt_types::coordinates::{Latitude, Longitude};
use gt_types::{
    Extent, LoadedTrack, LodChunk, MercBounds, MercPoint, PlacedPoint, PlacedPoints, TimeRange,
    mercator,
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

/// What one frame's geometry walk cuts a track's chunks against: the Mercator
/// bounds the map draws inside, and `filter`'s time window.
#[derive(Debug, Clone, Copy)]
pub(crate) struct GeometryCull<'a> {
    merc_bounds: MercBounds,
    filter: &'a GlobalFilter,
}

impl<'a> GeometryCull<'a> {
    pub(crate) fn new(
        transform: &MercTransform,
        cull_rect: egui::Rect,
        filter: &'a GlobalFilter,
    ) -> Self {
        Self {
            merc_bounds: transform.viewport_merc_bounds(cull_rect.expand(CULL_BOUNDS_SLACK_PX)),
            filter,
        }
    }

    fn visit(self, extent: Extent) -> ChunkVisit {
        let span = extent.time();
        if !span.overlaps_window(self.filter.time_start, self.filter.time_end) {
            return ChunkVisit::Nothing;
        }
        if !self.window_reaches_past_both_ends_of(span) {
            return ChunkVisit::EverySlotInTheWindow;
        }
        let merc = extent.merc();
        if merc.crosses_the_antimeridian() || merc.intersects(self.merc_bounds) {
            ChunkVisit::EverySlot
        } else {
            ChunkVisit::FirstAndLastSlot
        }
    }

    /// Whether every instant of `span` falls in the window. An absent end of
    /// the window is unbounded.
    fn window_reaches_past_both_ends_of(self, span: TimeRange) -> bool {
        self.filter
            .time_start
            .is_none_or(|start| start <= span.start)
            && self.filter.time_end.is_none_or(|end| span.end <= end)
    }

    fn keeps_the_fix_at(self, time: DateTime<Utc>) -> bool {
        gt_filter::point_passes_time_filter(time, self.filter)
    }
}

/// What the walk visits of one chunk.
#[derive(Debug, Clone, Copy)]
enum ChunkVisit {
    /// No fix of the chunk falls in the window.
    Nothing,
    /// The first and the last slot alone, which keep the segment entering the
    /// chunk and the one leaving it. The chunk is drawn beyond one edge of
    /// the cull bounds, and every fix of it falls in the window.
    FirstAndLastSlot,
    EverySlot,
    /// Every slot whose fix falls in the window. The chunk's span reaches
    /// past one end of the window.
    EverySlotInTheWindow,
}

/// Iterate `(index, point)` over the track's LOD level appropriate for the
/// current map scale, or over the full point list when no stored level is
/// fine enough (zoomed in, or no LOD built). Bounds polyline-pass iteration
/// by on-screen detail. `cull` cuts the walk in space and in time.
///
/// `placed` are `track`'s own points, which the caller has already gated on
/// the track having a geometry.
///
/// Of a chunk whose extent misses the cull bounds, the walk yields the first
/// and the last point alone: that chunk lies beyond one edge of the rect, and
/// [`crate::polyline::segment_outside`] drops every segment between the rest
/// of its points. Those two points keep the segment entering the chunk and
/// the one leaving it. The walk yields every point of a chunk with bounds
/// across the antimeridian, which cover two pieces of the world with no
/// single edge between them.
///
/// Of a chunk with every fix outside the window, the walk yields nothing.
/// Of a chunk whose span reaches past one end of the window, it yields every
/// point whose fix falls in the window, and it applies no spatial rule there.
/// Those points are not a contiguous run of the chunk: a track's timestamps
/// can step backwards. The segment entering the chunk and the segment leaving
/// it run to its first and its last point. Neither of those two is
/// necessarily a point the walk yields there.
pub(crate) fn lod_points<'a>(
    track: &'a LoadedTrack,
    placed: PlacedPoints<'a>,
    transform: &MercTransform,
    cull: GeometryCull<'a>,
) -> LodPoints<'a> {
    match track.lod.select(transform.px_per_merc(), MAX_LOD_ERROR_PX) {
        Some(level) => LodPoints::Level {
            indices: level.indices(),
            placed,
            walk: ChunkedWalk::new(level.chunks(), level.indices().len(), cull),
        },
        None => LodPoints::Full {
            placed,
            walk: ChunkedWalk::new(track.lod.full_point_chunks(), placed.len(), cull),
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
            let (indices, placed, walk) = match self {
                Self::Level {
                    indices,
                    placed,
                    walk,
                } => (Some(*indices), *placed, walk),
                Self::Full { placed, walk } => (None, *placed, walk),
            };
            let visited = walk.next_slot()?;
            let point_index = match indices {
                Some(indices) => indices
                    .get(visited.slot)
                    .and_then(|&i| usize::try_from(i).ok()),
                None => Some(visited.slot),
            };
            let Some(point_index) = point_index else {
                continue;
            };
            let Some(point) = placed.get(point_index) else {
                continue;
            };
            if visited.time_tested_per_fix
                && !walk.cull.keeps_the_fix_at(point.fix.tpv.time().utc())
            {
                continue;
            }
            return Some((point_index, point));
        }
    }
}

/// A slot the walk visits, and whether the walk tests the fix at it against
/// the time window before yielding it.
#[derive(Debug, Clone, Copy)]
struct VisitedSlot {
    slot: usize,
    time_tested_per_fix: bool,
}

/// The chunk the walk is on, and the [`ChunkVisit`] computed for its extent
/// when the walk opened it. Every slot of the chunk uses that one value.
struct OpenChunk {
    slots: Range<usize>,
    visit: ChunkVisit,
}

/// The slots of one chunked sequence of points a frame visits: every slot of a
/// chunk the cull keeps whole, the first and the last slot of a chunk drawn
/// outside the cull bounds, and no slot of a chunk recorded outside the time
/// window.
pub(crate) struct ChunkedWalk<'a> {
    chunks: &'a [LodChunk],
    slot_count: usize,
    cull: GeometryCull<'a>,
    next_slot: usize,
    /// Where the search for the chunk holding `next_slot` resumes. The walk
    /// never looks at a chunk it has left: it visits them in order.
    next_chunk: usize,
    open: Option<OpenChunk>,
}

impl<'a> ChunkedWalk<'a> {
    fn new(chunks: &'a [LodChunk], slot_count: usize, cull: GeometryCull<'a>) -> Self {
        Self {
            chunks,
            slot_count,
            cull,
            next_slot: 0,
            next_chunk: 0,
            open: None,
        }
    }

    /// The next slot to visit, `None` past the end of the sequence.
    fn next_slot(&mut self) -> Option<VisitedSlot> {
        loop {
            let slot = self.next_slot;
            if slot >= self.slot_count {
                return None;
            }
            if !self
                .open
                .as_ref()
                .is_some_and(|open| open.slots.contains(&slot))
            {
                self.open = self.chunk_holding(slot);
            }
            let Some((end, visit)) = self.open.as_ref().map(|open| (open.slots.end, open.visit))
            else {
                // No stored chunk covers this slot. The walk visits it and
                // tests its fix against the window.
                self.next_slot = slot + 1;
                return Some(VisitedSlot {
                    slot,
                    time_tested_per_fix: true,
                });
            };
            match visit {
                ChunkVisit::Nothing => self.next_slot = end,
                ChunkVisit::FirstAndLastSlot => {
                    self.next_slot = end.saturating_sub(1).max(slot + 1);
                    return Some(VisitedSlot {
                        slot,
                        time_tested_per_fix: false,
                    });
                }
                ChunkVisit::EverySlot => {
                    self.next_slot = slot + 1;
                    return Some(VisitedSlot {
                        slot,
                        time_tested_per_fix: false,
                    });
                }
                ChunkVisit::EverySlotInTheWindow => {
                    self.next_slot = slot + 1;
                    return Some(VisitedSlot {
                        slot,
                        time_tested_per_fix: true,
                    });
                }
            }
        }
    }

    /// The chunk covering `slot`, `None` when no stored chunk does.
    fn chunk_holding(&mut self, slot: usize) -> Option<OpenChunk> {
        while self
            .chunks
            .get(self.next_chunk)
            .is_some_and(|chunk| chunk.slots().end <= slot)
        {
            self.next_chunk += 1;
        }
        let chunk = self.chunks.get(self.next_chunk)?;
        let slots = chunk.slots();
        slots.contains(&slot).then(|| OpenChunk {
            slots,
            visit: self.cull.visit(chunk.extent()),
        })
    }
}

#[cfg(test)]
mod tests {
    use chrono::{DateTime, TimeDelta, Utc};
    use gt_filter::GlobalFilter;
    use gt_track_builder::LOD_CHUNK_POINTS;
    use gt_types::coordinates::Longitude;
    use gt_types::{LoadedTrack, PlacedPoint, PlacedPoints};
    use rstest::rstest;

    use super::{GeometryCull, Latitude, MercTransform, lod_points, wrap_longitude_degrees};
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

    /// The instant the first fix of every fixture track is stamped at.
    const FIRST_FIX_TIME: DateTime<Utc> = DateTime::<Utc>::UNIX_EPOCH;

    fn at_second(offset_secs: i64) -> DateTime<Utc> {
        FIRST_FIX_TIME + TimeDelta::seconds(offset_secs)
    }

    /// The window `[start, end]` in seconds from [`FIRST_FIX_TIME`].
    fn window(start_secs: i64, end_secs: i64) -> GlobalFilter {
        GlobalFilter {
            time_start: Some(at_second(start_secs)),
            time_end: Some(at_second(end_secs)),
            ..GlobalFilter::default()
        }
    }

    /// A track through `positions` with its `i`th fix stamped
    /// `offset_secs(i)` from [`FIRST_FIX_TIME`], and the LOD levels and
    /// chunks the track builder computes for it.
    fn track_stamped(
        positions: &[(Latitude, Longitude)],
        offset_secs: impl Fn(usize) -> i64,
    ) -> LoadedTrack {
        let points = gt_test_utils::nav_points_stamped(FIRST_FIX_TIME, positions, offset_secs);
        let mut track = gt_test_utils::loaded_track_with_points(points);
        let lod = track.placed_points().map(gt_track_builder::build_track_lod);
        if let Some(lod) = lod {
            track.lod = lod;
        }
        track
    }

    /// A track through `positions`, one fix per second.
    fn track_through(positions: &[(Latitude, Longitude)]) -> LoadedTrack {
        track_stamped(positions, |i| i.try_into().unwrap_or(i64::MAX))
    }

    /// Neither end of a chunk is its earliest or its latest fix. The 30th fix
    /// of every chunk is stamped a hundred seconds before the fixes around
    /// it.
    fn offset_secs_stepping_backwards_inside_every_chunk(i: usize) -> i64 {
        let seconds = i64::try_from(i).unwrap_or(i64::MAX);
        match i % LOD_CHUNK_POINTS == 30 {
            true => seconds - 100,
            false => seconds,
        }
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

    /// The walk as it ran before the chunk extents: every point of the
    /// selected level, or every point of the track, with the caller's own
    /// per-point time filter over it.
    fn unbounded_lod_points<'a>(
        track: &'a LoadedTrack,
        placed: PlacedPoints<'a>,
        transform: &MercTransform,
        filter: &GlobalFilter,
    ) -> Vec<(usize, PlacedPoint<'a>)> {
        let every_point: Vec<(usize, PlacedPoint<'a>)> =
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
            };
        every_point
            .into_iter()
            .filter(|(_, point)| {
                gt_filter::point_passes_time_filter(point.fix.tpv.time().utc(), filter)
            })
            .collect()
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

    /// Asserts the bounded walk draws what the unbounded walk drew and the
    /// caller then filtered per point, in every viewport the fixtures are
    /// framed in.
    fn assert_the_walks_draw_the_same_path(
        track: &LoadedTrack,
        positions: &[(Latitude, Longitude)],
        filter: &GlobalFilter,
    ) {
        let placed = track
            .placed_points()
            .expect("every fixture fix has a position");
        let cull_rect = MAP_RECT.expand(CULL_MARGIN_PX);
        for transform in viewports(positions) {
            let cull = GeometryCull::new(&transform, cull_rect, filter);
            let bounded = path_of(
                lod_points(track, placed, &transform, cull),
                &transform,
                cull_rect,
            );
            let unbounded = path_of(
                unbounded_lod_points(track, placed, &transform, filter).into_iter(),
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
        assert_the_walks_draw_the_same_path(&track, &positions, &GlobalFilter::default());
    }

    /// A window ending at 5 000 s cuts through the middle of one chunk, and
    /// one from 5 010 s to 5 040 s opens and closes inside a single chunk.
    /// The straight line runs one fix per second over 10 000 seconds.
    #[rstest]
    #[case::boundary_inside_a_chunk(window(3_000, 5_000))]
    #[case::inside_one_chunk(window(5_010, 5_040))]
    fn the_bounded_walk_draws_the_windowed_path_the_unbounded_walk_draws(
        #[case] filter: GlobalFilter,
    ) {
        let positions = straight_line();
        let track = track_through(&positions);
        assert_the_walks_draw_the_same_path(&track, &positions, &filter);
    }

    /// The fixes with a timestamp inside the window are not one contiguous
    /// run of the chunk. The first and the last fix of the chunk both have a
    /// timestamp outside the window.
    #[test]
    fn the_bounded_walk_draws_the_windowed_path_across_a_backward_time_step() {
        let positions = straight_line();
        let track = track_stamped(
            &positions,
            offset_secs_stepping_backwards_inside_every_chunk,
        );
        assert_the_walks_draw_the_same_path(&track, &positions, &window(3_000, 5_000));
    }

    #[test]
    fn the_walk_yields_nothing_of_a_track_the_window_excludes() {
        let positions = straight_line();
        let track = track_through(&positions);
        let placed = track
            .placed_points()
            .expect("every fixture fix has a position");
        let cull_rect = MAP_RECT.expand(CULL_MARGIN_PX);
        let (lat, lon) = positions[positions.len() / 2];
        let transform = MercTransform::for_test_view(1e4, lat, lon, MAP_RECT.center());
        let filter = window(30_000, 40_000);
        let cull = GeometryCull::new(&transform, cull_rect, &filter);

        assert_eq!(lod_points(&track, placed, &transform, cull).count(), 0);
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
        let filter = GlobalFilter::default();
        // Anchoring the viewport one fix further along moves the chunk
        // boundaries one fix across it: at this scale the fixes are about
        // 9 px apart.
        let world_px = 2_f64.powi(24);
        for anchor in 0..LOD_CHUNK_POINTS {
            let (lat, lon) = positions[positions.len() / 2 + anchor];
            let transform = MercTransform::for_test_view(world_px, lat, lon, MAP_RECT.center());
            let cull = GeometryCull::new(&transform, cull_rect, &filter);
            let bounded = path_of(
                lod_points(&track, placed, &transform, cull),
                &transform,
                cull_rect,
            );
            let unbounded = path_of(
                unbounded_lod_points(&track, placed, &transform, &filter).into_iter(),
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
        let filter = GlobalFilter::default();
        let cull = GeometryCull::new(&transform, cull_rect, &filter);
        let walked = lod_points(&track, placed, &transform, cull).count();
        let unbounded = unbounded_lod_points(&track, placed, &transform, &filter).len();
        assert!(
            walked < unbounded / 2,
            "walked {walked} of {unbounded} points"
        );
    }
}

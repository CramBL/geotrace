//! Per-frame projection from normalised Mercator coordinates to screen
//! pixels, and the LOD-aware point iteration built on top of it.

use gt_types::coordinates::{Latitude, Longitude};
use gt_types::{LoadedTrack, MercPoint, PlacedPoint, PlacedPoints, mercator};
use walkers::MapMemory;

use crate::polyline::MAX_LOD_ERROR_PX;

/// Wrap a longitude in degrees into `Longitude`'s valid `[-180, 180]` range.
///
/// At low zoom the viewport can span more than 360° of longitude, so the
/// pixel column at the viewport centre may sit past the antimeridian wrap and
/// `Projector::unproject` returns e.g. 185° instead of the equivalent -175°.
/// Longitude is periodic with period 360°, so the wrapped value names the same
/// meridian and is the correct input for [`Longitude::new`].
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
/// animations. Additionally, viewport culling arithmetic done in f32 before
/// casting to f64 has the same issue.
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

    /// Pixels per metre at the given latitude.
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

    /// Like [`MercTransform::for_test`], with the viewport centred on the
    /// given latitude (for scale math that depends on the centre).
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

    /// Pixels per metre at the given latitude. See [`MapScale`].
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

/// Iterate `(index, point)` over the track's LOD level appropriate for the
/// current map scale, or over the full point list when no stored level is
/// fine enough (zoomed in, or no LOD built). Bounds polyline-pass iteration
/// by on-screen detail instead of recording size.
///
/// `placed` are `track`'s own points, which the caller has already gated on
/// the track having a geometry.
pub(crate) fn lod_points<'a>(
    track: &'a LoadedTrack,
    placed: PlacedPoints<'a>,
    transform: &MercTransform,
) -> Box<dyn Iterator<Item = (usize, PlacedPoint<'a>)> + 'a> {
    match track.lod.select(transform.px_per_merc(), MAX_LOD_ERROR_PX) {
        Some(indices) => Box::new(indices.iter().filter_map(move |&i| {
            let pi = usize::try_from(i).ok()?;
            Some((pi, placed.get(pi)?))
        })),
        None => Box::new(placed.iter().enumerate()),
    }
}

#[cfg(test)]
mod tests {
    use super::{Latitude, MercTransform, wrap_longitude_degrees};

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
    /// `Longitude`'s `[-180, 180]` range rather than panic in `Longitude::new`.
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
}

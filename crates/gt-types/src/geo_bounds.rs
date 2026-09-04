use crate::coordinates::{FULL_CIRCLE_DEGREES, HALF_CIRCLE_DEGREES, Latitude, Longitude};

const NORTH_POLE_DEGREES: f64 = 90.0;
const SOUTH_POLE_DEGREES: f64 = -90.0;

/// Slack for the arc comparisons below, 1e-6° of it: an evenly sampled lap
/// closes with an arc as long as its others, and a closed path's arcs sum to
/// an exact multiple of 360°, both only up to floating-point rounding.
const ARC_TOLERANCE_DEGREES: f64 = 1e-6;

/// Whether the closed path through a track's fixes encircles a pole, and
/// which one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PoleWinding {
    None,
    AroundNorthPole,
    AroundSouthPole,
}

impl PoleWinding {
    /// The winding of `fixes`, read in recorded order.
    ///
    /// It sums the shorter longitude arc between consecutive fixes, plus the
    /// arc from the last fix back to the first. That closing arc counts only
    /// when it is no longer than the longest arc between recorded fixes: a
    /// track that ended far from where it started is treated as an open line.
    /// The arcs of a closed path sum to a multiple of 360°, and a non-zero
    /// multiple means the path encircles a pole - the one in the hemisphere
    /// the fixes' mean latitude lies in.
    pub fn of_track(fixes: impl IntoIterator<Item = (Latitude, Longitude)>) -> Self {
        let mut fixes = fixes.into_iter();
        let Some((first_latitude, first_longitude)) = fixes.next() else {
            return Self::None;
        };
        let mut latitude_sum_degrees = first_latitude.as_degrees();
        let mut arc_sum_degrees = 0.0;
        let mut longest_arc_degrees = 0.0_f64;
        let mut previous_longitude = first_longitude;
        for (latitude, longitude) in fixes {
            let Some(arc_degrees) = previous_longitude.shorter_arc_degrees_to(longitude) else {
                return Self::None;
            };
            latitude_sum_degrees += latitude.as_degrees();
            arc_sum_degrees += arc_degrees;
            longest_arc_degrees = longest_arc_degrees.max(arc_degrees.abs());
            previous_longitude = longitude;
        }

        let Some(closing_arc_degrees) = previous_longitude.shorter_arc_degrees_to(first_longitude)
        else {
            return Self::None;
        };
        if closing_arc_degrees.abs() > longest_arc_degrees + ARC_TOLERANCE_DEGREES {
            return Self::None;
        }

        let closed_sum_degrees = arc_sum_degrees + closing_arc_degrees;
        if closed_sum_degrees.abs() < FULL_CIRCLE_DEGREES - ARC_TOLERANCE_DEGREES {
            return Self::None;
        }
        if latitude_sum_degrees < 0.0 {
            Self::AroundSouthPole
        } else {
            Self::AroundNorthPole
        }
    }
}

/// A longitude interval, measured eastward from `start` over `span_degrees`.
///
/// `start` is in `[-180, 180]` and the span in `[0, 360]`: a range across the
/// antimeridian is `start = 179.0, span = 1.5`, and one around a pole is
/// [`LonRange::full_circle`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LonRange {
    start: Longitude,
    span_degrees: f64,
}

impl LonRange {
    pub fn single_meridian(longitude: Longitude) -> Self {
        Self {
            start: longitude,
            span_degrees: 0.0,
        }
    }

    /// Every meridian, for a track that encloses a pole.
    pub fn full_circle() -> Self {
        Self {
            start: Longitude::new(-180.0),
            span_degrees: FULL_CIRCLE_DEGREES,
        }
    }

    /// The range covering `longitudes`, each one grown onto the running range
    /// by whichever arc is shorter. A longitude exactly opposite the range's
    /// nearer end grows it eastward.
    ///
    /// `None` for an empty iterator.
    pub fn from_longitudes(longitudes: impl IntoIterator<Item = Longitude>) -> Option<Self> {
        let mut longitudes = longitudes.into_iter();
        let mut range = GrowingLonRange::starting_at(longitudes.next()?);
        for longitude in longitudes {
            range.extend_to(longitude);
        }
        Some(range.finish())
    }

    /// The narrower of the two ranges covering both `self` and `other`: the
    /// gap closed is whichever of the two between them is shorter, matching
    /// how [`LonRange::from_longitudes`] grows over a single longitude.
    pub fn union(self, other: Self) -> Self {
        if self.is_full_circle() || other.is_full_circle() {
            return Self::full_circle();
        }
        let eastward = self.grown_east_over(other);
        let westward = other.grown_east_over(self);
        if eastward.span_degrees <= westward.span_degrees {
            eastward
        } else {
            westward
        }
    }

    /// `self` grown eastward until it reaches the eastern end of `other`.
    fn grown_east_over(self, other: Self) -> Self {
        let span = (self.eastward_offset_degrees(other.start) + other.span_degrees)
            .max(self.span_degrees)
            .min(FULL_CIRCLE_DEGREES);
        Self {
            start: self.start,
            span_degrees: span,
        }
    }

    pub fn start(self) -> Longitude {
        self.start
    }

    pub fn span_degrees(self) -> f64 {
        self.span_degrees
    }

    /// The eastern end, in `[-180, 180]`.
    pub fn end(self) -> Longitude {
        self.eastward_of_start(self.span_degrees)
    }

    /// The meridian halfway along the range, in `[-180, 180]`. For a full
    /// circle it is the one opposite `start`.
    pub fn center(self) -> Longitude {
        self.eastward_of_start(self.span_degrees / 2.0)
    }

    pub fn contains(self, longitude: Longitude) -> bool {
        self.eastward_offset_degrees(longitude) <= self.span_degrees
    }

    pub fn is_full_circle(self) -> bool {
        self.span_degrees >= FULL_CIRCLE_DEGREES
    }

    /// How far east of `start` `longitude` lies, in `[0, 360)`.
    fn eastward_offset_degrees(self, longitude: Longitude) -> f64 {
        (longitude.as_degrees() - self.start.as_degrees()).rem_euclid(FULL_CIRCLE_DEGREES)
    }

    /// `start` moved east by `offset_degrees`, folded back into `[-180, 180]`.
    /// An offset landing on the antimeridian reads as 180°, which projects to
    /// the eastern edge of the world.
    fn eastward_of_start(self, offset_degrees: f64) -> Longitude {
        let degrees = self.start.as_degrees() + offset_degrees;
        Longitude::new(if degrees > HALF_CIRCLE_DEGREES {
            degrees - FULL_CIRCLE_DEGREES
        } else {
            degrees
        })
    }
}

/// The fold behind [`LonRange::from_longitudes`].
///
/// An end of the range it builds is always inside that range: [`Self::finish`]
/// derives the span from the same raw longitudes [`LonRange::contains`] later
/// takes its offsets against, so the two agree bit for bit.
#[derive(Debug, Clone, Copy)]
struct GrowingLonRange {
    west_degrees: f64,
    east_degrees: f64,
}

impl GrowingLonRange {
    fn starting_at(longitude: Longitude) -> Self {
        Self {
            west_degrees: longitude.as_degrees(),
            east_degrees: longitude.as_degrees(),
        }
    }

    fn span_degrees(self) -> f64 {
        (self.east_degrees - self.west_degrees).rem_euclid(FULL_CIRCLE_DEGREES)
    }

    fn extend_to(&mut self, longitude: Longitude) {
        let degrees = longitude.as_degrees();
        let offset = (degrees - self.west_degrees).rem_euclid(FULL_CIRCLE_DEGREES);
        let span = self.span_degrees();
        if offset <= span {
            return;
        }
        if offset - span <= FULL_CIRCLE_DEGREES - offset {
            self.east_degrees = degrees;
        } else {
            self.west_degrees = degrees;
        }
    }

    fn finish(self) -> LonRange {
        LonRange {
            start: Longitude::new(self.west_degrees),
            span_degrees: self.span_degrees(),
        }
    }
}

/// A latitude interval, `south` never north of `north`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LatRange {
    south: Latitude,
    north: Latitude,
}

impl LatRange {
    pub fn single_parallel(latitude: Latitude) -> Self {
        Self {
            south: latitude,
            north: latitude,
        }
    }

    /// `None` for an empty iterator.
    pub fn from_latitudes(latitudes: impl IntoIterator<Item = Latitude>) -> Option<Self> {
        let mut latitudes = latitudes.into_iter();
        let mut range = Self::single_parallel(latitudes.next()?);
        for latitude in latitudes {
            range = range.extended_to(latitude);
        }
        Some(range)
    }

    pub fn extended_to(self, latitude: Latitude) -> Self {
        Self {
            south: Latitude::new(self.south.as_degrees().min(latitude.as_degrees())),
            north: Latitude::new(self.north.as_degrees().max(latitude.as_degrees())),
        }
    }

    pub fn south(self) -> Latitude {
        self.south
    }

    pub fn north(self) -> Latitude {
        self.north
    }

    pub fn center(self) -> Latitude {
        Latitude::new((self.south.as_degrees() + self.north.as_degrees()) / 2.0)
    }

    /// The diameter, in degrees, of the polar cap a box over every meridian
    /// covers at these latitudes. The arc runs from the parallel nearest the
    /// equator over the pole to the far side. A range reaching both
    /// hemispheres gives a half circle.
    pub fn arc_across_the_pole_degrees(self) -> f64 {
        let (south_degrees, north_degrees) = (self.south.as_degrees(), self.north.as_degrees());
        let nearest_the_equator_degrees = if south_degrees <= 0.0 && north_degrees >= 0.0 {
            0.0
        } else {
            south_degrees.abs().min(north_degrees.abs())
        };
        2.0 * (NORTH_POLE_DEGREES - nearest_the_equator_degrees)
    }

    pub fn contains(self, latitude: Latitude) -> bool {
        (self.south.as_degrees()..=self.north.as_degrees()).contains(&latitude.as_degrees())
    }
}

/// The geographic extent of a set of positions.
///
/// Its longitude extent is a [`LonRange`], so it holds a track that crosses
/// the antimeridian or circles a pole.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GeoBounds {
    pub lat: LatRange,
    pub lon: LonRange,
}

impl GeoBounds {
    pub fn single_position(latitude: Latitude, longitude: Longitude) -> Self {
        Self {
            lat: LatRange::single_parallel(latitude),
            lon: LonRange::single_meridian(longitude),
        }
    }

    /// The bounds covering `positions`, growing the longitude extent as
    /// [`LonRange::from_longitudes`] does.
    ///
    /// `None` for an empty iterator.
    pub fn from_positions(
        positions: impl IntoIterator<Item = (Latitude, Longitude)>,
    ) -> Option<Self> {
        let mut positions = positions.into_iter();
        Some(Self::from_first_position_and_rest(
            positions.next()?,
            positions,
        ))
    }

    /// The same fold as [`GeoBounds::from_positions`], for a caller holding a
    /// position set the type system already proves non-empty.
    pub fn from_first_position_and_rest(
        (first_latitude, first_longitude): (Latitude, Longitude),
        rest: impl IntoIterator<Item = (Latitude, Longitude)>,
    ) -> Self {
        let mut lat = LatRange::single_parallel(first_latitude);
        let mut lon = GrowingLonRange::starting_at(first_longitude);
        for (latitude, longitude) in rest {
            lat = lat.extended_to(latitude);
            lon.extend_to(longitude);
        }
        Self {
            lat,
            lon: lon.finish(),
        }
    }

    /// This box as `winding` leaves it: a track that encircles a pole is held
    /// only by the cap over every meridian, reaching from its extreme fix to
    /// that pole.
    #[must_use]
    pub fn extended_to_the_encircled_pole(self, winding: PoleWinding) -> Self {
        match winding {
            PoleWinding::None => self,
            PoleWinding::AroundNorthPole => Self {
                lat: LatRange {
                    south: self.lat.south,
                    north: Latitude::new(NORTH_POLE_DEGREES),
                },
                lon: LonRange::full_circle(),
            },
            PoleWinding::AroundSouthPole => Self {
                lat: LatRange {
                    south: Latitude::new(SOUTH_POLE_DEGREES),
                    north: self.lat.north,
                },
                lon: LonRange::full_circle(),
            },
        }
    }

    /// The bounds covering both, closing the shorter of the two longitude
    /// gaps between them as [`LonRange::union`] does.
    pub fn union(self, other: Self) -> Self {
        Self {
            lat: self
                .lat
                .extended_to(other.lat.south())
                .extended_to(other.lat.north()),
            lon: self.lon.union(other.lon),
        }
    }

    pub fn center(self) -> (Latitude, Longitude) {
        (self.lat.center(), self.lon.center())
    }

    pub fn contains(self, latitude: Latitude, longitude: Longitude) -> bool {
        self.lat.contains(latitude) && self.lon.contains(longitude)
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::{FULL_CIRCLE_DEGREES, GeoBounds, LatRange, LonRange, PoleWinding};
    use crate::coordinates::{Latitude, Longitude};
    use crate::mercator;

    /// 1e-9° is about 0.1 mm.
    const DEGREES_TOLERANCE: f64 = 1e-9;

    /// An eastbound track over the antimeridian, 1.5° wide.
    const ACROSS_THE_ANTIMERIDIAN: &[f64] = &[179.0, 179.5, -179.9, -179.5];

    /// A receiver carried around the north pole, sampled a quarter turn apart.
    const AROUND_THE_NORTH_POLE: &[(f64, f64)] =
        &[(89.9, 0.0), (89.9, 90.0), (89.9, 180.0), (89.9, -90.0)];

    fn positions_of(fixes: &[(f64, f64)]) -> impl Iterator<Item = (Latitude, Longitude)> {
        fixes
            .iter()
            .map(|&(lat, lon)| (Latitude::new(lat), Longitude::new(lon)))
    }

    fn assert_degrees_close(actual: f64, expected: f64) {
        assert!(
            (actual - expected).abs() < DEGREES_TOLERANCE,
            "expected {expected}°, got {actual}°"
        );
    }

    fn range_over(degrees: &[f64]) -> LonRange {
        LonRange::from_longitudes(degrees.iter().map(|&d| Longitude::new(d)))
            .expect("at least one longitude")
    }

    #[rstest]
    #[case::across_the_antimeridian(ACROSS_THE_ANTIMERIDIAN, 179.0, 1.5)]
    #[case::local_eastbound(&[10.0, 11.0, 12.0], 10.0, 2.0)]
    #[case::local_westbound(&[12.0, 11.0, 10.0], 10.0, 2.0)]
    #[case::both_ways_from_the_first(&[10.0, 12.0, 8.0], 8.0, 4.0)]
    #[case::the_antimeridian_is_one_meridian(&[-180.0, 180.0], -180.0, 0.0)]
    #[case::single_longitude(&[42.0], 42.0, 0.0)]
    #[case::exactly_opposite_grows_east(&[0.0, 180.0], 0.0, 180.0)]
    fn from_longitudes_grows_by_the_shorter_arc(
        #[case] degrees: &[f64],
        #[case] expected_start: f64,
        #[case] expected_span: f64,
    ) {
        let range = range_over(degrees);
        assert_degrees_close(range.start().as_degrees(), expected_start);
        assert_degrees_close(range.span_degrees(), expected_span);
    }

    #[rstest]
    #[case::across_the_antimeridian(ACROSS_THE_ANTIMERIDIAN, -179.5, 179.75)]
    #[case::local(&[10.0, 12.0], 12.0, 11.0)]
    fn end_and_center_wrap_into_the_signed_range(
        #[case] degrees: &[f64],
        #[case] expected_end: f64,
        #[case] expected_center: f64,
    ) {
        let range = range_over(degrees);
        assert_degrees_close(range.end().as_degrees(), expected_end);
        assert_degrees_close(range.center().as_degrees(), expected_center);
    }

    #[rstest]
    #[case::at_the_western_end(179.0, true)]
    #[case::east_of_the_antimeridian(179.7, true)]
    #[case::west_of_the_antimeridian(-179.7, true)]
    #[case::at_the_eastern_end(-179.5, true)]
    #[case::just_west_of_the_range(178.9, false)]
    #[case::just_east_of_the_range(-179.4, false)]
    #[case::the_other_side_of_the_planet(0.0, false)]
    fn contains_covers_both_sides_of_the_antimeridian(
        #[case] degrees: f64,
        #[case] expected: bool,
    ) {
        let range = range_over(ACROSS_THE_ANTIMERIDIAN);
        assert_eq!(range.contains(Longitude::new(degrees)), expected);
    }

    #[rstest]
    #[case::west_edge(-180.0)]
    #[case::prime_meridian(0.0)]
    #[case::east_edge(180.0)]
    #[case::arbitrary(-97.3)]
    fn a_full_circle_contains_every_meridian(#[case] degrees: f64) {
        let range = LonRange::full_circle();
        assert!(range.is_full_circle());
        assert!(range.contains(Longitude::new(degrees)));
    }

    #[rstest]
    #[case::disjoint_across_the_antimeridian(&[179.0, 179.5], &[-179.9, -179.5], 179.0, 1.5)]
    #[case::overlapping(&[10.0, 14.0], &[12.0, 16.0], 10.0, 6.0)]
    #[case::one_inside_the_other(&[10.0, 20.0], &[12.0, 16.0], 10.0, 10.0)]
    #[case::the_gap_east_of_the_first_range_is_shorter(&[170.0, 175.0], &[-100.0, -95.0], 170.0, 95.0)]
    #[case::the_gap_east_of_the_second_range_is_shorter(&[-100.0, -95.0], &[170.0, 175.0], 170.0, 95.0)]
    #[case::a_single_meridian_east_of_the_range(&[10.0, 12.0], &[13.0], 10.0, 3.0)]
    fn union_closes_the_shorter_gap(
        #[case] left_degrees: &[f64],
        #[case] right_degrees: &[f64],
        #[case] expected_start: f64,
        #[case] expected_span: f64,
    ) {
        let union = range_over(left_degrees).union(range_over(right_degrees));

        assert_degrees_close(union.start().as_degrees(), expected_start);
        assert_degrees_close(union.span_degrees(), expected_span);
        for &degrees in left_degrees.iter().chain(right_degrees) {
            assert!(union.contains(Longitude::new(degrees)), "{degrees}°");
        }
    }

    #[test]
    fn a_union_with_a_full_circle_is_a_full_circle() {
        assert!(
            range_over(&[10.0, 12.0])
                .union(LonRange::full_circle())
                .is_full_circle()
        );
    }

    #[test]
    fn a_union_covering_every_meridian_is_a_full_circle() {
        let union = range_over(&[0.0, 120.0, -120.0]).union(range_over(&[120.0, -120.0, 0.0]));

        assert!(union.is_full_circle());
        assert_degrees_close(union.span_degrees(), FULL_CIRCLE_DEGREES);
    }

    #[test]
    fn bounds_unite_over_both_latitude_and_longitude_extents() {
        let west = GeoBounds::from_positions([(Latitude::new(60.0), Longitude::new(179.0))])
            .expect("one position");
        let east = GeoBounds::from_positions([(Latitude::new(58.0), Longitude::new(-179.5))])
            .expect("one position");
        let union = west.union(east);

        assert_degrees_close(union.lat.south().as_degrees(), 58.0);
        assert_degrees_close(union.lat.north().as_degrees(), 60.0);
        assert_degrees_close(union.lon.start().as_degrees(), 179.0);
        assert_degrees_close(union.lon.span_degrees(), 1.5);
    }

    #[test]
    fn a_grown_range_stops_short_of_a_full_circle() {
        assert!(!range_over(&[0.0, 90.0, 180.0, -90.0]).is_full_circle());
    }

    #[test]
    fn the_center_of_bounds_across_the_antimeridian_lands_on_the_track() {
        let bounds = GeoBounds::from_positions(
            ACROSS_THE_ANTIMERIDIAN
                .iter()
                .map(|&d| (Latitude::new(60.0), Longitude::new(d))),
        )
        .expect("at least one position");
        let (latitude, longitude) = bounds.center();

        assert_degrees_close(latitude.as_degrees(), 60.0);
        assert_degrees_close(longitude.as_degrees(), 179.75);
    }

    #[rstest]
    #[case::inside(60.5, 179.2, true)]
    #[case::north_of_the_bounds(61.5, 179.2, false)]
    #[case::south_of_the_bounds(59.5, 179.2, false)]
    #[case::east_of_the_bounds(60.5, -179.0, false)]
    fn bounds_contain_a_position_only_inside_both_extents(
        #[case] latitude: f64,
        #[case] longitude: f64,
        #[case] expected: bool,
    ) {
        let bounds = GeoBounds::from_positions([
            (Latitude::new(60.0), Longitude::new(179.0)),
            (Latitude::new(61.0), Longitude::new(-179.5)),
        ])
        .expect("at least one position");

        assert_eq!(
            bounds.contains(Latitude::new(latitude), Longitude::new(longitude)),
            expected
        );
    }

    #[rstest]
    #[case::around_the_north_pole(AROUND_THE_NORTH_POLE, PoleWinding::AroundNorthPole)]
    #[case::around_the_north_pole_the_other_way(
        &[(89.9, 0.0), (89.9, -90.0), (89.9, 180.0), (89.9, 90.0)],
        PoleWinding::AroundNorthPole
    )]
    #[case::around_the_south_pole(
        &[(-89.9, 0.0), (-89.9, 90.0), (-89.9, 180.0), (-89.9, -90.0)],
        PoleWinding::AroundSouthPole
    )]
    #[case::a_stationary_receiver_jittering_over_the_antimeridian(
        &[(89.99, 170.0), (89.99, -170.0), (89.99, 170.0), (89.99, -170.0)],
        PoleWinding::None
    )]
    #[case::across_the_antimeridian(
        &[(0.0, 179.0), (0.0, 179.5), (0.0, -179.9), (0.0, -179.5)],
        PoleWinding::None
    )]
    #[case::two_hundred_degrees_east_along_the_equator(
        &[(0.0, 0.0), (0.0, 100.0), (0.0, -160.0)],
        PoleWinding::None
    )]
    #[case::two_fixes(&[(55.0, 12.0), (55.2, 12.5)], PoleWinding::None)]
    #[case::two_fixes_on_opposite_meridians(&[(55.0, 0.0), (55.0, 180.0)], PoleWinding::None)]
    #[case::no_fixes(&[], PoleWinding::None)]
    fn a_track_winds_around_a_pole_only_once_its_closed_path_turns_full_circle(
        #[case] fixes: &[(f64, f64)],
        #[case] expected: PoleWinding,
    ) {
        assert_eq!(PoleWinding::of_track(positions_of(fixes)), expected);
    }

    #[rstest]
    #[case::north(PoleWinding::AroundNorthPole, 55.0, 90.0)]
    #[case::south(PoleWinding::AroundSouthPole, -90.0, 56.0)]
    fn the_cap_around_an_encircled_pole_reaches_from_the_extreme_fix_to_the_pole(
        #[case] winding: PoleWinding,
        #[case] expected_south: f64,
        #[case] expected_north: f64,
    ) {
        let bounds = GeoBounds::from_positions(positions_of(&[(55.0, 12.0), (56.0, 13.0)]))
            .expect("at least one position")
            .extended_to_the_encircled_pole(winding);

        assert!(bounds.lon.is_full_circle());
        assert_degrees_close(bounds.lat.south().as_degrees(), expected_south);
        assert_degrees_close(bounds.lat.north().as_degrees(), expected_north);
    }

    #[test]
    fn a_track_around_no_pole_keeps_the_box_over_its_fixes() {
        let bounds = GeoBounds::from_positions(positions_of(&[(55.0, 12.0), (56.0, 13.0)]))
            .expect("at least one position");

        assert_eq!(
            bounds.extended_to_the_encircled_pole(PoleWinding::None),
            bounds
        );
    }

    #[rstest]
    #[case::a_cap_around_the_north_pole(89.9, 90.0, 0.2)]
    #[case::a_cap_around_the_south_pole(-90.0, -89.9, 0.2)]
    #[case::a_band_north_of_the_equator(50.0, 60.0, 80.0)]
    #[case::a_band_over_the_equator(-10.0, 10.0, 180.0)]
    #[case::from_pole_to_pole(-90.0, 90.0, 180.0)]
    fn the_arc_across_the_pole_runs_from_the_parallel_nearest_the_equator(
        #[case] south: f64,
        #[case] north: f64,
        #[case] expected_degrees: f64,
    ) {
        let range = LatRange::from_latitudes([Latitude::new(south), Latitude::new(north)])
            .expect("two latitudes");

        assert_degrees_close(range.arc_across_the_pole_degrees(), expected_degrees);
    }

    #[test]
    fn a_single_position_bounds_that_position_alone() {
        let bounds = GeoBounds::single_position(Latitude::new(-33.9), Longitude::new(151.2));

        assert!(bounds.contains(Latitude::new(-33.9), Longitude::new(151.2)));
        assert!(!bounds.contains(Latitude::new(-33.9), Longitude::new(151.3)));
        assert_degrees_close(bounds.lon.span_degrees(), 0.0);
    }

    proptest::proptest! {
        /// A lap of evenly spaced fixes around a parallel encircles the pole
        /// of its hemisphere, whichever way round it was walked, however
        /// coarsely it was sampled and wherever it started.
        #[test]
        fn an_even_lap_of_a_parallel_encircles_the_pole_of_its_hemisphere(
            fix_count in 3_u32..24,
            start_degrees in -180.0_f64..180.0,
            latitude_degrees in -89.9_f64..=89.9,
            eastward in proptest::bool::ANY,
        ) {
            let spacing = FULL_CIRCLE_DEGREES / f64::from(fix_count);
            let direction = if eastward { 1.0 } else { -1.0 };
            let lap = (0..fix_count).map(|step| {
                let degrees = start_degrees + direction * spacing * f64::from(step);
                (
                    Latitude::new(latitude_degrees),
                    Longitude::new(mercator::wrap_longitude_degrees(degrees)),
                )
            });
            let expected = if latitude_degrees < 0.0 {
                PoleWinding::AroundSouthPole
            } else {
                PoleWinding::AroundNorthPole
            };

            proptest::prop_assert_eq!(PoleWinding::of_track(lap), expected);
        }

        #[test]
        fn a_grown_range_holds_every_longitude_and_spans_at_most_one_circle(
            degrees in proptest::collection::vec(-180.0_f64..=180.0_f64, 1..20),
        ) {
            let range = LonRange::from_longitudes(degrees.iter().map(|&d| Longitude::new(d)))
                .expect("at least one longitude");

            proptest::prop_assert!(range.span_degrees() >= 0.0);
            proptest::prop_assert!(range.span_degrees() <= FULL_CIRCLE_DEGREES);
            for &d in &degrees {
                proptest::prop_assert!(
                    range.contains(Longitude::new(d)),
                    "{d}° is outside {range:?}"
                );
            }
        }

        /// On the half-degree grid, where every sum and difference below is
        /// exact, a longitude on the union's own end is inside it.
        #[test]
        fn a_union_holds_every_longitude_of_both_ranges(
            left_half_degrees in proptest::collection::vec(-360_i32..=360, 1..10),
            right_half_degrees in proptest::collection::vec(-360_i32..=360, 1..10),
        ) {
            let left_degrees: Vec<f64> = left_half_degrees.iter().map(|&h| f64::from(h) / 2.0).collect();
            let right_degrees: Vec<f64> = right_half_degrees.iter().map(|&h| f64::from(h) / 2.0).collect();
            let left = LonRange::from_longitudes(left_degrees.iter().map(|&d| Longitude::new(d)))
                .expect("at least one longitude");
            let right = LonRange::from_longitudes(right_degrees.iter().map(|&d| Longitude::new(d)))
                .expect("at least one longitude");
            let union = left.union(right);

            proptest::prop_assert!(union.span_degrees() <= FULL_CIRCLE_DEGREES);
            proptest::prop_assert!(union.span_degrees() >= left.span_degrees());
            proptest::prop_assert!(union.span_degrees() >= right.span_degrees());
            for &d in left_degrees.iter().chain(&right_degrees) {
                proptest::prop_assert!(
                    union.contains(Longitude::new(d)),
                    "{d}° is outside {union:?}"
                );
            }
        }
    }
}

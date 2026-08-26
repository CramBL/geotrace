use crate::coordinates::{Latitude, Longitude};

const FULL_CIRCLE_DEGREES: f64 = 360.0;
const HALF_CIRCLE_DEGREES: f64 = 180.0;

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

    use super::{FULL_CIRCLE_DEGREES, GeoBounds, LonRange};
    use crate::coordinates::{Latitude, Longitude};

    /// 1e-9° is about 0.1 mm.
    const DEGREES_TOLERANCE: f64 = 1e-9;

    /// An eastbound track over the antimeridian, 1.5° wide.
    const ACROSS_THE_ANTIMERIDIAN: &[f64] = &[179.0, 179.5, -179.9, -179.5];

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

    #[test]
    fn a_single_position_bounds_that_position_alone() {
        let bounds = GeoBounds::single_position(Latitude::new(-33.9), Longitude::new(151.2));

        assert!(bounds.contains(Latitude::new(-33.9), Longitude::new(151.2)));
        assert!(!bounds.contains(Latitude::new(-33.9), Longitude::new(151.3)));
        assert_degrees_close(bounds.lon.span_degrees(), 0.0);
    }

    proptest::proptest! {
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

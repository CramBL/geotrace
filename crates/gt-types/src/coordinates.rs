use std::cmp::Ordering;
use std::fmt;

pub(crate) const FULL_CIRCLE_DEGREES: f64 = 360.0;
pub(crate) const HALF_CIRCLE_DEGREES: f64 = 180.0;
const QUARTER_CIRCLE_DEGREES: f64 = 90.0;

/// U+00B0 DEGREE SIGN, the unit a coordinate is written in.
const DEGREE_SIGN: &str = "°";

/// Marks a recorded coordinate the receiver wrote outside its axis' range.
const INVALID_MARKER: &str = "(invalid)";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoordinateAxis {
    Latitude,
    Longitude,
}

impl CoordinateAxis {
    fn limit_degrees(self) -> f64 {
        match self {
            Self::Latitude => QUARTER_CIRCLE_DEGREES,
            Self::Longitude => HALF_CIRCLE_DEGREES,
        }
    }
}

impl fmt::Display for CoordinateAxis {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Latitude => f.write_str("latitude"),
            Self::Longitude => f.write_str("longitude"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, thiserror::Error)]
#[error("{axis} {degrees} is outside {}..{}", -axis.limit_degrees(), axis.limit_degrees())]
pub struct OutOfRange {
    pub axis: CoordinateAxis,
    pub degrees: f64,
}

/// One axis of a position, holding degrees inside that axis' range by
/// construction.
pub trait Coordinate: Copy + Sized {
    const AXIS: CoordinateAxis;

    fn try_new(degrees: f64) -> Result<Self, OutOfRange>;

    fn as_degrees(self) -> f64;
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Latitude(f64);

impl Latitude {
    /// For a latitude the call site knows to be in range: a literal, a
    /// constant, or a value derived from other latitudes. Panics on anything
    /// else - read a latitude out of measured or parsed data with
    /// [`Latitude::try_new`].
    pub fn new(degrees: f64) -> Self {
        assert!(
            (-QUARTER_CIRCLE_DEGREES..=QUARTER_CIRCLE_DEGREES).contains(&degrees),
            "latitude out of range: {degrees}"
        );
        Self(degrees)
    }

    pub fn try_new(degrees: f64) -> Result<Self, OutOfRange> {
        <Self as Coordinate>::try_new(degrees)
    }

    pub fn as_degrees(self) -> f64 {
        self.0
    }
}

impl Coordinate for Latitude {
    const AXIS: CoordinateAxis = CoordinateAxis::Latitude;

    fn try_new(degrees: f64) -> Result<Self, OutOfRange> {
        if (-QUARTER_CIRCLE_DEGREES..=QUARTER_CIRCLE_DEGREES).contains(&degrees) {
            Ok(Self(degrees))
        } else {
            Err(OutOfRange {
                axis: Self::AXIS,
                degrees,
            })
        }
    }

    fn as_degrees(self) -> f64 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Longitude(f64);

impl Longitude {
    /// For a longitude the call site knows to be in range: a literal, a
    /// constant, or a value already wrapped into `[-180, 180]`. Panics on
    /// anything else - read a longitude out of measured or parsed data with
    /// [`Longitude::try_new`].
    pub fn new(degrees: f64) -> Self {
        assert!(
            (-HALF_CIRCLE_DEGREES..=HALF_CIRCLE_DEGREES).contains(&degrees),
            "longitude out of range: {degrees}"
        );
        Self(degrees)
    }

    pub fn try_new(degrees: f64) -> Result<Self, OutOfRange> {
        <Self as Coordinate>::try_new(degrees)
    }

    pub fn as_degrees(self) -> f64 {
        self.0
    }

    /// The shorter of the two arcs from `self` to `other`, in degrees,
    /// positive eastward and in `(-180, 180)`.
    ///
    /// `None` for the opposite meridian, which lies the same distance either
    /// way.
    pub fn shorter_arc_degrees_to(self, other: Self) -> Option<f64> {
        let eastward = (other.0 - self.0).rem_euclid(FULL_CIRCLE_DEGREES);
        match eastward.partial_cmp(&HALF_CIRCLE_DEGREES)? {
            Ordering::Less => Some(eastward),
            Ordering::Equal => None,
            Ordering::Greater => Some(eastward - FULL_CIRCLE_DEGREES),
        }
    }
}

impl Coordinate for Longitude {
    const AXIS: CoordinateAxis = CoordinateAxis::Longitude;

    fn try_new(degrees: f64) -> Result<Self, OutOfRange> {
        if (-HALF_CIRCLE_DEGREES..=HALF_CIRCLE_DEGREES).contains(&degrees) {
            Ok(Self(degrees))
        } else {
            Err(OutOfRange {
                axis: Self::AXIS,
                degrees,
            })
        }
    }

    fn as_degrees(self) -> f64 {
        self.0
    }
}

/// Degrees exactly as a receiver wrote them, on no particular axis and in no
/// particular range: 91.0, -181.0 and NaN all reach here.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RawDegrees(pub f64);

impl fmt::Display for RawDegrees {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}{DEGREE_SIGN}", self.0)
    }
}

/// One axis of a recorded fix's position, a [`Coordinate`] only when the
/// receiver wrote degrees inside that axis' range.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RecordedCoordinate<C> {
    Valid(C),
    Invalid(RawDegrees),
}

pub type RecordedLatitude = RecordedCoordinate<Latitude>;
pub type RecordedLongitude = RecordedCoordinate<Longitude>;

impl<C: Coordinate> RecordedCoordinate<C> {
    pub fn from_degrees(degrees: f64) -> Self {
        match C::try_new(degrees) {
            Ok(coordinate) => Self::Valid(coordinate),
            Err(_) => Self::Invalid(RawDegrees(degrees)),
        }
    }

    pub fn valid(self) -> Option<C> {
        match self {
            Self::Valid(coordinate) => Some(coordinate),
            Self::Invalid(_) => None,
        }
    }

    /// The degrees as recorded, valid or not, for display and for the plot.
    pub fn as_written(self) -> f64 {
        match self {
            Self::Valid(coordinate) => coordinate.as_degrees(),
            Self::Invalid(RawDegrees(degrees)) => degrees,
        }
    }
}

impl From<Latitude> for RecordedLatitude {
    fn from(latitude: Latitude) -> Self {
        Self::Valid(latitude)
    }
}

impl From<Longitude> for RecordedLongitude {
    fn from(longitude: Longitude) -> Self {
        Self::Valid(longitude)
    }
}

impl<C: Coordinate> fmt::Display for RecordedCoordinate<C> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Valid(coordinate) => write!(f, "{}", RawDegrees(coordinate.as_degrees())),
            Self::Invalid(degrees) => write!(f, "{degrees} {INVALID_MARKER}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::{
        CoordinateAxis, Latitude, Longitude, RawDegrees, RecordedLatitude, RecordedLongitude,
    };

    /// 1e-9° is about 0.1 mm.
    const ARC_TOLERANCE_DEGREES: f64 = 1e-9;

    #[rstest]
    #[case::the_south_pole(-90.0)]
    #[case::the_equator(0.0)]
    #[case::the_north_pole(90.0)]
    fn a_latitude_inside_its_range_reads_back_as_written(#[case] degrees: f64) {
        assert_eq!(
            Latitude::try_new(degrees).map(Latitude::as_degrees),
            Ok(degrees)
        );
    }

    #[rstest]
    #[case::the_antimeridian_west(-180.0)]
    #[case::the_prime_meridian(0.0)]
    #[case::the_antimeridian_east(180.0)]
    fn a_longitude_inside_its_range_reads_back_as_written(#[case] degrees: f64) {
        assert_eq!(
            Longitude::try_new(degrees).map(Longitude::as_degrees),
            Ok(degrees)
        );
    }

    #[rstest]
    #[case::just_past_the_north_pole(90.000_001, "latitude 90.000001 is outside -90..90")]
    #[case::far_past_the_south_pole(-181.0, "latitude -181 is outside -90..90")]
    #[case::not_a_number(f64::NAN, "latitude NaN is outside -90..90")]
    #[case::infinite(f64::INFINITY, "latitude inf is outside -90..90")]
    fn a_latitude_outside_its_range_names_its_axis_and_value(
        #[case] degrees: f64,
        #[case] expected: &str,
    ) {
        let error = Latitude::try_new(degrees).expect_err("out of range");

        assert_eq!(error.axis, CoordinateAxis::Latitude);
        assert_eq!(error.to_string(), expected);
    }

    #[rstest]
    #[case::just_past_the_antimeridian(180.000_001, "longitude 180.000001 is outside -180..180")]
    #[case::a_full_turn_west(-361.0, "longitude -361 is outside -180..180")]
    #[case::not_a_number(f64::NAN, "longitude NaN is outside -180..180")]
    #[case::infinite(f64::NEG_INFINITY, "longitude -inf is outside -180..180")]
    fn a_longitude_outside_its_range_names_its_axis_and_value(
        #[case] degrees: f64,
        #[case] expected: &str,
    ) {
        let error = Longitude::try_new(degrees).expect_err("out of range");

        assert_eq!(error.axis, CoordinateAxis::Longitude);
        assert_eq!(error.to_string(), expected);
    }

    #[rstest]
    #[case::at_the_pole(90.0, RecordedLatitude::Valid(Latitude::new(90.0)))]
    #[case::past_the_pole(91.0, RecordedLatitude::Invalid(RawDegrees(91.0)))]
    fn a_recorded_latitude_is_valid_only_inside_the_latitude_range(
        #[case] degrees: f64,
        #[case] expected: RecordedLatitude,
    ) {
        assert_eq!(RecordedLatitude::from_degrees(degrees), expected);
    }

    /// 91° is a longitude, where it is no latitude: each alias reads its own
    /// axis' range.
    #[test]
    fn a_recorded_longitude_is_valid_only_inside_the_longitude_range() {
        assert_eq!(
            RecordedLongitude::from_degrees(91.0),
            RecordedLongitude::Valid(Longitude::new(91.0))
        );
        assert_eq!(
            RecordedLongitude::from_degrees(-181.0),
            RecordedLongitude::Invalid(RawDegrees(-181.0))
        );
    }

    #[test]
    fn a_recorded_coordinate_written_as_nan_reads_back_as_nan() {
        assert!(
            RecordedLatitude::from_degrees(f64::NAN)
                .as_written()
                .is_nan()
        );
        assert!(
            RecordedLongitude::from_degrees(f64::NAN)
                .as_written()
                .is_nan()
        );
        assert_eq!(RecordedLatitude::from_degrees(f64::NAN).valid(), None);
        assert_eq!(RecordedLongitude::from_degrees(f64::NAN).valid(), None);
    }

    #[test]
    fn a_coordinate_converts_into_the_valid_case_of_its_axis() {
        assert_eq!(
            RecordedLatitude::from(Latitude::new(55.0)),
            RecordedLatitude::Valid(Latitude::new(55.0))
        );
        assert_eq!(
            RecordedLongitude::from(Longitude::new(12.0)),
            RecordedLongitude::Valid(Longitude::new(12.0))
        );
    }

    #[rstest]
    #[case::valid(RecordedLatitude::from_degrees(55.5), "55.5°")]
    #[case::invalid(RecordedLatitude::from_degrees(91.0), "91° (invalid)")]
    fn a_recorded_coordinate_displays_the_degrees_it_holds(
        #[case] latitude: RecordedLatitude,
        #[case] expected: &str,
    ) {
        assert_eq!(latitude.to_string(), expected);
    }

    #[rstest]
    #[case::eastward(10.0, 12.0, Some(2.0))]
    #[case::westward(12.0, 10.0, Some(-2.0))]
    #[case::eastward_across_the_antimeridian(179.0, -179.0, Some(2.0))]
    #[case::westward_across_the_antimeridian(-179.0, 179.0, Some(-2.0))]
    #[case::the_same_meridian(42.0, 42.0, Some(0.0))]
    #[case::just_short_of_the_opposite_meridian(0.0, 179.5, Some(179.5))]
    #[case::the_opposite_meridian(0.0, 180.0, None)]
    fn the_shorter_arc_is_signed_and_wraps_at_the_antimeridian(
        #[case] from_degrees: f64,
        #[case] to_degrees: f64,
        #[case] expected: Option<f64>,
    ) {
        let arc = Longitude::new(from_degrees).shorter_arc_degrees_to(Longitude::new(to_degrees));

        assert_eq!(arc, expected);
    }

    proptest::proptest! {
        /// Swapping the ends negates the arc, and the opposite meridian has
        /// none in either direction.
        #[test]
        fn the_shorter_arc_negates_when_its_ends_swap(
            from_degrees in -180.0_f64..=180.0,
            to_degrees in -180.0_f64..=180.0,
        ) {
            let from = Longitude::new(from_degrees);
            let to = Longitude::new(to_degrees);

            match (from.shorter_arc_degrees_to(to), to.shorter_arc_degrees_to(from)) {
                (Some(there), Some(back)) => proptest::prop_assert!(
                    (there + back).abs() < ARC_TOLERANCE_DEGREES,
                    "{there}° one way, {back}° the other"
                ),
                (None, None) => {}
                (there, back) => proptest::prop_assert!(
                    false,
                    "one direction has an arc and the other none: {there:?}, {back:?}"
                ),
            }
        }

        /// Every value inside an axis' range is a coordinate, and reads back
        /// bit for bit.
        #[test]
        fn a_coordinate_round_trips_through_its_range_check(
            latitude_degrees in -90.0_f64..=90.0,
            longitude_degrees in -180.0_f64..=180.0,
        ) {
            proptest::prop_assert_eq!(
                Latitude::try_new(latitude_degrees).map(Latitude::as_degrees),
                Ok(latitude_degrees)
            );
            proptest::prop_assert_eq!(
                Longitude::try_new(longitude_degrees).map(Longitude::as_degrees),
                Ok(longitude_degrees)
            );
        }
    }
}

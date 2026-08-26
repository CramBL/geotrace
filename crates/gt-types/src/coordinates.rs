use std::cmp::Ordering;

pub(crate) const FULL_CIRCLE_DEGREES: f64 = 360.0;
pub(crate) const HALF_CIRCLE_DEGREES: f64 = 180.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Latitude(f64);

impl Latitude {
    pub fn new(degrees: f64) -> Self {
        debug_assert!(
            (-90.0..=90.0).contains(&degrees),
            "latitude out of range: {degrees}"
        );
        Self(degrees)
    }

    pub fn as_degrees(self) -> f64 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Longitude(f64);

impl Longitude {
    pub fn new(degrees: f64) -> Self {
        debug_assert!(
            (-180.0..=180.0).contains(&degrees),
            "longitude out of range: {degrees}"
        );
        Self(degrees)
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

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::Longitude;

    /// 1e-9° is about 0.1 mm.
    const ARC_TOLERANCE_DEGREES: f64 = 1e-9;

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
    }
}

//! Where the Sun stands over a position at an instant, and which side of
//! Earth that puts a receiver on.

use chrono::{DateTime, Utc};

use crate::coordinates::{Latitude, Longitude};

/// Which side of Earth a position was on at an instant.
///
/// [`Sunlit`](Self::Sunlit) is the Sun's centre above the geometric horizon.
/// Refraction lifts the Sun by about half a degree at sunrise and sunset,
/// which is the width of the band the two sides are told apart across.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SunlitSide {
    Sunlit,
    Night,
}

impl SunlitSide {
    pub fn at_position(latitude: Latitude, longitude: Longitude, time: DateTime<Utc>) -> Self {
        if elevation_degrees(latitude, longitude, time) > 0.0 {
            Self::Sunlit
        } else {
            Self::Night
        }
    }
}

/// Unix seconds of the J2000.0 epoch, 2000-01-01T12:00:00Z, which the solar
/// coordinates are counted in days from.
const J2000_UNIX_SECONDS: f64 = 946_728_000.0;

const SECONDS_PER_DAY: f64 = 86_400.0;

const DEGREES_PER_TURN: f64 = 360.0;

/// The Sun's mean longitude, in degrees, at J2000.0 and per day after it.
const MEAN_LONGITUDE_AT_EPOCH: f64 = 280.460;
const MEAN_LONGITUDE_PER_DAY: f64 = 0.985_647_4;

/// The Sun's mean anomaly, in degrees, at J2000.0 and per day after it.
const MEAN_ANOMALY_AT_EPOCH: f64 = 357.528;
const MEAN_ANOMALY_PER_DAY: f64 = 0.985_600_3;

/// Coefficients, in degrees, of the equation of the centre, which corrects the
/// mean longitude to the true one.
const EQUATION_OF_CENTRE_FIRST: f64 = 1.915;
const EQUATION_OF_CENTRE_SECOND: f64 = 0.020;

/// Obliquity of the ecliptic, in degrees, at J2000.0 and per day after it.
const OBLIQUITY_AT_EPOCH: f64 = 23.439;
const OBLIQUITY_PER_DAY: f64 = -0.000_000_4;

/// Greenwich mean sidereal time, in degrees, at J2000.0 and per day after it.
/// A sidereal day is shorter than a solar one, which is the excess of the
/// daily rate over a full turn.
const SIDEREAL_TIME_AT_EPOCH: f64 = 280.460_618_37;
const SIDEREAL_TIME_PER_DAY: f64 = 360.985_647_366_29;

/// The Sun's elevation above the horizon, in degrees, over `latitude` and
/// `longitude` at `time`. It is negative wherever the Sun has set.
///
/// Computed from the low-precision solar coordinates of the Astronomical
/// Almanac as the US Naval Observatory publishes them
/// (<https://aa.usno.navy.mil/faq/sun_approx>), which place the Sun to about
/// a hundredth of a degree between 1950 and 2050. The elevation is geometric:
/// it is the direction to the Sun's centre, without the refraction that lifts
/// the Sun near the horizon.
pub fn elevation_degrees(latitude: Latitude, longitude: Longitude, time: DateTime<Utc>) -> f64 {
    let days = days_since_j2000(time);
    let mean_anomaly = (MEAN_ANOMALY_AT_EPOCH + MEAN_ANOMALY_PER_DAY * days).to_radians();
    let ecliptic_longitude = (MEAN_LONGITUDE_AT_EPOCH + MEAN_LONGITUDE_PER_DAY * days).to_radians()
        + EQUATION_OF_CENTRE_FIRST.to_radians() * mean_anomaly.sin()
        + EQUATION_OF_CENTRE_SECOND.to_radians() * (2.0 * mean_anomaly).sin();
    let obliquity = (OBLIQUITY_AT_EPOCH + OBLIQUITY_PER_DAY * days).to_radians();

    let declination = (obliquity.sin() * ecliptic_longitude.sin()).asin();
    let right_ascension_degrees = (obliquity.cos() * ecliptic_longitude.sin())
        .atan2(ecliptic_longitude.cos())
        .to_degrees();
    let hour_angle =
        (SIDEREAL_TIME_AT_EPOCH + SIDEREAL_TIME_PER_DAY * days + longitude.as_degrees()
            - right_ascension_degrees)
            .rem_euclid(DEGREES_PER_TURN)
            .to_radians();

    let latitude = latitude.as_degrees().to_radians();
    (latitude.sin() * declination.sin() + latitude.cos() * declination.cos() * hour_angle.cos())
        .asin()
        .to_degrees()
}

fn days_since_j2000(time: DateTime<Utc>) -> f64 {
    (time.timestamp() as f64 - J2000_UNIX_SECONDS) / SECONDS_PER_DAY
}

#[cfg(test)]
mod tests {
    use chrono::TimeDelta;
    use rstest::rstest;

    use crate::solar_position;

    use super::*;

    fn utc(year: i32, month: u32, day: u32, hour: u32) -> DateTime<Utc> {
        chrono::NaiveDate::from_ymd_opt(year, month, day)
            .and_then(|date| date.and_hms_opt(hour, 0, 0))
            .map(|naive| naive.and_utc())
            .expect("a calendar instant")
    }

    fn instant(rfc3339: &str) -> DateTime<Utc> {
        rfc3339.parse().expect("an RFC 3339 instant")
    }

    /// Elevations against the NOAA solar calculator, which solves the same
    /// geometry from the fuller Astronomical Almanac series. The two Greenwich
    /// solstice noons check themselves: solar noon on the reference meridian
    /// falls within a few minutes of 12:00 UTC, where the elevation is the
    /// solstice geometry alone.
    #[rstest]
    #[case::greenwich_june_solstice(51.4778, 0.0, "2024-06-21T12:00:00Z", 61.957)]
    #[case::greenwich_december_solstice(51.4778, 0.0, "2024-12-21T12:00:00Z", 15.083)]
    #[case::boulder_equinox_afternoon(40.0, -105.0, "2024-03-20T18:30:00Z", 49.360)]
    #[case::sydney_morning(-33.9, 151.2, "2023-09-05T03:15:00Z", 44.895)]
    #[case::svalbard_midnight_sun(78.22, 15.65, "2024-06-21T00:00:00Z", 12.042)]
    #[case::the_far_side_of_the_earth(0.0, 180.0, "2024-06-21T12:00:00Z", -66.558)]
    fn the_elevation_holds_to_the_published_solar_calculator(
        #[case] latitude: f64,
        #[case] longitude: f64,
        #[case] time: &str,
        #[case] expected_degrees: f64,
    ) {
        let elevation = solar_position::elevation_degrees(
            Latitude::new(latitude),
            Longitude::new(longitude),
            instant(time),
        );
        assert!(
            (elevation - expected_degrees).abs() < 0.05,
            "the Sun stood at {elevation} degrees, not {expected_degrees}"
        );
    }

    /// Each case is read at the hour whose usual result is the opposite:
    /// polar day at midnight, polar night at midday, and the far side of
    /// Earth at midday.
    #[rstest]
    #[case::midday(51.4778, 0.0, utc(2024, 6, 21, 12), SunlitSide::Sunlit)]
    #[case::midnight(51.4778, 0.0, utc(2024, 6, 21, 0), SunlitSide::Night)]
    #[case::the_far_side_at_midday(0.0, 180.0, utc(2024, 6, 21, 12), SunlitSide::Night)]
    #[case::svalbard_polar_day(78.22, 15.65, utc(2024, 6, 21, 0), SunlitSide::Sunlit)]
    #[case::svalbard_polar_night(78.22, 15.65, utc(2024, 12, 21, 12), SunlitSide::Night)]
    #[case::antarctic_polar_day(-77.85, 166.67, utc(2024, 12, 21, 12), SunlitSide::Sunlit)]
    #[case::antarctic_polar_night(-77.85, 166.67, utc(2024, 6, 21, 12), SunlitSide::Night)]
    fn a_position_is_read_on_the_side_the_sun_stands_over(
        #[case] latitude: f64,
        #[case] longitude: f64,
        #[case] time: DateTime<Utc>,
        #[case] expected: SunlitSide,
    ) {
        assert_eq!(
            SunlitSide::at_position(Latitude::new(latitude), Longitude::new(longitude), time),
            expected
        );
    }

    /// Second offsets from 2010-01-01, spanning twenty years.
    const SECONDS_OF_TWO_DECADES: std::ops::RangeInclusive<i64> = 0..=631_152_000;

    /// The antipode's solar elevation is as far below the horizon as the
    /// position's is above, wherever and whenever the two are read.
    #[test]
    fn the_antipode_sees_the_sun_as_far_below_the_horizon() {
        proptest::proptest!(|(
            latitude in -90.0_f64..=90.0,
            longitude in -180.0_f64..=0.0,
            seconds in SECONDS_OF_TWO_DECADES,
        )| {
            let time = utc(2010, 1, 1, 0) + TimeDelta::seconds(seconds);
            let here = solar_position::elevation_degrees(
                Latitude::new(latitude),
                Longitude::new(longitude),
                time,
            );
            let antipode = solar_position::elevation_degrees(
                Latitude::new(-latitude),
                Longitude::new(longitude + 180.0),
                time,
            );
            proptest::prop_assert!(
                (here + antipode).abs() < 1e-9,
                "{here} degrees here against {antipode} at the antipode"
            );
        });
    }
}

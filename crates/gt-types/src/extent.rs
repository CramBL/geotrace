use chrono::{DateTime, Utc};

use crate::geo_bounds::GeoBounds;
use crate::nav_point::ProjectedPosition;
use crate::track::{MercBounds, TimeRange};

/// One fix at the position the map draws it and the time the receiver stamped
/// it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DrawnFix {
    pub position: ProjectedPosition,
    pub time: DateTime<Utc>,
}

/// Where a contiguous run of drawn fixes lies on the map, and the span from
/// its earliest fix time to its latest.
///
/// [`Extent::spanning`] is the only constructor, and it reads the position and
/// the time of every fix of the run. The first and the last fix do not give
/// the span: a track's timestamps can step backwards. The earliest or the
/// latest fix of a run can be one in the middle of it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Extent {
    merc: MercBounds,
    time: TimeRange,
}

impl Extent {
    pub fn spanning(first: DrawnFix, rest: impl IntoIterator<Item = DrawnFix> + Clone) -> Self {
        let bounds = GeoBounds::from_first_position_and_rest(
            first.position.coordinates(),
            rest.clone()
                .into_iter()
                .map(|fix| fix.position.coordinates()),
        );
        Self {
            merc: MercBounds::from(bounds),
            time: TimeRange::spanning(first.time, rest.into_iter().map(|fix| fix.time)),
        }
    }

    /// The extent covering both runs, closing whatever gap in space and in
    /// time lies between them.
    pub fn union(self, other: Self) -> Self {
        Self {
            merc: self.merc.union(other.merc),
            time: self.time.union(other.time),
        }
    }

    pub fn merc(self) -> MercBounds {
        self.merc
    }

    pub fn time(self) -> TimeRange {
        self.time
    }
}

#[cfg(test)]
mod tests {
    use chrono::{DateTime, TimeDelta, Utc};

    use super::{DrawnFix, Extent};
    use crate::coordinates::{Latitude, Longitude};
    use crate::nav_point::ProjectedPosition;
    use crate::track::TimeRange;

    fn fix_at(longitude_degrees: f64, offset_secs: i64) -> DrawnFix {
        DrawnFix {
            position: ProjectedPosition::new(
                Latitude::new(55.0),
                Longitude::new(longitude_degrees),
            ),
            time: DateTime::<Utc>::UNIX_EPOCH + TimeDelta::seconds(offset_secs),
        }
    }

    #[test]
    fn spanning_a_run_with_a_backward_time_step_gives_its_earliest_and_latest_fix_time() {
        let run = [fix_at(12.0, 10), fix_at(12.1, 40), fix_at(12.2, 4)];
        let (first, rest) = run.split_first().expect("three fixes");

        let extent = Extent::spanning(*first, rest.iter().copied());

        assert_eq!(
            extent.time(),
            TimeRange::new(
                DateTime::<Utc>::UNIX_EPOCH + TimeDelta::seconds(4),
                DateTime::<Utc>::UNIX_EPOCH + TimeDelta::seconds(40),
            )
        );
    }
}

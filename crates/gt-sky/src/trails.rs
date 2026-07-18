//! Whole-track sky trails: each satellite's path across the sky over a
//! track, extracted once from the recorded points for the trails window.
//!
//! Pure data, no rendering - the window projects and draws these, sharing
//! the same [`crate::projection`] the per-report plot uses.

use std::collections::BTreeMap;

use gt_types::satellites::{Constellation, Prn, Snr};
use gt_types::{GpsTime, GpsTimeRange, LoadedTrack, PointIdx};

use crate::projection;

/// One satellite's position at one report epoch, a vertex of a [`SkyTrail`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TrailSample {
    pub time: GpsTime,
    /// The track point this sample came from, so the scrubber can
    /// cross-highlight the map through the existing highlight channel.
    pub point_index: PointIdx,
    pub azimuth: f32,
    pub elevation: f32,
    pub snr: Option<Snr>,
    pub in_fix: bool,
}

/// One satellite's path across the sky over a track: the polyline through
/// its azimuth/elevation at every report epoch where it had a sky position.
/// Epochs where the satellite lacked azimuth or elevation are simply gaps.
#[derive(Debug, Clone, PartialEq)]
pub struct SkyTrail {
    pub constellation: Constellation,
    pub prn: Prn,
    /// Ascending by time.
    pub samples: Vec<TrailSample>,
}

/// One report epoch on the track's timeline - the scrubber walks these.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrailEpoch {
    pub time: GpsTime,
    pub point_index: PointIdx,
}

/// Every satellite's trail over a track, plus the report-epoch timeline and
/// overall time span the window needs to normalize the time-ramp and drive
/// the scrubber.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SkyTrails {
    /// Sorted by constellation then PRN, so the legend and draw order are
    /// stable.
    pub trails: Vec<SkyTrail>,
    /// Report epochs, ascending by time.
    pub epochs: Vec<TrailEpoch>,
    /// First and last epoch times, or `None` when the track has no reports.
    pub time_range: Option<GpsTimeRange>,
}

impl SkyTrails {
    /// The distinct constellations present, in `Constellation` order. Relies
    /// on `trails` being sorted by constellation (as [`extract_trails`]
    /// produces), so the invariant and its consumer live together.
    pub fn constellations(&self) -> impl Iterator<Item = Constellation> + '_ {
        let mut last = None;
        self.trails.iter().filter_map(move |trail| {
            let constellation = trail.constellation;
            if last == Some(constellation) {
                return None;
            }
            last = Some(constellation);
            Some(constellation)
        })
    }
}

/// Extract every satellite's sky trail from a track.
///
/// Walks the report-bearing points in recording order, recording an epoch
/// per report and a sample per satellite that has a sky position at that
/// epoch. The ascending-by-time ordering of the trails and epochs relies on
/// `track.points` being in recording order, the same assumption
/// [`gt_types::LoadedTrack::nearest_satellite_report`] makes.
pub fn extract_trails(track: &LoadedTrack) -> SkyTrails {
    let mut by_satellite: BTreeMap<(Constellation, Prn), Vec<TrailSample>> = BTreeMap::new();
    let mut epochs: Vec<TrailEpoch> = Vec::new();

    for (index, point) in track.points.iter().enumerate() {
        let Some(satellites) = &point.satellites else {
            continue;
        };
        let point_index = PointIdx::new(index);
        let time = point.tpv.time();
        epochs.push(TrailEpoch { time, point_index });

        for satellite in satellites.satellites() {
            let Some((azimuth, elevation)) = projection::sky_position(satellite) else {
                continue;
            };
            by_satellite
                .entry((satellite.constellation(), satellite.prn()))
                .or_default()
                .push(TrailSample {
                    time,
                    point_index,
                    azimuth,
                    elevation,
                    snr: satellite.snr(),
                    in_fix: satellite.in_fix(),
                });
        }
    }

    let time_range = epochs
        .first()
        .zip(epochs.last())
        .map(|(first, last)| GpsTimeRange::new(first.time, last.time));
    let trails = by_satellite
        .into_iter()
        .map(|((constellation, prn), samples)| SkyTrail {
            constellation,
            prn,
            samples,
        })
        .collect();

    SkyTrails {
        trails,
        epochs,
        time_range,
    }
}

#[cfg(test)]
mod tests {
    use chrono::{DateTime, Duration, Utc};

    use gt_types::satellites::{Constellation, Satellite, Satellites};
    use gt_types::{GpsTime, Latitude, Longitude, NavPoint, TimePositionVelocity};

    use super::{PointIdx, extract_trails};

    fn sat(
        constellation: Constellation,
        prn: u32,
        azimuth: Option<f32>,
        elevation: Option<f32>,
        in_fix: bool,
    ) -> Satellite {
        Satellite::new(constellation, prn, elevation, azimuth, Some(40.0), in_fix)
    }

    /// A point `secs` after a fixed epoch, carrying the given satellites (or
    /// no report when `None`).
    fn point_at(secs: i64, satellites: Option<Vec<Satellite>>) -> NavPoint {
        let start = DateTime::<Utc>::from_timestamp(1_748_000_000, 0).expect("valid");
        let tpv = TimePositionVelocity::builder()
            .time(GpsTime::from_utc(start + Duration::seconds(secs)))
            .lat(Latitude::new(55.0))
            .lon(Longitude::new(12.0))
            .build();
        NavPoint::new(tpv, satellites.map(|s| Satellites::new(None, None, s)))
    }

    fn track(points: Vec<NavPoint>) -> gt_types::LoadedTrack {
        gt_test_utils::loaded_track_with_points(points)
    }

    #[test]
    fn groups_by_satellite_sorted_with_gaps() {
        // Reports at t0 and t2 (t1 has none). GPS G05 has a sky position at
        // both; Galileo E03 only at t0, so its trail has one sample.
        let track = track(vec![
            point_at(
                0,
                Some(vec![
                    sat(Constellation::Gps, 5, Some(45.0), Some(60.0), true),
                    sat(Constellation::Galileo, 3, Some(80.0), Some(40.0), true),
                ]),
            ),
            point_at(1, None),
            point_at(
                2,
                Some(vec![sat(
                    Constellation::Gps,
                    5,
                    Some(50.0),
                    Some(58.0),
                    true,
                )]),
            ),
        ]);

        let trails = extract_trails(&track);

        // Two report epochs, at points 0 and 2.
        assert_eq!(trails.epochs.len(), 2);
        assert_eq!(trails.epochs[0].point_index, PointIdx::new(0));
        assert_eq!(trails.epochs[1].point_index, PointIdx::new(2));
        assert_eq!(
            trails.time_range,
            Some(gt_types::GpsTimeRange::new(
                trails.epochs[0].time,
                trails.epochs[1].time
            ))
        );

        // Sorted by constellation: GPS before Galileo.
        assert_eq!(trails.trails.len(), 2);
        let gps = &trails.trails[0];
        assert_eq!(gps.constellation, Constellation::Gps);
        assert_eq!(gps.prn.value(), 5);
        assert_eq!(gps.samples.len(), 2);
        assert!(gps.samples[0].time < gps.samples[1].time);
        assert_eq!(gps.samples[1].point_index, PointIdx::new(2));

        let galileo = &trails.trails[1];
        assert_eq!(galileo.constellation, Constellation::Galileo);
        assert_eq!(galileo.samples.len(), 1);
    }

    #[test]
    #[expect(
        clippy::float_cmp,
        reason = "az/el/snr pass through unchanged, so the values are bit-exact"
    )]
    fn a_sample_carries_the_satellites_values() {
        let track = track(vec![point_at(
            0,
            Some(vec![sat(
                Constellation::Gps,
                5,
                Some(45.0),
                Some(60.0),
                true,
            )]),
        )]);
        let sample = extract_trails(&track).trails[0].samples[0];
        assert_eq!(sample.point_index, PointIdx::new(0));
        assert_eq!(sample.azimuth, 45.0);
        assert_eq!(sample.elevation, 60.0);
        assert_eq!(sample.snr.map(gt_types::satellites::Snr::value), Some(40.0));
        assert!(sample.in_fix);
    }

    #[test]
    fn constellations_lists_each_present_one_once_in_order() {
        // Two GPS satellites and one Galileo, interleaved across epochs;
        // `constellations()` collapses to one entry each, GPS before Galileo.
        let track = track(vec![
            point_at(
                0,
                Some(vec![
                    sat(Constellation::Gps, 5, Some(45.0), Some(60.0), true),
                    sat(Constellation::Galileo, 3, Some(80.0), Some(40.0), true),
                ]),
            ),
            point_at(
                1,
                Some(vec![sat(
                    Constellation::Gps,
                    12,
                    Some(30.0),
                    Some(50.0),
                    true,
                )]),
            ),
        ]);
        let present: Vec<_> = extract_trails(&track).constellations().collect();
        assert_eq!(present, vec![Constellation::Gps, Constellation::Galileo]);
    }

    #[test]
    fn same_prn_in_two_constellations_stays_separate() {
        // GPS and Galileo both have a PRN 5: the compound (constellation, prn)
        // key must keep them in distinct trails.
        let track = track(vec![point_at(
            0,
            Some(vec![
                sat(Constellation::Gps, 5, Some(45.0), Some(60.0), true),
                sat(Constellation::Galileo, 5, Some(80.0), Some(40.0), true),
            ]),
        )]);
        let trails = extract_trails(&track);
        assert_eq!(trails.trails.len(), 2);
        assert_eq!(trails.trails[0].constellation, Constellation::Gps);
        assert_eq!(trails.trails[1].constellation, Constellation::Galileo);
        assert!(trails.trails.iter().all(|t| t.prn.value() == 5));
    }

    #[test]
    fn unplaceable_samples_are_skipped() {
        // Azimuth but no elevation -> not placeable -> no sample, no trail.
        let track = track(vec![point_at(
            0,
            Some(vec![sat(Constellation::Gps, 5, Some(45.0), None, true)]),
        )]);
        let trails = extract_trails(&track);
        assert!(trails.trails.is_empty());
        // The epoch still exists - the report was there, just unplaceable.
        assert_eq!(trails.epochs.len(), 1);
    }

    #[test]
    fn a_track_without_reports_has_no_trails() {
        let trails = extract_trails(&track(vec![point_at(0, None), point_at(1, None)]));
        assert!(trails.trails.is_empty());
        assert!(trails.epochs.is_empty());
        assert_eq!(trails.time_range, None);
    }
}

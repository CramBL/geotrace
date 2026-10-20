//! Whole-track sky trails: each satellite's path across the sky over a
//! track, extracted once from the recorded points for the trails window.
//!
//! Pure data, no rendering - the window projects and draws these, sharing
//! the same [`crate::projection`] the per-report plot uses.

use std::collections::BTreeMap;

use gt_types::satellites::{Constellation, Prn, SlipCause, Snr};
use gt_types::{
    GeneratedMarkerKind, GpsTime, GpsTimeRange, LoadedTrack, PointIdx, SatelliteMembership,
    SatelliteQueryScope, SatelliteReportMetadata,
};
use smallvec::SmallVec;
use strum::{EnumCount as _, IntoEnumIterator as _};

/// The report index and preceding report index for this constellation.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct EpochIdx {
    index: usize,
    previous: Option<usize>,
}

impl EpochIdx {
    pub fn new(n: usize) -> Self {
        Self {
            index: n,
            previous: n.checked_sub(1),
        }
    }

    pub fn follows(self, earlier: Self) -> bool {
        self.previous == Some(earlier.index)
    }
}

/// One satellite's position at one report epoch, a vertex of a [`SkyTrail`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TrailSample {
    pub time: GpsTime,
    /// The report this sample came from.
    pub epoch: EpochIdx,
    /// The track point this sample came from, so the scrubber can
    /// cross-highlight the map through the existing highlight channel.
    pub point_index: PointIdx,
    pub azimuth: f32,
    pub elevation: f32,
    pub snr: Option<Snr>,
    pub membership: SatelliteMembership,
    pub usage_report_index: Option<usize>,
}

impl TrailSample {
    pub fn in_fix(&self) -> bool {
        self.membership == SatelliteMembership::Used
    }
}

/// One satellite's path across the sky over a track: the polyline through
/// its azimuth/elevation at every report epoch where it had a sky position.
/// Epochs where the satellite lacked azimuth or elevation are simply gaps.
#[derive(Debug, Clone, PartialEq)]
pub struct SkyTrail {
    pub constellation: Constellation,
    pub prn: Prn,
    /// Recorded report order.
    pub samples: Vec<TrailSample>,
}

impl SkyTrail {
    pub fn marker_at(&self, time: GpsTime) -> Option<TrailMarker<'_>> {
        if let Some(sample) = self.samples.iter().find(|sample| sample.time == time) {
            return Some(TrailMarker {
                sample,
                azimuth: sample.azimuth,
                elevation: sample.elevation,
            });
        }
        let [earlier, later] = self.samples.windows(2).find(
            |pair| matches!(pair, [a, b] if a.time < time && time < b.time && b.epoch.follows(a.epoch)),
        )? else {
            return None;
        };
        let fraction = time.signed_duration_since(earlier.time).as_seconds_f64()
            / later
                .time
                .signed_duration_since(earlier.time)
                .as_seconds_f64();
        Some(TrailMarker {
            sample: earlier,
            azimuth: earlier.azimuth + (later.azimuth - earlier.azimuth) * fraction as f32,
            elevation: earlier.elevation + (later.elevation - earlier.elevation) * fraction as f32,
        })
    }

    pub fn ever_in_fix(&self) -> bool {
        self.samples.iter().any(TrailSample::in_fix)
    }

    pub fn has_used_or_unknown_membership(&self) -> bool {
        self.samples
            .iter()
            .any(|sample| sample.membership != SatelliteMembership::Unused)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TrailMarker<'a> {
    /// Membership and time refer to this recorded sample, including during interpolation.
    pub sample: &'a TrailSample,
    pub azimuth: f32,
    pub elevation: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct TrailMembershipFilter {
    pub show_not_in_fix: bool,
    pub in_fix_only: bool,
}

impl TrailMembershipFilter {
    pub fn allows(self, trail: &SkyTrail, marker: TrailMarker<'_>) -> bool {
        (self.show_not_in_fix || trail.has_used_or_unknown_membership())
            && (!self.in_fix_only || marker.sample.in_fix())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TrailDisplayCount {
    pub constellation: Constellation,
    pub shown: usize,
    pub before_membership_filters: usize,
    /// Unavailable when any displayed satellite has unknown membership.
    pub used: Option<usize>,
}

/// One report epoch on the track's timeline - the scrubber walks these.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrailEpoch {
    pub time: GpsTime,
    pub point_index: PointIdx,
    pub observation_index: usize,
    pub metadata: SatelliteReportMetadata,
}

/// A cycle slip placed on the sky (lost lock, or a sharp SNR drop). Positioned
/// at its last-known sky position before the slip, the `from` side of the
/// detected transition. `from` is used for both causes: `to` is unavailable for
/// a lost-lock slip, and the from/to drift for an SNR drop is one report epoch,
/// negligible at plot scale.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SlipMark {
    pub constellation: Constellation,
    pub prn: Prn,
    pub azimuth: f32,
    pub elevation: f32,
    pub cause: SlipCause,
}

/// Every satellite's trail over a track, plus the report-epoch timeline and
/// overall time span the window needs to normalize the time-ramp and drive
/// the scrubber.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SkyTrails {
    /// Sorted by constellation then PRN, so the legend and draw order are
    /// stable.
    pub trails: Vec<SkyTrail>,
    /// Observation reports in recorded order.
    pub epochs: Vec<TrailEpoch>,
    /// Earliest and latest observation times.
    pub time_range: Option<GpsTimeRange>,
    /// Cycle slips detected over the track, placed on the sky. Empty when the
    /// track has no slip markers.
    pub slips: Vec<SlipMark>,
}

impl SkyTrails {
    /// The distinct constellations present, in `Constellation` order. Relies
    /// on `trails` being sorted by constellation (as [`extract_trails`]
    /// produces).
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

    pub fn display_counts_at(
        &self,
        time: GpsTime,
        filter: TrailMembershipFilter,
    ) -> SmallVec<[TrailDisplayCount; Constellation::COUNT]> {
        self.constellations()
            .map(|constellation| {
                let mut count = TrailDisplayCount {
                    constellation,
                    shown: 0,
                    before_membership_filters: 0,
                    used: Some(0),
                };
                for trail in self
                    .trails
                    .iter()
                    .filter(|trail| trail.constellation == constellation)
                {
                    let Some(marker) = trail.marker_at(time) else {
                        continue;
                    };
                    count.before_membership_filters += 1;
                    if filter.allows(trail, marker) {
                        count.shown += 1;
                        count.used = count
                            .used
                            .filter(|_| marker.sample.membership != SatelliteMembership::Unknown)
                            .map(|used| used + usize::from(marker.sample.in_fix()));
                    }
                }
                count
            })
            .collect()
    }
}

/// Extract every satellite's sky trail from a track.
///
pub fn extract_trails(track: &LoadedTrack) -> SkyTrails {
    let mut by_satellite: BTreeMap<(Constellation, Prn), Vec<TrailSample>> = BTreeMap::new();
    let mut epochs: Vec<TrailEpoch> = Vec::new();

    let mut previous_by_constellation = BTreeMap::new();
    let mut previous_time = None;
    let mut previous_fix_time = None;
    for (index, point) in track.points.iter().enumerate() {
        if previous_fix_time.is_some_and(|time| point.tpv.time() < time) {
            previous_by_constellation.clear();
            previous_time = None;
        }
        previous_fix_time = Some(point.tpv.time());
        for (observation_index, observation) in
            point.satellite_evidence.observations.iter().enumerate()
        {
            let time = observation.metadata.normalized_time;
            if previous_time.is_some_and(|previous| time < previous) {
                previous_by_constellation.clear();
            }
            previous_time = Some(time);
            let point_index = PointIdx::new(index);
            let epoch_index = epochs.len();
            epochs.push(TrailEpoch {
                time,
                point_index,
                observation_index,
                metadata: observation.metadata,
            });
            for satellite in &observation.satellites {
                let (Some(azimuth), Some(elevation)) = (
                    satellite.measurements.azimuth,
                    satellite.measurements.elevation,
                ) else {
                    continue;
                };
                let resolved = point.resolved_satellite_membership(satellite.identity);
                let constellation = satellite.identity.constellation;
                let epoch = EpochIdx {
                    index: epoch_index,
                    previous: previous_by_constellation.get(&constellation).copied(),
                };
                by_satellite
                    .entry((constellation, satellite.identity.prn))
                    .or_default()
                    .push(TrailSample {
                        time,
                        epoch,
                        point_index,
                        azimuth,
                        elevation,
                        snr: satellite.measurements.snr,
                        membership: resolved.membership,
                        usage_report_index: resolved.usage_report_index,
                    });
            }
            for constellation in Constellation::iter().filter(|constellation| {
                observation
                    .metadata
                    .scope
                    .covers(SatelliteQueryScope::Constellation(*constellation))
            }) {
                previous_by_constellation.insert(constellation, epoch_index);
            }
        }
    }

    let time_range = epochs
        .iter()
        .map(|epoch| epoch.time)
        .min()
        .zip(epochs.iter().map(|epoch| epoch.time).max())
        .map(|(first, last)| GpsTimeRange::new(first, last));
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
        slips: extract_slips(track),
    }
}

/// Collect the cycle-slip marks from a track's generated markers, each placed
/// at the slipped satellite's last-known sky position (the `from` side).
/// Slips whose `from` sample lacks an azimuth or elevation are skipped.
fn extract_slips(track: &LoadedTrack) -> Vec<SlipMark> {
    track
        .generated_markers
        .iter()
        .filter_map(|marker| match &marker.kind {
            GeneratedMarkerKind::Slip(event) => Some(&event.slips),
            _ => None,
        })
        .flatten()
        .filter_map(|slip| {
            Some(SlipMark {
                constellation: slip.constellation,
                prn: slip.prn,
                azimuth: slip.from.azimuth?,
                elevation: slip.from.elevation?,
                cause: slip.cause,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use gt_types::satellites::Constellation;

    use super::{PointIdx, TrailMembershipFilter};
    use crate::test_util::{self, Azimuth, Elevation};

    #[test]
    fn display_counts_separate_used_and_shown_per_constellation() {
        let trails = super::extract_trails(&gt_test_utils::loaded_track_with_points(vec![
            test_util::nav_point_reporting(
                0,
                Some(vec![
                    test_util::sat(
                        Constellation::Gps,
                        5,
                        Some(Azimuth(40.0)),
                        Some(Elevation(45.0)),
                        true,
                    ),
                    test_util::sat(
                        Constellation::Gps,
                        12,
                        Some(Azimuth(120.0)),
                        Some(Elevation(30.0)),
                        false,
                    ),
                    test_util::sat(
                        Constellation::Galileo,
                        3,
                        Some(Azimuth(60.0)),
                        Some(Elevation(50.0)),
                        true,
                    ),
                ]),
            ),
        ]));

        let counts = trails.display_counts_at(
            trails.epochs[0].time,
            TrailMembershipFilter {
                show_not_in_fix: true,
                in_fix_only: false,
            },
        );
        let gps = counts
            .iter()
            .find(|c| c.constellation == Constellation::Gps)
            .expect("gps present");
        assert_eq!((gps.shown, gps.used), (2, Some(1)));
        let galileo = counts
            .iter()
            .find(|c| c.constellation == Constellation::Galileo)
            .expect("galileo present");
        assert_eq!((galileo.shown, galileo.used), (1, Some(1)));

        assert_eq!(gps.before_membership_filters, 2);

        let in_fix_only = trails.display_counts_at(
            trails.epochs[0].time,
            TrailMembershipFilter {
                show_not_in_fix: false,
                in_fix_only: false,
            },
        );
        let gps = in_fix_only
            .iter()
            .find(|c| c.constellation == Constellation::Gps)
            .expect("gps present");
        assert_eq!((gps.shown, gps.used), (1, Some(1)));
        assert_eq!(gps.before_membership_filters, 2);
    }

    #[test]
    fn ever_in_fix_reflects_any_fix_over_the_track() {
        let trails = super::extract_trails(&gt_test_utils::loaded_track_with_points(vec![
            test_util::nav_point_reporting(
                0,
                Some(vec![test_util::sat(
                    Constellation::Gps,
                    5,
                    Some(Azimuth(40.0)),
                    Some(Elevation(45.0)),
                    false,
                )]),
            ),
            test_util::nav_point_reporting(
                1,
                Some(vec![test_util::sat(
                    Constellation::Gps,
                    5,
                    Some(Azimuth(50.0)),
                    Some(Elevation(40.0)),
                    true,
                )]),
            ),
        ]));
        assert!(trails.trails[0].ever_in_fix());

        let never = super::extract_trails(&gt_test_utils::loaded_track_with_points(vec![
            test_util::nav_point_reporting(
                0,
                Some(vec![test_util::sat(
                    Constellation::Gps,
                    5,
                    Some(Azimuth(40.0)),
                    Some(Elevation(45.0)),
                    false,
                )]),
            ),
        ]));
        assert!(!never.trails[0].ever_in_fix());
    }

    #[test]
    fn display_counts_include_interpolated_markers() {
        let trails = super::extract_trails(&gt_test_utils::loaded_track_with_points(vec![
            test_util::nav_point_reporting(
                0,
                Some(vec![test_util::sat(
                    Constellation::Gps,
                    5,
                    Some(Azimuth(40.0)),
                    Some(Elevation(45.0)),
                    true,
                )]),
            ),
            test_util::nav_point_reporting(
                2,
                Some(vec![test_util::sat(
                    Constellation::Gps,
                    5,
                    Some(Azimuth(50.0)),
                    Some(Elevation(40.0)),
                    true,
                )]),
            ),
        ]));
        let between = test_util::at(1);
        let counts = trails.display_counts_at(
            between,
            TrailMembershipFilter {
                show_not_in_fix: true,
                in_fix_only: false,
            },
        );
        assert!(counts.iter().all(|c| c.shown == 1 && c.used == Some(1)));
    }

    #[test]
    #[expect(
        clippy::float_cmp,
        reason = "az/el pass through unchanged, so the values are bit-exact"
    )]
    fn slips_are_placed_at_their_from_position() {
        use gt_types::satellites::{SatSample, Slip, SlipCause, SlipEvent, Snr};
        use gt_types::{GeneratedMarker, GeneratedMarkerKind, Latitude, Longitude, mercator};

        let placeable = Slip {
            constellation: Constellation::Gps,
            prn: gt_types::satellites::Prn::new(5),
            cause: SlipCause::LostLock,
            from: SatSample {
                elevation: Some(30.0),
                azimuth: Some(120.0),
                snr: Some(Snr::new(38.0)),
            },
            to: None,
        };
        // A second slip whose `from` lacks a sky position: skipped.
        let unplaceable = Slip {
            cause: SlipCause::SnrDrop,
            from: SatSample {
                elevation: None,
                azimuth: Some(90.0),
                snr: None,
            },
            to: None,
            ..placeable
        };
        let lat = Latitude::new(55.0);
        let lon = Longitude::new(12.0);
        let mut track =
            gt_test_utils::loaded_track_with_points(vec![test_util::nav_point_reporting(0, None)]);
        track.generated_markers = vec![GeneratedMarker {
            time: test_util::start(),
            kind: GeneratedMarkerKind::Slip(SlipEvent {
                slips: vec![placeable, unplaceable],
            }),
            lat,
            lon,
            merc: mercator::normalize(lat, lon),
        }];

        let slips = super::extract_trails(&track).slips;
        assert_eq!(slips.len(), 1);
        assert_eq!(slips[0].constellation, Constellation::Gps);
        assert_eq!(slips[0].azimuth, 120.0);
        assert_eq!(slips[0].elevation, 30.0);
        assert_eq!(slips[0].cause, SlipCause::LostLock);
    }

    #[test]
    fn groups_by_satellite_sorted_with_gaps() {
        // Reports at t0 and t2 (t1 has none). GPS G05 has a sky position at
        // both. Galileo E03 has one only at t0, so its trail has one sample.
        let track = gt_test_utils::loaded_track_with_points(vec![
            test_util::nav_point_reporting(
                0,
                Some(vec![
                    test_util::sat(
                        Constellation::Gps,
                        5,
                        Some(Azimuth(45.0)),
                        Some(Elevation(60.0)),
                        true,
                    ),
                    test_util::sat(
                        Constellation::Galileo,
                        3,
                        Some(Azimuth(80.0)),
                        Some(Elevation(40.0)),
                        true,
                    ),
                ]),
            ),
            test_util::nav_point_reporting(1, None),
            test_util::nav_point_reporting(
                2,
                Some(vec![test_util::sat(
                    Constellation::Gps,
                    5,
                    Some(Azimuth(50.0)),
                    Some(Elevation(58.0)),
                    true,
                )]),
            ),
        ]);

        let trails = super::extract_trails(&track);

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
        let track = gt_test_utils::loaded_track_with_points(vec![test_util::nav_point_reporting(
            0,
            Some(vec![test_util::sat(
                Constellation::Gps,
                5,
                Some(Azimuth(45.0)),
                Some(Elevation(60.0)),
                true,
            )]),
        )]);
        let sample = super::extract_trails(&track).trails[0].samples[0];
        assert_eq!(sample.point_index, PointIdx::new(0));
        assert_eq!(sample.azimuth, 45.0);
        assert_eq!(sample.elevation, 60.0);
        assert_eq!(sample.snr.map(gt_types::satellites::Snr::value), Some(40.0));
        assert!(sample.in_fix());
    }

    #[test]
    fn constellations_lists_each_present_one_once_in_order() {
        // Two GPS satellites and one Galileo, interleaved across epochs.
        // `constellations()` collapses to one entry each, GPS before Galileo.
        let track = gt_test_utils::loaded_track_with_points(vec![
            test_util::nav_point_reporting(
                0,
                Some(vec![
                    test_util::sat(
                        Constellation::Gps,
                        5,
                        Some(Azimuth(45.0)),
                        Some(Elevation(60.0)),
                        true,
                    ),
                    test_util::sat(
                        Constellation::Galileo,
                        3,
                        Some(Azimuth(80.0)),
                        Some(Elevation(40.0)),
                        true,
                    ),
                ]),
            ),
            test_util::nav_point_reporting(
                1,
                Some(vec![test_util::sat(
                    Constellation::Gps,
                    12,
                    Some(Azimuth(30.0)),
                    Some(Elevation(50.0)),
                    true,
                )]),
            ),
        ]);
        let present: Vec<_> = super::extract_trails(&track).constellations().collect();
        assert_eq!(present, vec![Constellation::Gps, Constellation::Galileo]);
    }

    #[test]
    fn same_prn_in_two_constellations_stays_separate() {
        // GPS and Galileo both have a PRN 5: the compound `(constellation, prn)`
        // key must keep them in distinct trails.
        let track = gt_test_utils::loaded_track_with_points(vec![test_util::nav_point_reporting(
            0,
            Some(vec![
                test_util::sat(
                    Constellation::Gps,
                    5,
                    Some(Azimuth(45.0)),
                    Some(Elevation(60.0)),
                    true,
                ),
                test_util::sat(
                    Constellation::Galileo,
                    5,
                    Some(Azimuth(80.0)),
                    Some(Elevation(40.0)),
                    true,
                ),
            ]),
        )]);
        let trails = super::extract_trails(&track);
        assert_eq!(trails.trails.len(), 2);
        assert_eq!(trails.trails[0].constellation, Constellation::Gps);
        assert_eq!(trails.trails[1].constellation, Constellation::Galileo);
        assert!(trails.trails.iter().all(|t| t.prn.value() == 5));
    }

    #[test]
    fn unplaceable_samples_are_skipped() {
        // Azimuth but no elevation -> not placeable -> no sample, no trail.
        let track = gt_test_utils::loaded_track_with_points(vec![test_util::nav_point_reporting(
            0,
            Some(vec![test_util::sat(
                Constellation::Gps,
                5,
                Some(Azimuth(45.0)),
                None,
                true,
            )]),
        )]);
        let trails = super::extract_trails(&track);
        assert!(trails.trails.is_empty());
        // The epoch still exists - the report was there, just unplaceable.
        assert_eq!(trails.epochs.len(), 1);
    }

    #[test]
    fn a_track_without_reports_has_no_trails() {
        let trails = super::extract_trails(&gt_test_utils::loaded_track_with_points(vec![
            test_util::nav_point_reporting(0, None),
            test_util::nav_point_reporting(1, None),
        ]));
        assert!(trails.trails.is_empty());
        assert!(trails.epochs.is_empty());
        assert_eq!(trails.time_range, None);
    }
}

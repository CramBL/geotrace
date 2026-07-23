//! Whole-track sky trails: each satellite's path across the sky over a
//! track, extracted once from the recorded points for the trails window.
//!
//! Pure data, no rendering - the window projects and draws these, sharing
//! the same [`crate::projection`] the per-report plot uses.

use std::collections::BTreeMap;

use gt_types::satellites::{Constellation, Prn, SlipCause, Snr};
use gt_types::{GeneratedMarkerKind, GpsTime, GpsTimeRange, LoadedTrack, PointIdx};
use smallvec::SmallVec;
use strum::EnumCount as _;

use crate::projection;

/// Which report epoch a [`TrailSample`] came from: an index into
/// [`SkyTrails::epochs`].
///
/// Consecutive indices mean consecutive reports, so a break in them is exactly
/// a gap in the satellite's tracking. Painting a trail asks that question once
/// per sample, and answering it by index is a comparison rather than a search
/// through the epochs.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct EpochIdx(usize);

impl EpochIdx {
    /// Only meaningful against the [`SkyTrails::epochs`] it was built from:
    /// [`extract_trails`] hands each sample the index of the report it came
    /// from, and nothing else constructs one.
    pub fn new(n: usize) -> Self {
        Self(n)
    }

    /// Whether `self` is the epoch immediately after `earlier`, i.e. the two
    /// samples are from back-to-back reports with nothing skipped between.
    pub fn follows(self, earlier: Self) -> bool {
        self.0 == earlier.0 + 1
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

impl SkyTrail {
    /// The sample at exactly `time`, if the satellite reported at that epoch.
    /// A binary search over the ascending-by-time samples.
    pub(crate) fn sample_exactly_at(&self, time: GpsTime) -> Option<&TrailSample> {
        let idx = self.samples.partition_point(|s| s.time < time);
        self.samples.get(idx).filter(|s| s.time == time)
    }

    /// Whether the satellite was in the fix at any point over the track. False
    /// means it was only ever tracked, never used - the trails window can hide
    /// these to focus on the satellites that actually contributed.
    pub fn ever_in_fix(&self) -> bool {
        self.samples.iter().any(|s| s.in_fix)
    }
}

/// Per-constellation satellite counts at one epoch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EpochCount {
    pub constellation: Constellation,
    /// Satellites of this constellation with a sky position at the epoch,
    /// counting only the ones currently shown.
    pub seen: usize,
    /// Satellites with a sky position at the epoch regardless of the
    /// not-in-fix filter, so the window can show what hiding them cost.
    pub seen_unfiltered: usize,
    /// Of the shown ones, those in the fix.
    pub fix: usize,
}

impl EpochCount {
    /// Whether the not-in-fix filter is hiding satellites from [`Self::seen`].
    pub const fn is_filtered(self) -> bool {
        self.seen_unfiltered > self.seen
    }
}

/// One report epoch on the track's timeline - the scrubber walks these.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrailEpoch {
    pub time: GpsTime,
    pub point_index: PointIdx,
}

/// A cycle slip placed on the sky: where a satellite ran into trouble (lost
/// lock, or a sharp SNR drop). Positioned at its last-known sky position
/// before the slip - the `from` side of the detected transition. `from` is
/// used for both causes for a single consistent anchor: `to` is unavailable
/// for a lost-lock slip, and the from/to drift for an SNR drop is one report
/// epoch, negligible at plot scale.
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
    /// Report epochs, ascending by time.
    pub epochs: Vec<TrailEpoch>,
    /// First and last epoch times, or `None` when the track has no reports.
    pub time_range: Option<GpsTimeRange>,
    /// Cycle slips detected over the track, placed on the sky. Empty when the
    /// track has no slip markers.
    pub slips: Vec<SlipMark>,
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

    /// Seen and in-fix counts per constellation at `time`, over the
    /// constellations present in the track. A constellation with no satellite
    /// up at that epoch still gets a row, at zero, so the window's stats rows
    /// stay stable as the scrubber moves. `time` is a report epoch time (from
    /// [`SkyTrails::epochs`]); satellites are matched exactly, not
    /// interpolated.
    ///
    /// When `show_not_in_fix` is false, satellites that were never in the fix
    /// over the track ([`SkyTrail::ever_in_fix`]) are left out entirely, so the
    /// counts match the trails the window is drawing.
    pub fn counts_at(
        &self,
        time: GpsTime,
        show_not_in_fix: bool,
    ) -> SmallVec<[EpochCount; Constellation::COUNT]> {
        self.constellations()
            .map(|constellation| {
                let present = || {
                    self.trails
                        .iter()
                        .filter(|trail| trail.constellation == constellation)
                        .filter(|trail| trail.sample_exactly_at(time).is_some())
                };
                let (seen, fix) = present()
                    .filter(|trail| show_not_in_fix || trail.ever_in_fix())
                    .filter_map(|trail| trail.sample_exactly_at(time))
                    .fold((0, 0), |(seen, fix), s| {
                        (seen + 1, fix + usize::from(s.in_fix))
                    });
                EpochCount {
                    constellation,
                    seen,
                    seen_unfiltered: present().count(),
                    fix,
                }
            })
            .collect()
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
        let epoch = EpochIdx::new(epochs.len());
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
                    epoch,
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
    fn counts_at_splits_seen_and_fix_per_constellation() {
        // One epoch: two GPS satellites up but only one in the fix, one Galileo
        // in the fix.
        let trails = extract_trails(&track(vec![point_at(
            0,
            Some(vec![
                sat(Constellation::Gps, 5, Some(40.0), Some(45.0), true),
                sat(Constellation::Gps, 12, Some(120.0), Some(30.0), false),
                sat(Constellation::Galileo, 3, Some(60.0), Some(50.0), true),
            ]),
        )]));

        let counts = trails.counts_at(trails.epochs[0].time, true);
        let gps = counts
            .iter()
            .find(|c| c.constellation == Constellation::Gps)
            .expect("gps present");
        assert_eq!((gps.seen, gps.fix), (2, 1));
        let galileo = counts
            .iter()
            .find(|c| c.constellation == Constellation::Galileo)
            .expect("galileo present");
        assert_eq!((galileo.seen, galileo.fix), (1, 1));

        // Nothing hidden, so the unfiltered total matches what is shown.
        assert_eq!(gps.seen_unfiltered, 2);
        assert!(!gps.is_filtered());

        // Excluding never-in-fix satellites drops the tracked-only GPS-12, so
        // GPS now counts one seen (and still one in fix) - but the unfiltered
        // total still reports both, so the window can say what was hidden
        // rather than letting the count silently drop.
        let in_fix_only = trails.counts_at(trails.epochs[0].time, false);
        let gps = in_fix_only
            .iter()
            .find(|c| c.constellation == Constellation::Gps)
            .expect("gps present");
        assert_eq!((gps.seen, gps.fix), (1, 1));
        assert_eq!(gps.seen_unfiltered, 2);
        assert!(gps.is_filtered());
    }

    #[test]
    fn ever_in_fix_reflects_any_fix_over_the_track() {
        let trails = extract_trails(&track(vec![
            point_at(
                0,
                Some(vec![sat(
                    Constellation::Gps,
                    5,
                    Some(40.0),
                    Some(45.0),
                    false,
                )]),
            ),
            point_at(
                1,
                Some(vec![sat(
                    Constellation::Gps,
                    5,
                    Some(50.0),
                    Some(40.0),
                    true,
                )]),
            ),
        ]));
        assert!(trails.trails[0].ever_in_fix());

        let never = extract_trails(&track(vec![point_at(
            0,
            Some(vec![sat(
                Constellation::Gps,
                5,
                Some(40.0),
                Some(45.0),
                false,
            )]),
        )]));
        assert!(!never.trails[0].ever_in_fix());
    }

    #[test]
    fn counts_at_between_epochs_finds_nobody() {
        // counts_at matches an epoch exactly (no interpolation), so a time
        // between reports yields zero everywhere.
        let trails = extract_trails(&track(vec![
            point_at(
                0,
                Some(vec![sat(
                    Constellation::Gps,
                    5,
                    Some(40.0),
                    Some(45.0),
                    true,
                )]),
            ),
            point_at(
                2,
                Some(vec![sat(
                    Constellation::Gps,
                    5,
                    Some(50.0),
                    Some(40.0),
                    true,
                )]),
            ),
        ]));
        let between = GpsTime::from_utc(trails.epochs[0].time.utc() + Duration::seconds(1));
        let counts = trails.counts_at(between, true);
        assert!(counts.iter().all(|c| c.seen == 0 && c.fix == 0));
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
        let mut track = track(vec![point_at(0, None)]);
        track.generated_markers = vec![GeneratedMarker {
            time: DateTime::<Utc>::from_timestamp(1_748_000_000, 0).expect("valid"),
            kind: GeneratedMarkerKind::Slip(SlipEvent {
                slips: vec![placeable, unplaceable],
            }),
            lat,
            lon,
            merc: mercator::normalize(lat, lon),
        }];

        let slips = extract_trails(&track).slips;
        assert_eq!(slips.len(), 1);
        assert_eq!(slips[0].constellation, Constellation::Gps);
        assert_eq!(slips[0].azimuth, 120.0);
        assert_eq!(slips[0].elevation, 30.0);
        assert_eq!(slips[0].cause, SlipCause::LostLock);
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

//! Where the receiver was over time, for the context lines whose value
//! depends on position.
//!
//! The interference share and TEC are read over a position. A context line
//! spans the whole plot, including times no recording covers, so those
//! samples are read at the position of the fix nearest them in time.

use std::sync::Arc;

use chrono::{DateTime, Utc};

use gt_types::{Latitude, LoadedFile, Longitude, TimeRange};

/// One fix's position at the second it was recorded.
#[derive(Debug, Clone, Copy)]
struct PositionedFix {
    secs: i64,
    latitude: Latitude,
    longitude: Longitude,
}

/// Every loaded fix's position, oldest first.
#[derive(Debug, Default)]
pub struct FixPositionTimeline {
    fixes: Vec<PositionedFix>,
}

impl FixPositionTimeline {
    fn of(files: &[LoadedFile]) -> Self {
        let mut fixes: Vec<PositionedFix> = files
            .iter()
            .flat_map(|file| file.tracks.iter())
            .filter_map(|track| track.placed_points())
            .flat_map(|placed| placed.iter())
            .map(|point| {
                let (latitude, longitude) = point.resolved_position();
                PositionedFix {
                    secs: point.fix.tpv.time().utc().timestamp(),
                    latitude,
                    longitude,
                }
            })
            .collect();
        fixes.sort_unstable_by_key(|fix| fix.secs);
        Self { fixes }
    }

    /// The position of the fix nearest `time`, or [`None`] with no recording
    /// loaded.
    ///
    /// Ties go to the earlier fix.
    pub fn nearest_position(&self, time: DateTime<Utc>) -> Option<(Latitude, Longitude)> {
        let secs = time.timestamp();
        let after = self.fixes.partition_point(|fix| fix.secs < secs);
        let earlier = after.checked_sub(1).and_then(|index| self.fixes.get(index));
        let later = self.fixes.get(after);
        let nearest = match (earlier, later) {
            (Some(earlier), Some(later)) => {
                if secs - earlier.secs <= later.secs - secs {
                    earlier
                } else {
                    later
                }
            }
            (Some(only), None) | (None, Some(only)) => only,
            (None, None) => return None,
        };
        Some((nearest.latitude, nearest.longitude))
    }
}

/// What a timeline was built from: it is rebuilt exactly when this changes.
#[derive(Debug, Clone, Copy, PartialEq)]
struct TrackShape {
    time_range: TimeRange,
    fix_count: usize,
}

/// Keeps [`FixPositionTimeline`] in step with the loaded recordings.
#[derive(Debug, Default)]
pub struct FixPositions {
    timeline: Arc<FixPositionTimeline>,
    built_from: Vec<TrackShape>,
}

impl FixPositions {
    /// The timeline of `files`, rebuilt when a recording was loaded or
    /// closed. The [`Arc`] identity is what the context lines key their
    /// position-dependent values on.
    pub fn timeline(&mut self, files: &[LoadedFile]) -> &Arc<FixPositionTimeline> {
        let shapes: Vec<TrackShape> = files
            .iter()
            .flat_map(|file| file.tracks.iter())
            .map(|track| TrackShape {
                time_range: track.metadata.time_range,
                fix_count: track.points.len(),
            })
            .collect();
        if self.built_from != shapes {
            self.timeline = Arc::new(FixPositionTimeline::of(files));
            self.built_from = shapes;
        }
        &self.timeline
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeDelta;
    use rstest::rstest;

    use gt_ui_types::ArcIdentity;
    use rustc_hash::FxHashMap;

    use super::*;

    fn at(hour: u32) -> DateTime<Utc> {
        chrono::NaiveDate::from_ymd_opt(2026, 7, 20)
            .and_then(|date| date.and_hms_opt(hour, 0, 0))
            .map(|naive| naive.and_utc())
            .unwrap_or_default()
    }

    fn files_of(track: gt_types::LoadedTrack) -> Vec<LoadedFile> {
        vec![LoadedFile {
            metadata: gt_test_utils::empty_file_metadata(),
            tracks: vec![track],
            event_marker_styles: FxHashMap::default(),
            orphaned_event_markers: vec![],
            load_warnings: vec![],
            source: gt_types::FileSource::GtdBytes(Arc::from(Vec::<u8>::new())),
        }]
    }

    /// Four fixes an hour apart from 08:00.
    fn hourly_track() -> gt_types::LoadedTrack {
        let mut track = gt_test_utils::loaded_track_with_points(
            gt_test_utils::fixtures::nav_points_from(at(8), 4, 3600),
        );
        track.metadata.time_range = TimeRange::new(at(8), at(11));
        track
    }

    /// A time before, inside or after the recording all resolve to the fix
    /// nearest them, so a context sample outside every recording is still
    /// placed somewhere the receiver was.
    #[rstest]
    #[case::long_before(at(0), 0)]
    #[case::the_first_fix(at(8), 0)]
    #[case::between_two_fixes(at(9) + TimeDelta::minutes(31), 2)]
    #[case::the_last_fix(at(11), 3)]
    #[case::long_after(at(23), 3)]
    fn a_time_resolves_to_the_fix_nearest_it(
        #[case] time: DateTime<Utc>,
        #[case] expected_index: usize,
    ) {
        let track = hourly_track();
        let expected = track
            .resolved_position_at(expected_index)
            .expect("the fixture has four placed fixes");

        let timeline = FixPositionTimeline::of(&files_of(track));
        assert_eq!(timeline.nearest_position(time), Some(expected));
    }

    /// Exactly between two fixes the earlier one wins.
    #[test]
    fn a_tie_takes_the_earlier_fix() {
        let track = hourly_track();
        let expected = track
            .resolved_position_at(1)
            .expect("the fixture has four placed fixes");

        let timeline = FixPositionTimeline::of(&files_of(track));
        assert_eq!(
            timeline.nearest_position(at(9) + TimeDelta::minutes(30)),
            Some(expected)
        );
    }

    #[test]
    fn no_recording_leaves_every_time_unplaced() {
        assert_eq!(FixPositionTimeline::default().nearest_position(at(8)), None);
    }

    /// The timeline is rebuilt when the loaded recordings change, and handed
    /// back as the same allocation while they do not.
    #[test]
    fn the_timeline_follows_the_loaded_recordings() {
        let mut positions = FixPositions::default();
        let files = files_of(hourly_track());

        let first = ArcIdentity::of(positions.timeline(&files));
        assert_eq!(ArcIdentity::of(positions.timeline(&files)), first);

        assert_ne!(ArcIdentity::of(positions.timeline(&[])), first);
    }
}

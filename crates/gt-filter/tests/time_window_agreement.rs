//! The global filter's time window, as the three readers of it must agree on
//! it: the per-point predicate, the range form the query evaluates over, and
//! the track-level clause the side panel and the map gate a whole track with.

use chrono::{DateTime, Duration, Utc};
use gt_filter::{GlobalFilter, point_passes_time_filter, time_filtered_range, track_passes_filter};
use gt_types::coordinates::{Latitude, Longitude};
use gt_types::time_types::GpsTime;
use gt_types::tpv::TimePositionVelocity;
use gt_types::{LoadedTrack, NavPoint, TimeRange};

/// The instant every fix and every window below is placed relative to.
fn epoch() -> DateTime<Utc> {
    DateTime::UNIX_EPOCH
}

/// The instant `seconds` after [`epoch`].
fn at(seconds: i64) -> DateTime<Utc> {
    epoch() + Duration::seconds(seconds)
}

/// One fix per entry of `times`, stamped in the order given and all at one
/// position, so only the timestamps distinguish them.
fn fixes_at(times: &[DateTime<Utc>]) -> Vec<NavPoint> {
    times
        .iter()
        .map(|&time| {
            let tpv = TimePositionVelocity::builder()
                .time(GpsTime::from_utc(time))
                .lat(Latitude::new(55.0))
                .lon(Longitude::new(12.0))
                .build();
            NavPoint::new(tpv, None)
        })
        .collect()
}

/// A filter whose only active condition is the time window.
fn window(start: Option<DateTime<Utc>>, end: Option<DateTime<Utc>>) -> GlobalFilter {
    GlobalFilter {
        time_start: start,
        time_end: end,
        ..GlobalFilter::default()
    }
}

/// A track over `fixes`, carrying the metadata time range the track builder
/// computes for them: the span from the earliest fix to the latest one.
fn track_over(fixes: Vec<NavPoint>) -> LoadedTrack {
    let mut track = gt_test_utils::loaded_track_with_points(fixes);
    let times = || track.points.iter().map(|p| p.tpv.time().utc());
    if let Some(first) = times().next() {
        let time_range = TimeRange::spanning(first, times());
        track.metadata.time_range = time_range;
        track.metadata.duration = time_range.duration();
    }
    track
}

/// Whether the per-point predicate keeps the fix at `index`.
fn fix_passes(fixes: &[NavPoint], index: usize, filter: &GlobalFilter) -> bool {
    fixes
        .get(index)
        .is_some_and(|fix| point_passes_time_filter(fix.tpv.time().utc(), filter))
}

/// A fix stamped before its predecessor reaches the filter: nothing sorts the
/// fixes a recording is read from, and a backward time step smaller than the
/// track split gap keeps its fixes in one track. The query must still evaluate
/// the fix at 1 s, which is inside the window that ends at 5 s.
#[test]
fn a_fix_after_a_backward_time_step_stays_inside_the_filtered_range() {
    let fixes = fixes_at(&[at(0), at(10), at(1)]);
    let filter = window(None, Some(at(5)));

    let range = time_filtered_range(&fixes, &filter);

    assert!(
        range.contains(&2),
        "the fix at 1 s passes the window, and the range {range:?} drops it"
    );
}

/// The mirror of the case above: the fix at 10 s is outside the window that
/// ends at 5 s, so the query must not evaluate it.
#[test]
fn a_fix_before_a_backward_time_step_stays_outside_the_filtered_range() {
    let fixes = fixes_at(&[at(10), at(1)]);
    let filter = window(None, Some(at(5)));

    let range = time_filtered_range(&fixes, &filter);

    assert!(
        !range.contains(&0),
        "the fix at 10 s fails the window, and the range {range:?} keeps it"
    );
}

/// Both ends of the window are inclusive, down to the nanosecond the
/// timestamp stores, and the range form draws the same two boundaries.
#[test]
fn both_ends_of_the_window_include_a_fix_stamped_exactly_on_them() {
    let nanosecond = Duration::nanoseconds(1);
    let start = at(10);
    let end = at(20);
    let fixes = fixes_at(&[start - nanosecond, start, end, end + nanosecond]);
    let filter = window(Some(start), Some(end));

    let kept: Vec<bool> = (0..fixes.len())
        .map(|index| fix_passes(&fixes, index, &filter))
        .collect();

    assert_eq!(
        kept,
        vec![false, true, true, false],
        "the window's ends are inclusive, and one nanosecond outside is not"
    );
    assert_eq!(time_filtered_range(&fixes, &filter), 1..3);
}

/// A window of one instant keeps every fix stamped at that instant, however
/// many fixes share the timestamp.
#[test]
fn a_window_of_one_instant_keeps_every_fix_stamped_at_it() {
    let instant = at(1);
    let fixes = fixes_at(&[at(0), instant, instant, instant, at(2)]);
    let filter = window(Some(instant), Some(instant));

    let kept: Vec<bool> = (0..fixes.len())
        .map(|index| fix_passes(&fixes, index, &filter))
        .collect();

    assert_eq!(kept, vec![false, true, true, true, false]);
    assert_eq!(time_filtered_range(&fixes, &filter), 1..4);
}

/// An empty slice of fixes has no fix to select, whatever the window is.
#[test]
fn an_empty_track_yields_an_empty_filtered_range() {
    let filter = window(Some(at(10)), Some(at(20)));

    assert_eq!(time_filtered_range(&[], &filter), 0..0);
}

/// The track-level clause reads the metadata's time range alone, so a track
/// whose recording gap covers the whole window passes it while none of its
/// fixes does. The per-point gates decide what the map then draws.
#[test]
fn a_track_passes_a_window_that_its_recording_gap_covers_entirely() {
    let fixes = fixes_at(&[at(0), at(600)]);
    let track = track_over(fixes.clone());
    let filter = window(Some(at(200)), Some(at(400)));
    assert!(
        (0..fixes.len()).all(|index| !fix_passes(&fixes, index, &filter)),
        "the window is inside the gap, so it keeps no fix"
    );

    assert!(track_passes_filter(&track, &filter));
}

/// A window whose start is after its end excludes every instant: the
/// per-point predicate rejects every fix of the track. The track-level clause
/// must reject the track: it reads the same window.
#[test]
fn an_inverted_window_rejects_a_track_no_fix_of_which_it_keeps() {
    let fixes = fixes_at(&[at(0), at(30), at(60)]);
    let track = track_over(fixes.clone());
    let filter = window(Some(at(40)), Some(at(20)));
    assert!(
        (0..fixes.len()).all(|index| !fix_passes(&fixes, index, &filter)),
        "the inverted window keeps no fix: it selects no instant"
    );

    assert!(
        !track_passes_filter(&track, &filter),
        "the track passes a window that keeps none of its fixes"
    );
}

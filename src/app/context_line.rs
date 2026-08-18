//! The span the context metric lines are sampled over, and the per-day
//! sample cache each of them is assembled from.
//!
//! A context line spans what the plot shows, so it is rebuilt as the view
//! moves. Days are read from the archive once and kept, so a pan reads only
//! the days entering the span.

use std::collections::BTreeMap;
use std::ops::RangeInclusive;
use std::sync::Arc;

use chrono::{DateTime, Datelike as _, NaiveDate, NaiveTime, Utc};
use gt_ui_types::ArcIdentity;

/// How many buckets one visible span is divided into. The span is snapped
/// out to bucket boundaries, so panning re-resolves the lines only once the
/// view has moved by this fraction of what it shows.
const BUCKETS_PER_SPAN: i64 = 8;

/// The UTC days a context line is sampled over: the days the plot shows,
/// widened to whole buckets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContextSpan {
    first: NaiveDate,
    last: NaiveDate,
}

impl ContextSpan {
    /// The span covering a plot x range, in Unix seconds.
    pub fn covering(x_secs: RangeInclusive<f64>) -> Self {
        let start = day_at(*x_secs.start());
        let end = day_at(*x_secs.end());
        let (low, high) = if start <= end {
            (start, end)
        } else {
            (end, start)
        };
        let bucket = (i64::from(high.num_days_from_ce() - low.num_days_from_ce()) + 1)
            .div_euclid(BUCKETS_PER_SPAN)
            .max(1);
        Self {
            first: snap_down(low, bucket),
            last: snap_up(high, bucket),
        }
    }

    pub fn days(self) -> RangeInclusive<NaiveDate> {
        self.first..=self.last
    }
}

/// The UTC day a plot x coordinate falls in. A coordinate outside the
/// representable range lands on the calendar's own end.
fn day_at(x_secs: f64) -> NaiveDate {
    #[expect(
        clippy::cast_possible_truncation,
        reason = "a float-to-integer cast saturates, which the out-of-range fallback then handles"
    )]
    let secs = x_secs as i64;
    DateTime::<Utc>::from_timestamp(secs, 0).map_or(
        if secs < 0 {
            NaiveDate::MIN
        } else {
            NaiveDate::MAX
        },
        |time| time.date_naive(),
    )
}

fn snap_down(day: NaiveDate, bucket: i64) -> NaiveDate {
    let days_from_ce = i64::from(day.num_days_from_ce());
    day_from_ce(
        days_from_ce - days_from_ce.rem_euclid(bucket),
        NaiveDate::MIN,
    )
}

fn snap_up(day: NaiveDate, bucket: i64) -> NaiveDate {
    let days_from_ce = i64::from(day.num_days_from_ce());
    day_from_ce(
        days_from_ce + bucket - 1 - days_from_ce.rem_euclid(bucket),
        NaiveDate::MAX,
    )
}

fn day_from_ce(days_from_ce: i64, beyond_the_calendar: NaiveDate) -> NaiveDate {
    i32::try_from(days_from_ce)
        .ok()
        .and_then(NaiveDate::from_num_days_from_ce_opt)
        .unwrap_or(beyond_the_calendar)
}

/// A UTC day's midnight as a plot x coordinate.
pub fn midnight_secs(day: NaiveDate) -> f64 {
    day.and_time(NaiveTime::MIN).and_utc().timestamp() as f64
}

/// What a context line was resolved from. It is rebuilt exactly when this
/// changes.
#[derive(Debug, Clone, PartialEq)]
pub struct ContextSource {
    pub span: ContextSpan,
    /// The archived days inside the span, oldest first.
    pub archived_days: Vec<NaiveDate>,
    /// The fix positions the values were read at, absent for a planetary
    /// index, whose values do not depend on where the receiver was.
    pub positions: Option<ArcIdentity>,
}

/// The samples one context line is assembled from, and the archived days
/// they were read from.
#[derive(Debug)]
pub struct ContextSampleCache<S> {
    /// The samples each archived day contributed, kept so a pan reads only
    /// the days entering the span. One entry per archived day the plot has
    /// shown.
    days: BTreeMap<NaiveDate, Vec<S>>,
    resolved_from: Option<ContextSource>,
    line: Arc<Vec<S>>,
}

impl<S> Default for ContextSampleCache<S> {
    fn default() -> Self {
        Self {
            days: BTreeMap::new(),
            resolved_from: None,
            line: Arc::new(Vec::new()),
        }
    }
}

impl<S: Clone> ContextSampleCache<S> {
    /// Drop what was read for `day`, so an archive the fetch worker revised
    /// is read again.
    pub fn forget(&mut self, day: NaiveDate) {
        if self.days.remove(&day).is_some() {
            self.resolved_from = None;
        }
    }

    /// The samples `source` describes, assembled from the archive.
    ///
    /// `read_day` produces one day's samples, oldest first. `gap_at` builds
    /// the valueless sample that ends a stretch where the next day holds
    /// nothing, so a line breaks over days the archive does not cover. It
    /// yields [`None`] for samples drawn one by one, which have no stretches
    /// to break.
    pub fn resolve(
        &mut self,
        source: ContextSource,
        mut read_day: impl FnMut(NaiveDate) -> Vec<S>,
        gap_at: impl Fn(NaiveDate) -> Option<S>,
    ) -> Arc<Vec<S>> {
        if self
            .resolved_from
            .as_ref()
            .is_some_and(|resolved| resolved.positions != source.positions)
        {
            self.days.clear();
        }
        if self.resolved_from.as_ref() == Some(&source) {
            return Arc::clone(&self.line);
        }

        let mut samples: Vec<S> = Vec::new();
        let mut previous: Option<NaiveDate> = None;
        for &day in &source.archived_days {
            let read = self
                .days
                .entry(day)
                .or_insert_with(|| read_day(day))
                .as_slice();
            if read.is_empty() {
                continue;
            }
            if let Some(uncovered) = previous
                .and_then(|previous| previous.succ_opt())
                .filter(|uncovered| *uncovered != day)
            {
                samples.extend(gap_at(uncovered));
            }
            samples.extend_from_slice(read);
            previous = Some(day);
        }

        self.line = Arc::new(samples);
        self.resolved_from = Some(source);
        Arc::clone(&self.line)
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    fn day(year: i32, month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(year, month, day).unwrap_or_default()
    }

    fn at(year: i32, month: u32, day_of_month: u32, hour: u32) -> f64 {
        day(year, month, day_of_month)
            .and_hms_opt(hour, 0, 0)
            .map_or(0.0, |naive| naive.and_utc().timestamp() as f64)
    }

    /// A span shorter than the bucket count is snapped out to whole days on
    /// both sides, so panning within a day reuses the resolved line.
    #[test]
    fn a_span_inside_one_day_covers_that_day() {
        let span = ContextSpan::covering(at(2026, 7, 20, 8)..=at(2026, 7, 20, 17));
        assert_eq!(span.days(), day(2026, 7, 20)..=day(2026, 7, 20));
    }

    /// A span of weeks snaps out to a bucket an eighth of its length, so a
    /// pan of a few days reads the same line back.
    #[test]
    fn a_long_span_snaps_out_to_whole_buckets() {
        let span = ContextSpan::covering(at(2026, 1, 1, 0)..=at(2026, 3, 1, 0));
        let moved = ContextSpan::covering(at(2026, 1, 2, 0)..=at(2026, 3, 2, 0));
        assert_eq!(span, moved);
        assert!(span.days().contains(&day(2026, 1, 1)));
        assert!(span.days().contains(&day(2026, 3, 1)));
    }

    /// The snapped span never loses a day of the view.
    #[rstest]
    #[case::one_day(at(2026, 7, 20, 0), at(2026, 7, 20, 23))]
    #[case::a_week(at(2026, 7, 20, 0), at(2026, 7, 27, 0))]
    #[case::a_year(at(2026, 1, 1, 0), at(2026, 12, 31, 0))]
    #[case::inverted(at(2026, 7, 27, 0), at(2026, 7, 20, 0))]
    fn the_span_covers_the_view(#[case] x_min: f64, #[case] x_max: f64) {
        let span = ContextSpan::covering(x_min..=x_max);
        assert!(span.days().contains(&day_at(x_min)));
        assert!(span.days().contains(&day_at(x_max)));
    }

    /// A view zoomed past the calendar produces a span, not a panic.
    #[test]
    fn a_view_beyond_the_calendar_still_produces_a_span() {
        let span = ContextSpan::covering(f64::MIN..=f64::MAX);
        assert!(span.days().contains(&day(2026, 7, 20)));
    }

    #[derive(Debug, Clone, Copy, PartialEq)]
    struct Sample {
        day: NaiveDate,
        value: Option<u32>,
    }

    fn source(days: &[NaiveDate]) -> ContextSource {
        ContextSource {
            span: ContextSpan::covering(at(2026, 7, 20, 0)..=at(2026, 7, 27, 0)),
            archived_days: days.to_vec(),
            positions: None,
        }
    }

    fn valued(day: NaiveDate) -> Vec<Sample> {
        vec![Sample {
            day,
            value: Some(1),
        }]
    }

    fn gap(day: NaiveDate) -> Option<Sample> {
        Some(Sample { day, value: None })
    }

    /// Days the archive does not cover break the line, and adjacent days run
    /// on unbroken.
    #[test]
    fn a_missing_day_breaks_the_line() {
        let mut cache = ContextSampleCache::default();
        let archived = [day(2026, 7, 20), day(2026, 7, 21), day(2026, 7, 24)];

        let line = cache.resolve(source(&archived), valued, gap);

        assert_eq!(
            line.as_slice(),
            [
                Sample {
                    day: day(2026, 7, 20),
                    value: Some(1)
                },
                Sample {
                    day: day(2026, 7, 21),
                    value: Some(1)
                },
                Sample {
                    day: day(2026, 7, 22),
                    value: None
                },
                Sample {
                    day: day(2026, 7, 24),
                    value: Some(1)
                },
            ]
        );
    }

    /// An archived day the source published nothing for breaks the line like
    /// a day that was never fetched.
    #[test]
    fn an_archived_day_without_samples_breaks_the_line() {
        let mut cache = ContextSampleCache::default();
        let archived = [day(2026, 7, 20), day(2026, 7, 21), day(2026, 7, 22)];

        let line = cache.resolve(
            source(&archived),
            |read| {
                if read == day(2026, 7, 21) {
                    Vec::new()
                } else {
                    valued(read)
                }
            },
            gap,
        );

        assert_eq!(
            line.iter().map(|sample| sample.value).collect::<Vec<_>>(),
            [Some(1), None, Some(1)]
        );
    }

    /// A resolved line is handed back as the same allocation until its
    /// source changes, which is what keeps the plot from rebuilding its
    /// mipmaps every frame.
    #[test]
    fn an_unchanged_source_hands_back_the_same_line() {
        let mut cache = ContextSampleCache::default();
        let archived = [day(2026, 7, 20)];

        let first = cache.resolve(source(&archived), valued, gap);
        let again = cache.resolve(source(&archived), valued, gap);
        assert_eq!(ArcIdentity::of(&first), ArcIdentity::of(&again));

        let extended = cache.resolve(source(&[day(2026, 7, 20), day(2026, 7, 21)]), valued, gap);
        assert_ne!(ArcIdentity::of(&first), ArcIdentity::of(&extended));
    }

    /// A day is read once: a later span holding it again reuses what was
    /// read, so panning back and forth costs no archive reads.
    #[test]
    fn a_day_already_read_is_not_read_again() {
        let mut cache = ContextSampleCache::default();
        let mut reads = Vec::new();
        let mut read_day = |read: NaiveDate| {
            reads.push(read);
            valued(read)
        };

        cache.resolve(source(&[day(2026, 7, 20)]), &mut read_day, gap);
        cache.resolve(
            source(&[day(2026, 7, 20), day(2026, 7, 21)]),
            &mut read_day,
            gap,
        );

        assert_eq!(reads, [day(2026, 7, 20), day(2026, 7, 21)]);
    }

    /// The values of the position-dependent lines are read where the
    /// receiver was, so a changed set of recordings drops what was read.
    #[test]
    fn changed_positions_drop_the_days_already_read() {
        let mut cache = ContextSampleCache::default();
        let mut reads = Vec::new();
        let mut read_day = |read: NaiveDate| {
            reads.push(read);
            valued(read)
        };
        let positioned = |positions| ContextSource {
            positions: Some(positions),
            ..source(&[day(2026, 7, 20)])
        };
        let (first, second) = (Arc::new(0_u8), Arc::new(0_u8));

        cache.resolve(positioned(ArcIdentity::of(&first)), &mut read_day, gap);
        cache.resolve(positioned(ArcIdentity::of(&second)), &mut read_day, gap);

        assert_eq!(reads, [day(2026, 7, 20), day(2026, 7, 20)]);
    }
}

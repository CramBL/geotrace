use chrono::{DateTime, Duration, Utc};

use crate::error::Error;
use crate::types::{
    Annotation, DebugTimeRepair, EventMarkerPoint, Marker, NavFile, NavFixTime, NavPoint,
    SatelliteReport,
};

pub(crate) fn repair_repeated_time_spans(
    nav_file: &mut NavFile,
    config: DebugTimeRepair,
) -> Result<(), Error> {
    let repair_map = TimeRepairMap::from_nav_times(
        nav_file
            .nav_points
            .iter()
            .map(|point| point.fix.effective_gps_time()),
        config.backward_jump_threshold(),
    )?;
    for (point, shift) in nav_file
        .nav_points
        .iter_mut()
        .zip(repair_map.nav_shifts.iter().copied())
    {
        shift_nav_point(point, shift)?;
    }

    let mut marker_cursor = TimeRepairCursor::default();
    for marker in &mut nav_file.markers {
        let shift = repair_map.shift_for_series_time(marker.annotation.time, &mut marker_cursor)?;
        shift_marker(marker, shift)?;
    }

    let mut event_marker_cursor = TimeRepairCursor::default();
    for event_marker in &mut nav_file.event_markers {
        let shift =
            repair_map.shift_for_series_time(event_marker.sys_time, &mut event_marker_cursor)?;
        shift_event_marker(event_marker, shift)?;
    }

    for channel in &mut nav_file.channels {
        let mut channel_cursor = TimeRepairCursor::default();
        for time in &mut channel.times {
            let shift = repair_map.shift_for_series_time(*time, &mut channel_cursor)?;
            *time = shift_time(*time, shift)?;
        }
    }

    Ok(())
}

#[derive(Debug, Default)]
struct TimeRepairCursor {
    span_index: usize,
    previous_repaired: Option<DateTime<Utc>>,
}

#[derive(Debug)]
struct TimeRepairMap {
    nav_shifts: Vec<Duration>,
    spans: Vec<TimeRepairSpan>,
}

impl TimeRepairMap {
    fn from_nav_times(
        times: impl IntoIterator<Item = DateTime<Utc>>,
        backward_jump_threshold: Duration,
    ) -> Result<Self, Error> {
        let mut nav_shifts = Vec::new();
        let mut spans: Vec<TimeRepairSpan> = Vec::new();
        let mut current_shift = Duration::zero();
        let mut current_span: Option<TimeRepairSpan> = None;
        let mut previous_original = None;
        let mut previous_repaired = None;
        for time in times {
            if let (Some(original), Some(repaired)) = (previous_original, previous_repaired)
                && original - time >= backward_jump_threshold
            {
                if let Some(span) = current_span.take() {
                    spans.push(span);
                }
                current_shift = shift_time(repaired, backward_jump_threshold)? - time;
            }
            current_span = Some(match current_span {
                Some(span) => span.with_time(time),
                None => TimeRepairSpan::new(time, current_shift),
            });

            let repaired = shift_time(time, current_shift)?;
            nav_shifts.push(current_shift);
            previous_original = Some(time);
            previous_repaired = Some(repaired);
        }
        if let Some(span) = current_span {
            spans.push(span);
        }
        Ok(Self { nav_shifts, spans })
    }

    fn shift_for_series_time(
        &self,
        time: DateTime<Utc>,
        cursor: &mut TimeRepairCursor,
    ) -> Result<Duration, Error> {
        let Some((span_index, span)) = self.best_span_for_series_time(time, cursor) else {
            return Ok(Duration::zero());
        };
        cursor.span_index = span_index;
        cursor.previous_repaired = Some(shift_time(time, span.shift)?);
        Ok(span.shift)
    }

    fn best_span_for_series_time(
        &self,
        time: DateTime<Utc>,
        cursor: &TimeRepairCursor,
    ) -> Option<(usize, TimeRepairSpan)> {
        let containing_span = self
            .spans
            .iter()
            .copied()
            .enumerate()
            .skip(cursor.span_index)
            .find(|(_, span)| span.contains(time));

        self.spans
            .iter()
            .copied()
            .enumerate()
            .skip(cursor.span_index)
            .find(|(_, span)| {
                span.contains(time)
                    && cursor.previous_repaired.is_none_or(|previous| {
                        time.checked_add_signed(span.shift)
                            .is_some_and(|repaired| repaired >= previous)
                    })
            })
            .or(containing_span)
            .or_else(|| {
                let span_index = cursor.span_index.min(self.spans.len().saturating_sub(1));
                self.spans
                    .get(span_index)
                    .copied()
                    .map(|span| (span_index, span))
            })
    }
}

#[derive(Debug, Clone, Copy)]
struct TimeRepairSpan {
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    shift: Duration,
}

impl TimeRepairSpan {
    fn new(time: DateTime<Utc>, shift: Duration) -> Self {
        Self {
            start: time,
            end: time,
            shift,
        }
    }

    fn with_time(self, time: DateTime<Utc>) -> Self {
        Self {
            start: self.start.min(time),
            end: self.end.max(time),
            shift: self.shift,
        }
    }

    fn contains(self, time: DateTime<Utc>) -> bool {
        self.start <= time && time <= self.end
    }
}

fn shift_nav_point(point: &mut NavPoint, shift: Duration) -> Result<(), Error> {
    point.fix.time = shift_fix_time(point.fix.time, shift)?;
    if let Some(satellites) = &mut point.satellites {
        shift_satellite_report(satellites, shift)?;
    }
    Ok(())
}

fn shift_marker(marker: &mut Marker, shift: Duration) -> Result<(), Error> {
    marker.annotation = Annotation {
        time: shift_time(marker.annotation.time, shift)?,
        label: marker.annotation.label.clone(),
        icon: marker.annotation.icon,
    };
    Ok(())
}

fn shift_event_marker(marker: &mut EventMarkerPoint, shift: Duration) -> Result<(), Error> {
    marker.sys_time = shift_time(marker.sys_time, shift)?;
    Ok(())
}

fn shift_satellite_report(report: &mut SatelliteReport, shift: Duration) -> Result<(), Error> {
    report.time = shift_fix_time(report.time, shift)?;
    Ok(())
}

fn shift_fix_time(time: NavFixTime, shift: Duration) -> Result<NavFixTime, Error> {
    match time {
        NavFixTime::Both { gps, sys } => Ok(NavFixTime::Both {
            gps: shift_time(gps, shift)?,
            sys: shift_time(sys, shift)?,
        }),
        NavFixTime::Host(sys) => Ok(NavFixTime::Host(shift_time(sys, shift)?)),
        NavFixTime::Receiver(gps) => Ok(NavFixTime::Receiver(shift_time(gps, shift)?)),
    }
}

fn shift_time(time: DateTime<Utc>, shift: Duration) -> Result<DateTime<Utc>, Error> {
    time.checked_add_signed(shift)
        .ok_or(Error::DebugTimeRepairTimestampOutOfRange {
            timestamp: time,
            shift_us: shift.num_microseconds().unwrap_or_else(|| {
                if shift < Duration::zero() {
                    i64::MIN
                } else {
                    i64::MAX
                }
            }),
        })
}

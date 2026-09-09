use std::{
    borrow::Cow,
    fmt::{self, Write},
    num::NonZeroUsize,
    ops::Range,
};

use chrono::{DateTime, Utc};
use gt_types::LoadedTrack;
use gt_types::track::FixStats;

pub mod name_template;
pub use name_template::{NameFields, Token, render_name_template};
use uom::si::{
    f64,
    length::{kilometer, meter},
};

/// U+2014 EM DASH, standing in for a value that is absent.
pub const EM_DASH: &str = "—";

/// U+2212 MINUS SIGN, visually distinct from a hyphen in front of a number.
/// Re-exported by `gt-ui-theme` alongside the other UI glyphs.
pub const MINUS_SIGN: &str = "−";

/// U+2026 HORIZONTAL ELLIPSIS, marking a value cut short.
/// Re-exported by `gt-ui-theme` alongside the other UI glyphs.
pub const ELLIPSIS: &str = "…";

/// U+00B7 MIDDLE DOT, separating the fields of a one-line summary.
pub const MIDDLE_DOT: &str = "·";

/// U+2013 EN DASH, joining the two ends of a numeric range.
pub const EN_DASH: &str = "–";

/// U+2192 RIGHTWARDS ARROW, leading from where a span starts to where it ends.
pub const RIGHTWARDS_ARROW: &str = "→";

/// U+2248 ALMOST EQUAL TO, marking a value that stands for another.
pub const ALMOST_EQUAL_TO: &str = "≈";

/// A UTC instant to the minute, the precision the solar flare, geomagnetic
/// index and TEC map archives publish. The surface showing it writes `UTC`
/// after it.
pub const UTC_MINUTE_FORMAT: &str = "%Y-%m-%d %H:%M";

/// A UTC instant to the second, the precision a fix or a plot sample has.
/// The surface showing it writes `UTC` after it.
pub const UTC_SECOND_FORMAT: &str = "%Y-%m-%d %H:%M:%S";

/// Two spaces, U+00B7 MIDDLE DOT, two spaces, joins fields inside tooltip strings.
const TOOLTIP_JOINER: &str = "  ·  ";

/// Returns the percentage of time spent with a satellite fix, rounded to the
/// nearest integer in `[0, 100]`.
///
/// Returns `0` when `time_with_fix + time_without_fix` is zero or negative.
pub fn fix_percentage(stats: FixStats) -> u32 {
    let total_ms = (stats.time_with_fix + stats.time_without_fix).num_milliseconds();
    if total_ms <= 0 {
        return 0;
    }
    let fix_ms = stats.time_with_fix.num_milliseconds().max(0);
    u32::try_from((fix_ms * 100 + total_ms / 2) / total_ms).unwrap_or(100)
}

/// Formats just the fix-percentage portion of the tooltip, e.g. `"85% fix"`.
pub fn format_fix_percentage(stats: FixStats) -> String {
    format!("{}% fix", fix_percentage(stats))
}

/// Formats a `0..=1` fraction as a whole percentage, e.g. `0.87` -> `"87%"`.
///
/// Rounds half-up like [`fix_percentage`] and clamps to `[0, 100]`, so an
/// out-of-range or non-finite input degrades to a bound.
pub fn format_fraction_percent(fraction: f64) -> String {
    // `clamp` propagates NaN, so map non-finite inputs to the lower bound
    // explicitly.
    let percent = (fraction * 100.0 + 0.5).floor().clamp(0.0, 100.0);
    let percent = if percent.is_finite() { percent } else { 0.0 };
    format!("{percent}%")
}

/// Formats the remaining tooltip details (time without fix, loss count, max
/// continuous gap), each prefixed by `TOOLTIP_JOINER`.
///
/// Returns an empty string when there is nothing to add beyond the
/// percentage, e.g. for `"100% fix"` with no losses.
pub fn format_fix_tooltip_details(stats: FixStats) -> String {
    let mut parts: Vec<String> = Vec::new();
    if stats.time_without_fix > chrono::Duration::zero() {
        parts.push(format!(
            "{} w/o fix",
            format_human_terse_duration(stats.time_without_fix)
        ));
    }
    if stats.fix_loss_count == 1 {
        parts.push("1 loss".to_owned());
    } else if stats.fix_loss_count > 1 {
        parts.push(format!("{} losses", stats.fix_loss_count));
    }
    if stats.max_continuous_no_fix > chrono::Duration::zero() {
        parts.push(format!(
            "max gap {}",
            format_human_terse_duration(stats.max_continuous_no_fix)
        ));
    }
    if parts.is_empty() {
        String::new()
    } else {
        format!("{TOOLTIP_JOINER}{}", parts.join(TOOLTIP_JOINER))
    }
}

/// Format a distance as a compact human-readable string.
///
/// Uses metres for distances under 1 km, kilometres otherwise.
pub fn format_distance(d: f64::Length) -> String {
    let km = d.get::<kilometer>();
    if km < 1.0 {
        format!("{:.0}m", d.get::<meter>())
    } else {
        format!("{km:.1}km")
    }
}

/// A distance in kilometres to one decimal, without the unit. A distance under
/// 50 m reads `0.0`.
pub fn format_kilometers(d: f64::Length) -> String {
    format!("{:.1}", d.get::<kilometer>())
}

/// Formats a duration as a timeline offset: `M:SS`, or `H:MM:SS` once past an
/// hour. Unlike [`format_human_terse_duration`] the fields are fixed-width and
/// colon-separated, so a running position reads like a media scrubber's clock.
/// Negative durations (a position before the start) clamp to zero.
pub fn format_timeline_offset(d: chrono::Duration) -> String {
    let secs = d.num_seconds().max(0);
    DurationClockFormat::fitting_longest_duration(secs).format_seconds(secs)
}

/// Format a duration as a compact human-readable string.
///
/// Rules:
/// - Durations ≥ 48 h are shown as `"Xd"` or `"XdYh"` (e.g. `"2d5h"`).
/// - Seconds are shown only when the total duration is under 2 minutes.
/// - Minutes are shown only when the whole hours is less than 3.
/// - Zero-valued components are omitted entirely.
/// - A duration between zero and a second is shown in tenths of a second,
///   rounded up and capped at `"0.9s"`, so it never reads as no time at all.
/// - Zero duration returns `"0s"` to avoid an empty string.
///
/// Examples: `"20m"`, `"1h28m"`, `"3h"`, `"1m30s"`, `"45s"`, `"0.4s"`, `"2d5h"`.
#[expect(
    clippy::let_underscore_must_use,
    reason = "writing to String cannot fail"
)]
pub fn format_human_terse_duration(d: chrono::Duration) -> String {
    let total_secs = d.num_seconds();

    if total_secs == 0 {
        let millis = d.num_milliseconds();
        return if millis <= 0 {
            "0s".to_owned()
        } else {
            format!("0.{}s", millis.unsigned_abs().div_ceil(100).min(9))
        };
    }

    let h = d.num_hours();
    let m = d.num_minutes() % 60;
    let s = total_secs % 60;

    if h >= 48 {
        let days = h / 24;
        let rem_h = h % 24;
        return if rem_h > 0 {
            format!("{days}d{rem_h}h")
        } else {
            format!("{days}d")
        };
    }

    let show_s = total_secs < 120;
    let show_m = h < 3;

    let mut out = String::with_capacity(16);

    if h > 0 {
        let _ = write!(out, "{h}h");
    }
    if show_m && m > 0 {
        let _ = write!(out, "{m}m");
    }
    if show_s && s > 0 {
        let _ = write!(out, "{s}s");
    }

    out
}

const MICROS_PER_MILLISECOND: u64 = 1_000;

const MICROS_PER_SECOND: u64 = 1_000 * MICROS_PER_MILLISECOND;

const MICROS_PER_MINUTE: u64 = 60 * MICROS_PER_SECOND;

/// A magnitude as whole units and thousandths of a unit, written `4`, `4.5` or
/// `4.512`.
struct UnitsAndThousandths {
    units: u64,
    thousandths: u64,
}

impl fmt::Display for UnitsAndThousandths {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.thousandths == 0 {
            return write!(f, "{}", self.units);
        }
        let fraction = format!("{:03}", self.thousandths);
        write!(f, "{}.{}", self.units, fraction.trim_end_matches('0'))
    }
}

/// Format a duration as [`format_human_terse_duration`] does, with everything
/// under a minute resolved to the microsecond: `"900µs"`, `"4ms"`, `"4.5ms"`,
/// `"1.5s"`, `"1m30s"`.
///
/// Under a second the reading is exact. From a second to a minute the fraction
/// stops at milliseconds. From a minute up the reading is
/// [`format_human_terse_duration`]'s. A negative duration is led by
/// [`MINUS_SIGN`].
pub fn format_human_terse_duration_with_microseconds(d: chrono::Duration) -> String {
    let Some(micros) = d.num_microseconds() else {
        return format_human_terse_duration(d);
    };
    let sign = if micros < 0 { MINUS_SIGN } else { "" };
    let magnitude = micros.unsigned_abs();
    if magnitude == 0 {
        "0s".to_owned()
    } else if magnitude < MICROS_PER_MILLISECOND {
        format!("{sign}{magnitude}µs")
    } else if magnitude < MICROS_PER_SECOND {
        format!(
            "{sign}{}ms",
            UnitsAndThousandths {
                units: magnitude / MICROS_PER_MILLISECOND,
                thousandths: magnitude % MICROS_PER_MILLISECOND,
            }
        )
    } else if magnitude < MICROS_PER_MINUTE {
        format!(
            "{sign}{}s",
            UnitsAndThousandths {
                units: magnitude / MICROS_PER_SECOND,
                thousandths: (magnitude % MICROS_PER_SECOND) / MICROS_PER_MILLISECOND,
            }
        )
    } else {
        format!("{sign}{}", format_human_terse_duration(d.abs()))
    }
}

/// Format a signed time delta, in milliseconds, for display beside a value.
///
/// - Sub-2-second deltas are shown as `+250ms` / `−1500ms`.
/// - 2s–59s: fractional seconds up to 2 decimal places with trailing zeros
///   dropped (`+2.1s`, `+9.23s`).
/// - ≥1 minute: compact terse format (`+1m9s`, `+1h2m`).
///
/// The negative sign uses [`MINUS_SIGN`] so it is visually distinct from a hyphen.
pub fn format_signed_delta(delta_ms: i64) -> String {
    let sign = if delta_ms < 0 { MINUS_SIGN } else { "+" };
    let abs_ms = delta_ms.unsigned_abs();
    if abs_ms < 2_000 {
        format!("{sign}{abs_ms}ms")
    } else if abs_ms < 60_000 {
        let secs = abs_ms / 1_000;
        let frac = (abs_ms % 1_000) / 10;
        if frac == 0 {
            format!("{sign}{secs}s")
        } else if frac.is_multiple_of(10) {
            format!("{sign}{secs}.{}s", frac / 10)
        } else {
            format!("{sign}{secs}.{frac:02}s")
        }
    } else {
        let total_s = abs_ms / 1_000;
        let h = total_s / 3_600;
        let m = (total_s % 3_600) / 60;
        let s = total_s % 60;
        let mut out = sign.to_owned();
        if h > 0 {
            write!(out, "{h}h").ok();
        }
        if m > 0 {
            write!(out, "{m}m").ok();
        }
        if s > 0 || (h == 0 && m == 0) {
            write!(out, "{s}s").ok();
        }
        out
    }
}

/// Formats a time span for a tooltip header: the start date and time, then the
/// end time - including the end date as well when the span crosses midnight
/// into a different day. Times are UTC, matching the rest of the UI.
pub fn format_time_range(start: DateTime<Utc>, end: DateTime<Utc>) -> String {
    let start_str = start.format(UTC_SECOND_FORMAT);
    if start.date_naive() == end.date_naive() {
        format!("{start_str} – {}", end.format("%H:%M:%S"))
    } else {
        format!("{start_str} – {}", end.format(UTC_SECOND_FORMAT))
    }
}

/// The last index `range` covers, absent where it holds one point or none.
/// Both ends of a span are stated only where they differ.
pub fn last_index_of_span(range: &Range<usize>) -> Option<usize> {
    range.end.checked_sub(1).filter(|last| *last > range.start)
}

/// Wall-clock seconds from a match's first to its last point. `None` for a
/// single-point match and for a range reaching past the track.
pub fn match_duration_seconds(track: &LoadedTrack, range: &Range<usize>) -> Option<i64> {
    let last = last_index_of_span(range)?;
    let first = track.points.get(range.start)?;
    let last = track.points.get(last)?;
    Some((last.tpv.time().utc() - first.tpv.time().utc()).num_seconds())
}

/// A match's duration, as `42s` under a minute and `12:34min` above it.
pub fn format_match_duration(secs: i64) -> String {
    if secs >= 60 {
        format!("{}:{:02}min", secs / 60, secs % 60)
    } else {
        format!("{secs}s")
    }
}

/// Seconds in one hour, the point a clock reading grows an hours field.
const SECONDS_PER_HOUR: u64 = 3_600;

/// How a duration prints as a clock reading. A table picks one format for a
/// whole column: cells of different magnitudes then line up under each other.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DurationClockFormat {
    /// `1:01`. The minutes count on past 59 for a duration past an hour.
    MinutesSeconds,
    /// `1:23:45`.
    HoursMinutesSeconds,
}

impl DurationClockFormat {
    /// The narrowest format that gives a duration of `longest_secs` an hours
    /// field, whichever way that duration runs.
    pub fn fitting_longest_duration(longest_secs: i64) -> Self {
        if longest_secs.unsigned_abs() >= SECONDS_PER_HOUR {
            Self::HoursMinutesSeconds
        } else {
            Self::MinutesSeconds
        }
    }

    /// `secs` as a clock reading, a negative duration led by [`MINUS_SIGN`].
    pub fn format_seconds(self, secs: i64) -> String {
        let sign = if secs < 0 { MINUS_SIGN } else { "" };
        let magnitude = secs.unsigned_abs();
        let seconds = magnitude % 60;
        match self {
            Self::MinutesSeconds => format!("{sign}{}:{seconds:02}", magnitude / 60),
            Self::HoursMinutesSeconds => format!(
                "{sign}{}:{:02}:{seconds:02}",
                magnitude / SECONDS_PER_HOUR,
                (magnitude % SECONDS_PER_HOUR) / 60
            ),
        }
    }
}

pub fn pluralize<'a>(count: usize, singular: &'a str, plural: &'a str) -> &'a str {
    if count == 1 { singular } else { plural }
}

/// Bytes in one kibibyte. The steps between display units.
pub const BYTES_PER_KB: u64 = 1_024;

/// Bytes in one gibibyte, the unit storage limits are entered in.
pub const BYTES_PER_GB: u64 = BYTES_PER_KB * BYTES_PER_KB * BYTES_PER_KB;

/// Format a byte count in binary units (`1.5 KB`, `126.6 MB`, `10.0 GB`).
///
/// Data sizes keep the space between the number and its unit.
/// Zero renders as an em dash, the table convention for an absent value.
pub fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["KB", "MB", "GB", "TB"];
    if bytes == 0 {
        return EM_DASH.to_owned();
    }
    if bytes < BYTES_PER_KB {
        return format!("{bytes} B");
    }
    let step = BYTES_PER_KB as f64;
    let mut scaled = bytes as f64 / step;
    let mut unit = UNITS[0];
    for larger in UNITS.iter().skip(1) {
        if scaled < step {
            break;
        }
        scaled /= step;
        unit = larger;
    }
    format!("{scaled:.1} {unit}")
}

/// `value` cut to its first `max_chars` characters, with [`ELLIPSIS`] appended
/// when characters were dropped. The ellipsis is extra, not part of the limit.
pub fn truncate_with_ellipsis(value: &str, max_chars: NonZeroUsize) -> Cow<'_, str> {
    match value.char_indices().nth(max_chars.get()) {
        Some((cut, _)) => {
            let head = value.get(..cut).unwrap_or(value);
            Cow::Owned(format!("{head}{ELLIPSIS}"))
        }
        None => Cow::Borrowed(value),
    }
}

/// Format a count with comma thousands separators (`8,940`).
pub fn format_count(n: usize) -> String {
    group_thousands(&n.to_string())
}

/// [`format_count`] for a count that arrives as a [`u64`], such as one a log
/// exporter stated about the file it wrote.
pub fn format_count_u64(n: u64) -> String {
    group_thousands(&n.to_string())
}

fn group_thousands(digits: &str) -> String {
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests;

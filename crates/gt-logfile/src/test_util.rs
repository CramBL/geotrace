#![cfg(test)]
//! Fixtures shared between the test modules of gt-logfile.

use chrono::{DateTime, TimeZone as _, Utc};

pub mod strategies;

/// The UTC instant of a wall-clock reading, in the field order
/// [`chrono::TimeZone::with_ymd_and_hms`] takes.
pub fn utc(y: i32, mo: u32, d: u32, h: u32, m: u32, s: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(y, mo, d, h, m, s)
        .single()
        .expect("valid")
}

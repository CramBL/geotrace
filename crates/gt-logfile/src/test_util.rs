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

/// A real journald summary with two rows per service table.
pub(crate) const EXPORTED_SUMMARY: &str = "\
----------- Journal summary -----------
Device type: nav-devkit-mk2
Logs begin at: Thu 29-May-2025 18:48:25 UTC
Logs end at  : Fri 26-Jun-2026 07:59:50 UTC
Log entries: 622286
--- Service error count ---
hal-powerd           -> 56429 Errors
ofonod               -> 1092 Errors
--- Service warning count ---
core-appd               -> 29562 Warnings
kernel               -> 315 Warnings";

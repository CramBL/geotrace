//! Converting between the [`chrono`] dates the app works in and the [`jiff`]
//! dates `egui_extras`' date picker edits.

use chrono::{Datelike as _, NaiveDate};
use jiff::civil::Date;

pub fn to_jiff(date: NaiveDate) -> Date {
    i16::try_from(date.year())
        .ok()
        .zip(i8::try_from(date.month()).ok())
        .zip(i8::try_from(date.day()).ok())
        .and_then(|((year, month), day)| Date::new(year, month, day).ok())
        .unwrap_or_default()
}

/// The date, or [`None`] for one no calendar day matches.
pub fn to_chrono(date: Date) -> Option<NaiveDate> {
    NaiveDate::from_ymd_opt(
        i32::from(date.year()),
        u32::try_from(date.month()).ok()?,
        u32::try_from(date.day()).ok()?,
    )
}

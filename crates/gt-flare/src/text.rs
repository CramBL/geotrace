//! The wording the plot marker, its chip, and the settings page share.
//!
//! All of them describe the same events and must describe them the same way:
//! a flare is an event on the Sun, not a measurement at the receiver. Shared
//! here so they cannot drift into three levels of confidence.
//!
//! A snapshot test pins the exact wording.

use std::sync::LazyLock;

use chrono::{DateTime, Utc};
use gt_fmt::UTC_MINUTE_FORMAT;
use gt_types::SunlitSide;
use gt_ui_types::MetricChipHover;

use crate::class::RadioBlackoutClass;
use crate::flare::{MarkedFlare, SolarFlare};
use crate::reference::SOLAR_FLARES;

/// Name of the data everywhere it is offered: the plot chip, the settings
/// page, the legend.
pub const LAYER_LABEL: &str = "Solar flares";

/// The standing caveat, shown wherever a flare is. Never abbreviated, even
/// when another surface already said it.
pub const SOURCE_CAVEAT: &str = "Lists every flare on the Sun. Only one on the receiver's sunlit \
                                 side raises the ionization above it.";

/// Shown for a flare whose peak flux stays under the radio blackout scale's
/// first level.
pub static BELOW_BLACKOUT_SCALE: LazyLock<String> =
    LazyLock::new(|| format!("Below {}", RadioBlackoutClass::Minor.scale_name()));

/// Closes the time range of a flare the catalog published no end time for.
pub const NO_END_TIME: &str = "end not published";

/// Which side of Earth the receiver was on when the flare peaked, shown once
/// a loaded recording places it.
pub const RECEIVER_SUNLIT: &str = "Receiver: sunlit side";
pub const RECEIVER_NIGHT: &str = "Receiver: night side";

/// How the flare hover writes an hour and a minute of the peak's own UTC day.
const HOUR_MINUTE_FORMAT: &str = "%H:%M";

/// The lines describing one flare, leading with the classification that
/// places it on the scale and closing with where the receiver stood.
pub fn flare_summary(marked: &MarkedFlare) -> Vec<String> {
    let flare = &marked.flare;
    let mut lines = vec![
        format!("{} solar flare", flare.classification),
        flare.classification.radio_blackout_class().map_or_else(
            || BELOW_BLACKOUT_SCALE.clone(),
            |class| class.display_name().to_owned(),
        ),
        format!("Peak: {} UTC", flare.peak.format(UTC_MINUTE_FORMAT)),
        flare.time_range_line(),
    ];
    if let Some(origin) = flare.origin_line() {
        lines.push(origin);
    }
    if let Some(side) = marked.receiver_side {
        lines.push(
            match side {
                SunlitSide::Sunlit => RECEIVER_SUNLIT,
                SunlitSide::Night => RECEIVER_NIGHT,
            }
            .to_owned(),
        );
    }
    lines
}

impl SolarFlare {
    /// The stretch from the flare's begin to its end, closed by
    /// [`NO_END_TIME`] where the catalog published no end.
    ///
    /// A stretch inside the peak's own UTC day is written in hours and
    /// minutes, joined by an en dash with no spaces around it. One reaching
    /// another day includes the date on both sides, and the dash is spaced.
    fn time_range_line(&self) -> String {
        let on_the_peaks_day =
            |instant: DateTime<Utc>| instant.date_naive() == self.peak.date_naive();
        let Some(end) = self.end else {
            let begin = if on_the_peaks_day(self.begin) {
                self.begin.format(HOUR_MINUTE_FORMAT)
            } else {
                self.begin.format(UTC_MINUTE_FORMAT)
            };
            return format!("{begin} – {NO_END_TIME}");
        };
        if on_the_peaks_day(self.begin) && on_the_peaks_day(end) {
            return format!(
                "{}–{} UTC",
                self.begin.format(HOUR_MINUTE_FORMAT),
                end.format(HOUR_MINUTE_FORMAT)
            );
        }
        format!(
            "{} – {} UTC",
            self.begin.format(UTC_MINUTE_FORMAT),
            end.format(UTC_MINUTE_FORMAT)
        )
    }

    /// Where on the Sun the flare came from, or [`None`] when the catalog
    /// gave neither the region nor the location.
    fn origin_line(&self) -> Option<String> {
        match (self.active_region, self.source_location.as_deref()) {
            (Some(region), Some(location)) => Some(format!("AR {region}, {location}")),
            (Some(region), None) => Some(format!("AR {region}")),
            (None, Some(location)) => Some(location.to_owned()),
            (None, None) => None,
        }
    }
}

pub static PLOT_HOVER: LazyLock<MetricChipHover> = LazyLock::new(|| MetricChipHover {
    definition: format!("Solar flares from the {SOURCE_NAME} catalog, marked at their peak."),
    source_cadence_and_scale: "One vertical line at each peak, coloured by class, X ten times M."
        .to_owned(),
    reference: SOLAR_FLARES,
});

/// The events GeoTrace downloads, as every control offering them names them.
pub const EVENT_NAMES: &str = "solar flare events";

/// The local archive of downloaded days, as the download controls name it.
pub const ARCHIVE_NAME: &str = "solar flare archive";

/// Name of the catalog the events come from.
pub const SOURCE_NAME: &str = "NASA DONKI";

/// Where the data comes from, shown in the legend and the about dialog.
pub static ATTRIBUTION: LazyLock<String> =
    LazyLock::new(|| format!("Solar flare events from the {SOURCE_NAME} catalog."));

/// Home page of the catalog, linked from the attribution.
pub const PUBLISHER_URL: &str = "https://ccmc.gsfc.nasa.gov/tools/DONKI/";

/// Where a user registers for the key the endpoint needs.
pub const KEY_SIGNUP_URL: &str = "https://api.nasa.gov";

/// Shown wherever a control is disabled for want of a key.
pub static MISSING_KEY: LazyLock<String> = LazyLock::new(|| {
    format!(
        "The {SOURCE_NAME} endpoint needs an API key of your own. Without one GeoTrace requests \
         no flares. Registering at {KEY_SIGNUP_URL} is free and the key arrives by email."
    )
});

#[cfg(test)]
mod tests {
    use chrono::{DateTime, Utc};
    use rstest::rstest;

    use super::*;
    use crate::wire::parse_flare_time;

    fn time(text: &str) -> DateTime<Utc> {
        parse_flare_time(text).expect("a catalog time")
    }

    /// The May 2024 X2.2, with every field the catalog gave it.
    fn storm_flare() -> SolarFlare {
        SolarFlare {
            id: "2024-05-09T08:45:00-FLR-001".to_owned(),
            begin: time("2024-05-09T08:45Z"),
            peak: time("2024-05-09T09:13Z"),
            end: Some(time("2024-05-09T09:36Z")),
            classification: "X2.2".parse().expect("a published class"),
            source_location: Some("S20W25".to_owned()),
            active_region: Some(13664),
        }
    }

    /// The storm flare with no recording loaded to place the receiver.
    fn unplaced(flare: SolarFlare) -> MarkedFlare {
        MarkedFlare {
            flare,
            receiver_side: None,
        }
    }

    #[test]
    fn a_flare_summary_leads_with_the_classification() {
        let lines = flare_summary(&unplaced(storm_flare()));
        insta::assert_debug_snapshot!("flare_summary", lines);
    }

    /// A flare the catalog left open, from a region it never numbered.
    #[test]
    fn a_flare_summary_states_what_the_catalog_left_out() {
        let flare = SolarFlare {
            end: None,
            source_location: None,
            active_region: None,
            classification: "C5.0".parse().expect("a published class"),
            ..storm_flare()
        };
        let lines = flare_summary(&unplaced(flare));
        insta::assert_debug_snapshot!("flare_summary_without_an_end", lines);
    }

    /// The receiver's side closes the summary, and is stated only for a flare
    /// a loaded recording places the receiver at.
    #[rstest]
    #[case::sunlit(Some(SunlitSide::Sunlit), RECEIVER_SUNLIT)]
    #[case::night(Some(SunlitSide::Night), RECEIVER_NIGHT)]
    #[case::no_recording_loaded(None, "AR 13664, S20W25")]
    fn a_flare_summary_closes_on_the_side_the_receiver_was_on(
        #[case] receiver_side: Option<SunlitSide>,
        #[case] expected_last_line: &str,
    ) {
        let marked = MarkedFlare {
            flare: storm_flare(),
            receiver_side,
        };
        let lines = flare_summary(&marked);
        assert_eq!(lines.last().map(String::as_str), Some(expected_last_line));
    }

    /// The forms of the time range: one stretch inside the peak's own UTC
    /// day, one reaching the next day, and one the catalog left open, whose
    /// begin takes the date on any day but the peak's.
    #[rstest]
    #[case::inside_the_peaks_day(
        "2024-05-09T14:10Z",
        "2024-05-09T14:32Z",
        Some("2024-05-09T14:58Z"),
        "14:10–14:58 UTC"
    )]
    #[case::reaching_the_next_day(
        "2024-05-10T23:50Z",
        "2024-05-11T00:05Z",
        Some("2024-05-11T00:20Z"),
        "2024-05-10 23:50 – 2024-05-11 00:20 UTC"
    )]
    #[case::left_open(
        "2024-05-09T14:10Z",
        "2024-05-09T14:32Z",
        None,
        "14:10 – end not published"
    )]
    #[case::left_open_from_the_day_before(
        "2024-05-08T23:40Z",
        "2024-05-09T00:05Z",
        None,
        "2024-05-08 23:40 – end not published"
    )]
    fn the_time_range_states_the_stretch_the_catalog_published(
        #[case] begin: &str,
        #[case] peak: &str,
        #[case] end: Option<&str>,
        #[case] expected: &str,
    ) {
        let flare = SolarFlare {
            begin: time(begin),
            peak: time(peak),
            end: end.map(time),
            ..storm_flare()
        };

        assert_eq!(flare.time_range_line(), expected);
    }

    /// A location without a region number still says where the flare came
    /// from.
    #[test]
    fn a_flare_summary_names_a_location_without_a_region() {
        let flare = SolarFlare {
            active_region: None,
            ..storm_flare()
        };
        assert_eq!(flare.origin_line().as_deref(), Some("S20W25"));
    }

    /// The feature's user-visible wording, in one place.
    #[test]
    fn shared_wording() {
        let wording = format!(
            "label: {LAYER_LABEL}\n\
             events: {EVENT_NAMES}\n\
             archive: {ARCHIVE_NAME}\n\
             source caveat: {SOURCE_CAVEAT}\n\
             below the scale: {}\n\
             no end time: {NO_END_TIME}\n\
             receiver sunlit: {RECEIVER_SUNLIT}\n\
             receiver on the night side: {RECEIVER_NIGHT}\n\
             plot hover: {}\n\
             missing key: {}\n\
             attribution: {}\n\
             publisher: {PUBLISHER_URL}\n\
             key signup: {KEY_SIGNUP_URL}",
            *BELOW_BLACKOUT_SCALE, *PLOT_HOVER, *MISSING_KEY, *ATTRIBUTION
        );
        insta::assert_snapshot!("shared_wording", wording);
    }
}

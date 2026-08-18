//! The wording the plot marker, its chip, and the settings page share.
//!
//! All of them describe the same events and must describe them the same way:
//! a flare is an event on the Sun, not a measurement at the receiver. Shared
//! here so they cannot drift into three levels of confidence.
//!
//! A snapshot test pins the exact wording.

use std::sync::LazyLock;

use crate::class::RadioBlackoutClass;
use crate::flare::SolarFlare;

/// Name of the data everywhere it is offered: the plot chip, the settings
/// page, the legend.
pub const LAYER_LABEL: &str = "Solar flares";

/// One-line description of what a marker is, for hover text on the chip.
pub const LAYER_SUMMARY: &str = "Solar flares of the days in the archive, marked at their peak.";

/// The standing caveat, shown wherever a flare is. Never abbreviated, even
/// when another surface already said it.
pub const SOURCE_CAVEAT: &str = "The catalog lists every flare the Sun produced, so a flare \
                                 raises the ionization above a receiver only where the Sun was \
                                 up at the time.";

/// What the scale means, shown alongside the first flare on a surface.
pub const SCALE_CAVEAT: &str = "Each class letter is ten times the peak X-ray flux of the one \
                                before, so an X1 is ten times an M1. NOAA counts a radio \
                                blackout from M1 upwards.";

/// Shown for a flare below the radio blackout scale, which starts at M1.
pub const BELOW_BLACKOUT_SCALE: &str = "Below the radio blackout scale";

/// Shown for a flare the catalog published no end time for.
pub const NO_END_TIME: &str = "no end published";

/// The three times of one flare, formatted by the surface showing them so
/// every hover reads the same way.
#[derive(Debug, Clone, Copy)]
pub struct FormattedFlareTimes<'a> {
    pub begin: &'a str,
    pub peak: &'a str,
    /// [`None`] for a flare the catalog left open.
    pub end: Option<&'a str>,
}

/// The lines describing one flare, leading with the classification that
/// places it on the scale.
pub fn flare_summary(flare: &SolarFlare, times: FormattedFlareTimes<'_>) -> Vec<String> {
    let mut lines = vec![
        format!("{} solar flare", flare.classification),
        flare
            .classification
            .radio_blackout_class()
            .map_or(BELOW_BLACKOUT_SCALE, RadioBlackoutClass::display_name)
            .to_owned(),
        format!("Peaked at {} (UTC)", times.peak),
        match times.end {
            Some(end) => format!("Began {}, ended {end}", times.begin),
            None => format!("Began {}, {NO_END_TIME}", times.begin),
        },
    ];
    if let Some(origin) = flare_origin(flare) {
        lines.push(origin);
    }
    lines
}

/// Where on the Sun the flare came from, or [`None`] when the catalog gave
/// neither the region nor the location.
fn flare_origin(flare: &SolarFlare) -> Option<String> {
    match (flare.active_region, flare.source_location.as_deref()) {
        (Some(region), Some(location)) => Some(format!("Active region {region} at {location}")),
        (Some(region), None) => Some(format!("Active region {region}")),
        (None, Some(location)) => Some(format!("Source location {location}")),
        (None, None) => None,
    }
}

/// The plot chip's hover text, composed from the shared caveats so it cannot
/// drift from what the marker hover says.
pub static PLOT_HOVER: LazyLock<String> = LazyLock::new(|| {
    format!(
        "{LAYER_SUMMARY} Each flare is one vertical line at its peak, coloured by class. \
         {SOURCE_CAVEAT} {SCALE_CAVEAT}"
    )
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

    fn times<'a>(begin: &'a str, peak: &'a str, end: Option<&'a str>) -> FormattedFlareTimes<'a> {
        FormattedFlareTimes { begin, peak, end }
    }

    #[test]
    fn a_flare_summary_leads_with_the_classification() {
        let lines = flare_summary(&storm_flare(), times("08:45", "09:13", Some("09:36")));
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
        let lines = flare_summary(&flare, times("13:13", "13:22", None));
        insta::assert_debug_snapshot!("flare_summary_without_an_end", lines);
    }

    /// A location without a region number still says where the flare came
    /// from.
    #[test]
    fn a_flare_summary_names_a_location_without_a_region() {
        let flare = SolarFlare {
            active_region: None,
            ..storm_flare()
        };
        assert_eq!(
            flare_origin(&flare).as_deref(),
            Some("Source location S20W25")
        );
    }

    /// The feature's user-visible wording, in one place.
    #[test]
    fn shared_wording() {
        let wording = format!(
            "label: {LAYER_LABEL}\n\
             events: {EVENT_NAMES}\n\
             archive: {ARCHIVE_NAME}\n\
             summary: {LAYER_SUMMARY}\n\
             source caveat: {SOURCE_CAVEAT}\n\
             scale caveat: {SCALE_CAVEAT}\n\
             below the scale: {BELOW_BLACKOUT_SCALE}\n\
             no end time: {NO_END_TIME}\n\
             plot hover: {}\n\
             missing key: {}\n\
             attribution: {}\n\
             publisher: {PUBLISHER_URL}\n\
             key signup: {KEY_SIGNUP_URL}",
            *PLOT_HOVER, *MISSING_KEY, *ATTRIBUTION
        );
        insta::assert_snapshot!("shared_wording", wording);
    }
}

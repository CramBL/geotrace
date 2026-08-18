//! The wording the plot line, the download controls, the settings page and
//! the query metric share.
//!
//! Every surface shows the same modelled value and must describe it the same
//! way: vertical content over a region, not the delay one receiver measured.
//! Shared here so they cannot drift into separate levels of confidence.
//!
//! A snapshot test pins the exact wording.

use std::sync::LazyLock;

use crate::tec::{L1_DELAY_METERS_PER_TECU, TotalElectronContent};

/// The maps GeoTrace downloads, as every control offering them names them.
pub const MAP_NAMES: &str = "global ionosphere maps";

/// The local archive of downloaded days, as the download controls name it.
pub const ARCHIVE_NAME: &str = "TEC map archive";

/// Name of the data everywhere it is offered: the plot chip, the hover label.
pub const LAYER_LABEL: &str = "Ionospheric TEC";

/// The standing caveat, shown wherever a value is. Never abbreviated, even
/// when another surface already said it.
pub const SOURCE_CAVEAT: &str = "Modelled from ground stations onto a global grid and published \
                                 as vertical content on a 450 km shell, so a value describes the \
                                 ionosphere over a region, not the slant path one receiver saw.";

/// What the unit means, shown alongside the first value on a surface.
pub static SCALE_CAVEAT: LazyLock<String> = LazyLock::new(|| {
    format!(
        "One TEC unit is 10^16 electrons per square metre and adds about \
         {L1_DELAY_METERS_PER_TECU:.2} m of range on L1. A quiet mid-latitude day stays under 20 \
         TECU, and a storm reaches past 150."
    )
});

/// The lines describing one value, leading with the value itself. `instant`
/// is the formatted UTC time it was interpolated at.
pub fn value_summary(content: TotalElectronContent, instant: &str) -> Vec<String> {
    vec![
        format!("TEC {:.1} TECU", content.tecu()),
        format!("L1 delay about {:.1} m", content.l1_delay_meters()),
        format!("Interpolated between maps at {instant} (UTC)"),
    ]
}

/// The plot chip's hover text, composed from the shared caveats so it cannot
/// drift from what the hover label says.
pub static PLOT_HOVER: LazyLock<String> = LazyLock::new(|| {
    format!(
        "Vertical total electron content across the span the plot shows, interpolated from the \
         archived {MAP_NAMES} at the position of the fix nearest each map epoch in time. \
         {SOURCE_CAVEAT} {} The line breaks over days no maps are archived for.",
        *SCALE_CAVEAT
    )
});

/// The query metric's documentation body.
pub static QUERY_DOC: LazyLock<String> = LazyLock::new(|| {
    format!(
        "Vertical total electron content over the fix's own position and UTC time, in TEC units, \
         interpolated from the archived {MAP_NAMES}. {SOURCE_CAVEAT} {} Fixes whose day is not in \
         the {ARCHIVE_NAME} carry no value.",
        *SCALE_CAVEAT
    )
});

/// Hover text of the settings page's fetch queue row, stating what one
/// request covers.
pub const FETCH_QUEUE_HOVER: &str = "Map days waiting to be downloaded. One day is requested at a \
                                     time, and a day costs one request per mirror tried until one \
                                     serves the file.";

/// Hover text of the settings page's recording day row, stating that a
/// backfilled day is outside the count.
pub const RECORDING_DAY_COVERAGE_HOVER: &str = "UTC days the recordings loaded this session span, \
                                                and how many of them the archive holds maps for. \
                                                Days downloaded by a backfill are not counted \
                                                here.";

#[cfg(test)]
mod tests {
    use super::*;

    /// One value's hover lines: the value, the range it delays L1 by, then
    /// the instant it was interpolated at.
    #[test]
    fn value_summary_leads_with_the_value() {
        let lines = value_summary(TotalElectronContent::from_tecu(42.3), "2024-05-10T18:30:00");
        assert_eq!(
            lines,
            [
                "TEC 42.3 TECU",
                "L1 delay about 6.9 m",
                "Interpolated between maps at 2024-05-10T18:30:00 (UTC)",
            ]
        );
    }

    /// The feature's user-visible wording, in one place.
    #[test]
    fn shared_wording() {
        let wording = format!(
            "label: {LAYER_LABEL}\n\
             maps: {MAP_NAMES}\n\
             archive: {ARCHIVE_NAME}\n\
             source caveat: {SOURCE_CAVEAT}\n\
             scale caveat: {}\n\
             plot hover: {}\n\
             query doc: {}\n\
             fetch queue hover: {FETCH_QUEUE_HOVER}\n\
             recording day coverage hover: {RECORDING_DAY_COVERAGE_HOVER}",
            *SCALE_CAVEAT, *PLOT_HOVER, *QUERY_DOC
        );
        insta::assert_snapshot!("shared_wording", wording);
    }
}

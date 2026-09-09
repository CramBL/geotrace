//! The wording the map overlay, the plot line, the display toggle, and the
//! settings page share.
//!
//! The surfaces showing values must describe them the same way: aircraft
//! reports over a whole UTC day and a cell tens of kilometers across, not a
//! measurement at the receiver. Shared here so they cannot drift into
//! separate levels of confidence.
//!
//! A snapshot test pins the exact wording.

use std::sync::LazyLock;

use gt_ui_types::MetricChipHover;

use crate::reference::AIRCRAFT_INTERFERENCE;

/// Name of the data everywhere it is offered: the display-toggle row, the
/// plot line, the legend.
///
/// Says "aircraft", not "jamming": the dataset cannot attribute a cause.
pub const LAYER_LABEL: &str = "Aircraft interference";

/// One-line description of what a value is, for hover text on the toggle
/// row and the plot line.
pub const LAYER_SUMMARY: &str = "Share of aircraft over this area that reported low navigation \
                                 accuracy, over one UTC day.";

/// The standing caveat, shown wherever a value is. Never abbreviated, even
/// when another surface already said it.
pub const SOURCE_CAVEAT: &str = "Reported by aircraft in flight, not measured on the ground - a \
                                 track under an affected cell is not necessarily affected, and a \
                                 clear cell is not a guarantee.";

/// What one cell and one value cover.
pub const RESOLUTION_CAVEAT: &str = "One cell spans roughly 22km and one value covers a 24h UTC \
                                     day, so neither the minute nor the kilometer of an event can \
                                     be read from this.";

/// Shown for cells whose aircraft count is too small for the share to mean
/// anything, alongside their hatched fill on the map.
pub const LOW_SAMPLE_CAVEAT: &str =
    "Too few aircraft passed through this cell for the share to carry weight.";

/// The lines describing one cell, leading with the counts that produced the
/// share. Shared so the map hover and the plot hover agree.
pub fn cell_summary(day: &str, good: u32, bad: u32, bad_percent: f64) -> Vec<String> {
    vec![
        format!(
            "{bad} of {} aircraft reported low navigation accuracy",
            good.saturating_add(bad)
        ),
        format!("{bad_percent:.1}% over {day} (UTC)"),
    ]
}

/// The query metric's documentation body, composed from the shared caveats.
pub static QUERY_DOC: LazyLock<String> = LazyLock::new(|| {
    format!(
        "Share of aircraft over the fix's cell that reported low navigation accuracy, in percent, \
         for the fix's own UTC day. {SOURCE_CAVEAT} {RESOLUTION_CAVEAT} Fixes whose day is not in \
         the interference archive carry no value."
    )
});

pub static PLOT_HOVER: LazyLock<MetricChipHover> = LazyLock::new(|| MetricChipHover {
    definition: "Share of aircraft over the fix's cell that reported low navigation accuracy."
        .to_owned(),
    source_cadence_and_scale: "gpsjam.org over ADS-B Exchange, one value per UTC day and 22km \
                               cell, percent."
        .to_owned(),
    reference: AIRCRAFT_INTERFERENCE,
});

/// Hover text of the settings page's fetch queue row, stating what one
/// request covers.
pub const FETCH_QUEUE_HOVER: &str = "Interference days waiting to be downloaded. One day is \
                                     requested at a time, and one day costs one request.";

/// Hover text of the settings page's recording day row, stating that a
/// backfilled day is outside the count.
pub const RECORDING_DAY_COVERAGE_HOVER: &str = "UTC days the recordings loaded this session span, \
                                                and how many of them the archive holds a dataset \
                                                for. Days downloaded by a backfill are not \
                                                counted here.";

/// Where the data comes from, shown in the legend and the about dialog.
pub const ATTRIBUTION: &str = "Interference data from gpsjam.org, derived from aircraft reports \
                               collected by adsbexchange.com.";

/// Home page of the dataset's publisher, linked from the attribution.
pub const PUBLISHER_URL: &str = "https://gpsjam.org";

/// Home page of the upstream aircraft-report collector, linked from the
/// attribution.
pub const UPSTREAM_URL: &str = "https://adsbexchange.com";

#[cfg(test)]
mod tests {
    use super::*;

    /// One cell's hover lines: the counts, then the share.
    #[test]
    fn cell_summary_leads_with_the_counts() {
        let lines = cell_summary("2026-07-20", 412, 3, 0.72);
        insta::assert_debug_snapshot!("cell_summary", lines);
    }

    /// A cell with no good aircraft still states a count, not a bare
    /// percentage.
    #[test]
    fn cell_summary_handles_an_all_bad_cell() {
        let lines = cell_summary("2026-07-20", 0, 4, 100.0);
        assert_eq!(
            lines.first().map(String::as_str),
            Some("4 of 4 aircraft reported low navigation accuracy")
        );
    }

    /// The feature's user-visible wording, in one place.
    #[test]
    fn shared_wording() {
        let wording = format!(
            "label: {LAYER_LABEL}\n\
             summary: {LAYER_SUMMARY}\n\
             source caveat: {SOURCE_CAVEAT}\n\
             resolution caveat: {RESOLUTION_CAVEAT}\n\
             low sample caveat: {LOW_SAMPLE_CAVEAT}\n\
             fetch queue hover: {FETCH_QUEUE_HOVER}\n\
             recording day coverage hover: {RECORDING_DAY_COVERAGE_HOVER}\n\
             plot hover: {}\n\
             query doc: {}\n\
             attribution: {ATTRIBUTION}\n\
             publisher: {PUBLISHER_URL}\n\
             upstream: {UPSTREAM_URL}",
            *PLOT_HOVER, *QUERY_DOC
        );
        insta::assert_snapshot!("shared_wording", wording);
    }

    #[test]
    fn label_names_the_reporter_not_a_cause() {
        assert!(LAYER_LABEL.contains("Aircraft"));
        assert!(!LAYER_LABEL.to_lowercase().contains("jamming"));
    }
}

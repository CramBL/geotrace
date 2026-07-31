//! The wording the map overlay, the plot line, and the display toggle share.
//!
//! All three show the same numbers and must describe them the same way:
//! aircraft reports over a whole UTC day and a cell tens of kilometers
//! across, not a measurement at the receiver. Shared here so they cannot
//! drift into three levels of confidence.
//!
//! A snapshot test pins the exact wording.

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
pub const RESOLUTION_CAVEAT: &str = "One cell spans roughly 22 km and one value covers a full \
                                     24-hour UTC day, so neither the minute nor the kilometer of \
                                     an event can be read from this.";

/// Shown for cells whose aircraft count is too small for the share to mean
/// anything, alongside their hatched fill on the map.
pub const LOW_SAMPLE_CAVEAT: &str =
    "Too few aircraft passed through this cell for the share to carry weight.";

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

    /// The feature's user-visible wording, in one place.
    #[test]
    fn shared_wording() {
        let wording = format!(
            "label: {LAYER_LABEL}\n\
             summary: {LAYER_SUMMARY}\n\
             source caveat: {SOURCE_CAVEAT}\n\
             resolution caveat: {RESOLUTION_CAVEAT}\n\
             low sample caveat: {LOW_SAMPLE_CAVEAT}\n\
             attribution: {ATTRIBUTION}\n\
             publisher: {PUBLISHER_URL}\n\
             upstream: {UPSTREAM_URL}"
        );
        insta::assert_snapshot!("shared_wording", wording);
    }

    #[test]
    fn label_names_the_reporter_not_a_cause() {
        assert!(LAYER_LABEL.contains("Aircraft"));
        assert!(!LAYER_LABEL.to_lowercase().contains("jamming"));
    }
}

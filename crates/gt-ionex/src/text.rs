//! The wording the download controls and the settings page share.
//!
//! A snapshot test pins the exact wording.

/// The maps GeoTrace downloads, as every control offering them names them.
pub const MAP_NAMES: &str = "global ionosphere maps";

/// The local archive of downloaded days, as the download controls name it.
pub const ARCHIVE_NAME: &str = "TEC map archive";

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

    /// The feature's user-visible wording, in one place.
    #[test]
    fn shared_wording() {
        let wording = format!(
            "maps: {MAP_NAMES}\n\
             archive: {ARCHIVE_NAME}\n\
             fetch queue hover: {FETCH_QUEUE_HOVER}\n\
             recording day coverage hover: {RECORDING_DAY_COVERAGE_HOVER}"
        );
        insta::assert_snapshot!("shared_wording", wording);
    }
}

//! The wording the plot line, the map heatmap, the download controls, the
//! settings page and the query metric share.
//!
//! Every surface shows the same modelled value and must describe it the same
//! way: vertical content over a region, not the delay one receiver measured.
//! Shared here so they cannot drift into separate levels of confidence.
//!
//! A snapshot test pins the exact wording.

use std::sync::LazyLock;

use gt_types::{Latitude, Longitude};
use gt_ui_types::MetricChipHover;

use crate::reference::IONOSPHERIC_TEC;
use crate::tec::{L1_DELAY_METERS_PER_TECU, TotalElectronContent};

/// The maps GeoTrace downloads, as every control offering them names them.
pub const MAP_NAMES: &str = "global ionosphere maps";

/// The local archive of downloaded days, as the download controls name it.
pub const ARCHIVE_NAME: &str = "TEC map archive";

/// Name of the data everywhere it is offered: the plot chip, the hover label,
/// the map layer's display-toggle row.
pub const LAYER_LABEL: &str = "Ionospheric TEC";

/// One-line description of what the map layer draws, for hover text on the
/// display-toggle row.
pub const LAYER_SUMMARY: &str = "Colours the published grid by vertical total electron content at \
                                 the instant shown, under the tracks.";

/// Format the map heatmap writes the instant it shows in, matching the epochs
/// the archived files declare.
pub const INSTANT_FORMAT: &str = "%Y-%m-%dT%H:%M:%S";

/// Unit the legend labels its scale in.
pub const LEGEND_UNIT: &str = "TECU";

/// Hover text of the heatmap's opacity control.
pub const OPACITY_HOVER: &str = "How strongly the heatmap is drawn over the base map";

/// Hover text of the instant the heatmap draws.
pub const INSTANT_HOVER: &str = "The map epoch the heatmap draws";

/// Hover text of the instant stepper while a hovered or selected fix is
/// driving the instant.
pub const FOLLOWING_A_FIX: &str = "The heatmap follows the hovered or selected fix. Deselect it to \
                                   step map epochs.";

/// Hover text of the instant stepper while the layer is hidden.
pub const HIDDEN_LAYER_STEPPER: &str = "Show the TEC heatmap to step map epochs";

/// Hover text of the opacity control while the layer is hidden.
pub const HIDDEN_LAYER_OPACITY: &str = "Show the TEC heatmap to change its opacity";

/// The standing caveat, shown wherever a value is. Never abbreviated, even
/// when another surface already said it.
pub const SOURCE_CAVEAT: &str = "Modelled from ground stations onto a global grid and published \
                                 as vertical content on a 450 km shell, so a value describes the \
                                 ionosphere over a region, not the slant path one receiver saw.";

/// What the unit means, shown alongside the first value on a surface.
pub static SCALE_CAVEAT: LazyLock<String> = LazyLock::new(|| {
    format!(
        "One TEC unit is 10¹⁶ electrons per square metre and adds about \
         {L1_DELAY_METERS_PER_TECU:.2} m of range on L1. A quiet mid-latitude day stays under 20 \
         TECU, and a storm reaches past 150."
    )
});

/// The lines describing one value, leading with the value itself. `instant`
/// is the formatted UTC time it was interpolated at.
pub fn value_summary(content: TotalElectronContent, instant: &str) -> Vec<String> {
    let mut lines = value_lines(content);
    lines.push(format!("Interpolated between maps at {instant} (UTC)"));
    lines
}

/// The lines describing one grid node of the map heatmap, leading with the
/// value. `instant` is the formatted UTC time the two bracketing maps were
/// interpolated to.
pub fn grid_node_summary(
    content: TotalElectronContent,
    instant: &str,
    latitude: Latitude,
    longitude: Longitude,
) -> Vec<String> {
    let mut lines = value_lines(content);
    lines.push(format!(
        "Grid node {} at {instant} (UTC)",
        node_position(latitude, longitude)
    ));
    lines
}

/// The value and the range it delays L1 by, which every surface leads with.
fn value_lines(content: TotalElectronContent) -> Vec<String> {
    vec![
        format!("TEC {:.1} TECU", content.tecu()),
        format!("L1 delay about {:.1} m", content.l1_delay_meters()),
    ]
}

/// One node's position, in the hemisphere form the hover writes.
fn node_position(latitude: Latitude, longitude: Longitude) -> String {
    let degrees_north = latitude.as_degrees();
    let degrees_east = longitude.as_degrees();
    let north_south = if degrees_north < 0.0 { 'S' } else { 'N' };
    let east_west = if degrees_east < 0.0 { 'W' } else { 'E' };
    format!(
        "{:.1} {north_south}, {:.1} {east_west}",
        degrees_north.abs(),
        degrees_east.abs()
    )
}

pub static PLOT_HOVER: LazyLock<MetricChipHover> = LazyLock::new(|| MetricChipHover {
    definition: "Total electron content of the ionosphere above the fix's position.".to_owned(),
    source_cadence_and_scale: "NASA JPL, maps every 1 to 2 h, vertical content in TECU.".to_owned(),
    reference: IONOSPHERIC_TEC,
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

/// Where a user registers for the token the CDDIS archive needs.
pub const EARTHDATA_SIGNUP_URL: &str = "https://urs.earthdata.nasa.gov";

/// Why a mirror was passed over, as the download failures list it.
pub const MIRROR_SKIPPED_WITHOUT_TOKEN: &str = "no Earthdata token set";

/// Shown wherever a mirror is held back for want of a token.
pub static MISSING_EARTHDATA_TOKEN: LazyLock<String> = LazyLock::new(|| {
    format!(
        "GeoTrace skips this mirror until an Earthdata token is set: CDDIS serves the maps to \
         registered callers only. Registering at {EARTHDATA_SIGNUP_URL} is free."
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

    /// A node's hover lines name the value, the delay, and where the node
    /// sits, with the hemisphere letters of a southwestern position.
    #[test]
    fn grid_node_summary_names_the_node_it_read() {
        let lines = grid_node_summary(
            TotalElectronContent::from_tecu(175.4),
            "2024-05-10T18:00:00",
            Latitude::new(-12.5),
            Longitude::new(-75.0),
        );
        assert_eq!(
            lines,
            [
                "TEC 175.4 TECU",
                "L1 delay about 28.5 m",
                "Grid node 12.5 S, 75.0 W at 2024-05-10T18:00:00 (UTC)",
            ]
        );
    }

    /// The feature's user-visible wording, in one place.
    #[test]
    fn shared_wording() {
        let wording = format!(
            "label: {LAYER_LABEL}\n\
             layer summary: {LAYER_SUMMARY}\n\
             maps: {MAP_NAMES}\n\
             archive: {ARCHIVE_NAME}\n\
             source caveat: {SOURCE_CAVEAT}\n\
             scale caveat: {}\n\
             plot hover: {}\n\
             query doc: {}\n\
             legend unit: {LEGEND_UNIT}\n\
             instant hover: {INSTANT_HOVER}\n\
             opacity hover: {OPACITY_HOVER}\n\
             following a fix: {FOLLOWING_A_FIX}\n\
             hidden layer stepper: {HIDDEN_LAYER_STEPPER}\n\
             hidden layer opacity: {HIDDEN_LAYER_OPACITY}\n\
             fetch queue hover: {FETCH_QUEUE_HOVER}\n\
             recording day coverage hover: {RECORDING_DAY_COVERAGE_HOVER}\n\
             mirror skipped without a token: {MIRROR_SKIPPED_WITHOUT_TOKEN}\n\
             missing Earthdata token: {}\n\
             Earthdata signup: {EARTHDATA_SIGNUP_URL}",
            *SCALE_CAVEAT, *PLOT_HOVER, *QUERY_DOC, *MISSING_EARTHDATA_TOKEN
        );
        insta::assert_snapshot!("shared_wording", wording);
    }
}

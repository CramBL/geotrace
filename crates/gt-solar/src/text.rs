//! The wording the plot line, the side panel, and the query metric share.
//!
//! All of them show the same index values and must describe them the same
//! way: a planetary average over a period, not a measurement at the receiver.
//! Shared here so they cannot drift into three levels of confidence.
//!
//! A snapshot test pins the exact wording.

use std::sync::LazyLock;

use crate::GeomagneticIndex;
use crate::activity::GeomagneticActivity;

/// Name of the data everywhere it is offered: the display-toggle row, the
/// plot line, the legend.
pub const LAYER_LABEL: &str = "Geomagnetic activity";

/// One-line description of what a value is, for hover text on the toggle row
/// and the plot line.
pub const LAYER_SUMMARY: &str =
    "Planetary geomagnetic activity index over the time each fix was recorded.";

/// The standing caveat, shown wherever a value is. Never abbreviated, even
/// when another surface already said it.
pub const SOURCE_CAVEAT: &str = "Averaged over magnetometer stations worldwide, so a value \
                                 describes the planet over a period, not the ionosphere above \
                                 one receiver.";

/// What the scale means, shown alongside the first value on a surface.
pub const SCALE_CAVEAT: &str = "The scale is quasi logarithmic and published in thirds of a \
                                unit. Storm levels start at 5 (G1), Kp stops at 9, and Hp30 \
                                keeps climbing past it in an extreme storm.";

/// Shown for a period the service published no value for.
pub const NO_VALUE_CAVEAT: &str = "No value was published for this period.";

/// The lines describing one period, leading with the value that classified
/// it. `period_start` is the formatted UTC time the period begins at.
pub fn period_summary(
    index: GeomagneticIndex,
    activity: Option<GeomagneticActivity>,
    period_start: &str,
) -> Vec<String> {
    let mut lines = match activity {
        Some(activity) => vec![
            format!("{index} {activity}"),
            activity.class().display_name().to_owned(),
        ],
        None => vec![format!("{index} not published"), NO_VALUE_CAVEAT.to_owned()],
    };
    lines.push(format!(
        "{} from {period_start} (UTC)",
        index.period_length_words()
    ));
    lines
}

/// Name of the service the values come from, as it names itself in every
/// response.
pub const SOURCE_NAME: &str = "GFZ Potsdam";

/// License the values are published under, as named in every response.
pub const LICENSE_NAME: &str = "CC BY 4.0";

/// Where the data comes from, shown in the legend and the about dialog.
pub static ATTRIBUTION: LazyLock<String> = LazyLock::new(|| {
    format!("Geomagnetic index data from {SOURCE_NAME}, published under {LICENSE_NAME}.")
});

/// Home page of the index publisher, linked from the attribution.
pub const PUBLISHER_URL: &str = "https://kp.gfz.de";

/// The license the attribution links to.
pub const LICENSE_URL: &str = "https://creativecommons.org/licenses/by/4.0/";

#[cfg(test)]
mod tests {
    use crate::activity::GeomagneticStormClass;

    use super::*;

    fn kp(value: f64) -> Option<GeomagneticActivity> {
        GeomagneticActivity::from_published_value(GeomagneticIndex::Kp, value)
    }

    /// One period's hover lines: the value, its class, then what it covers.
    #[test]
    fn period_summary_leads_with_the_value() {
        let lines = period_summary(GeomagneticIndex::Kp, kp(8.667), "2024-05-10T18:00:00");
        insta::assert_debug_snapshot!("period_summary", lines);
    }

    #[test]
    fn period_summary_states_a_missing_value_as_such() {
        let lines = period_summary(GeomagneticIndex::Hp30, None, "1980-01-01T00:00:00");
        insta::assert_debug_snapshot!("period_summary_without_a_value", lines);
    }

    #[test]
    fn period_summary_names_the_class_of_a_quiet_period() {
        let lines = period_summary(GeomagneticIndex::Kp, kp(1.667), "2024-04-01T00:00:00");
        assert_eq!(lines.get(1).map(String::as_str), Some("Quiet"));
    }

    /// The class line is the one the storm class supplies.
    #[test]
    fn period_summary_names_the_storm_class() {
        let lines = period_summary(GeomagneticIndex::Kp, kp(9.0), "2024-05-11T00:00:00");
        assert_eq!(
            lines.get(1).map(String::as_str),
            Some(GeomagneticStormClass::Extreme.display_name())
        );
    }

    /// The feature's user-visible wording, in one place.
    #[test]
    fn shared_wording() {
        let wording = format!(
            "label: {LAYER_LABEL}\n\
             summary: {LAYER_SUMMARY}\n\
             source caveat: {SOURCE_CAVEAT}\n\
             scale caveat: {SCALE_CAVEAT}\n\
             no value caveat: {NO_VALUE_CAVEAT}\n\
             attribution: {}\n\
             publisher: {PUBLISHER_URL}\n\
             license: {LICENSE_URL}",
            *ATTRIBUTION
        );
        insta::assert_snapshot!("shared_wording", wording);
    }
}

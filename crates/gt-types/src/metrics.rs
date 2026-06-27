/// One variant per plot/telemetry metric.
///
/// Shared between the persisted UI settings (which key per-metric visibility
/// flags by this type, see `geotrace::settings::PlotSettings::metric`) and
/// the plot widget (which drives chip labels, colors, and series lookups off
/// the same variant set). The two previously carried independent copies of
/// this enum kept in sync by hand. A single definition here removes that
/// drift risk entirely.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    strum::EnumCount,
    strum::EnumIter,
    serde::Serialize,
    serde::Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum MetricKind {
    SatsSeen,
    SatsFix,
    GpsSeen,
    GpsFix,
    GlonassSeen,
    GlonassFix,
    GalileoSeen,
    GalileoFix,
    BeidouSeen,
    BeidouFix,
    NavicSeen,
    NavicFix,
    QzssSeen,
    QzssFix,
    Velocity,
    Eph,
    HeadingDeg,
    ClockDeltaMs,
    /// Satellite utilization rate across all constellations: share of in-view
    /// satellites (above the elevation mask) the receiver used in the fix.
    UtilAll,
    UtilGps,
    UtilGlonass,
    UtilGalileo,
    UtilBeidou,
    UtilNavic,
    UtilQzss,
    /// Loss-of-lock (cycle slip) rate per minute across all constellations, and
    /// broken down per constellation.  A slip is a satellite lost while still
    /// trackable above the mask, or a steep SNR drop between epochs.
    SlipAll,
    SlipGps,
    SlipGlonass,
    SlipGalileo,
    SlipBeidou,
    SlipNavic,
    SlipQzss,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;
    use serde::de::IntoDeserializer;
    use serde::de::value::{Error as DeError, StrDeserializer};
    use strum::EnumCount;

    /// Locks the on-disk spelling of every variant. `Settings` files persist
    /// these strings under `[plot.metric]`. A silent rename here would orphan
    /// every user's saved metric-visibility preferences on their next launch.
    ///
    /// Asserts the table is exhaustive (`expected.len() == MetricKind::COUNT`)
    /// so adding a variant without adding its wire-name entry fails here
    /// rather than silently leaving it unchecked.
    #[test]
    fn wire_names_are_stable() {
        let expected = [
            (MetricKind::SatsSeen, "sats_seen"),
            (MetricKind::SatsFix, "sats_fix"),
            (MetricKind::GpsSeen, "gps_seen"),
            (MetricKind::GpsFix, "gps_fix"),
            (MetricKind::GlonassSeen, "glonass_seen"),
            (MetricKind::GlonassFix, "glonass_fix"),
            (MetricKind::GalileoSeen, "galileo_seen"),
            (MetricKind::GalileoFix, "galileo_fix"),
            (MetricKind::BeidouSeen, "beidou_seen"),
            (MetricKind::BeidouFix, "beidou_fix"),
            (MetricKind::NavicSeen, "navic_seen"),
            (MetricKind::NavicFix, "navic_fix"),
            (MetricKind::QzssSeen, "qzss_seen"),
            (MetricKind::QzssFix, "qzss_fix"),
            (MetricKind::Velocity, "velocity"),
            (MetricKind::Eph, "eph"),
            (MetricKind::HeadingDeg, "heading_deg"),
            (MetricKind::ClockDeltaMs, "clock_delta_ms"),
            (MetricKind::UtilAll, "util_all"),
            (MetricKind::UtilGps, "util_gps"),
            (MetricKind::UtilGlonass, "util_glonass"),
            (MetricKind::UtilGalileo, "util_galileo"),
            (MetricKind::UtilBeidou, "util_beidou"),
            (MetricKind::UtilNavic, "util_navic"),
            (MetricKind::UtilQzss, "util_qzss"),
            (MetricKind::SlipAll, "slip_all"),
            (MetricKind::SlipGps, "slip_gps"),
            (MetricKind::SlipGlonass, "slip_glonass"),
            (MetricKind::SlipGalileo, "slip_galileo"),
            (MetricKind::SlipBeidou, "slip_beidou"),
            (MetricKind::SlipNavic, "slip_navic"),
            (MetricKind::SlipQzss, "slip_qzss"),
        ];
        assert_eq!(expected.len(), MetricKind::COUNT);
        for (kind, wire) in expected {
            let de: StrDeserializer<'_, DeError> = wire.into_deserializer();
            assert_eq!(
                MetricKind::deserialize(de),
                Ok(kind),
                "deserializing {wire:?}"
            );
        }
    }
}

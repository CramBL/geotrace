/// One variant per plot/telemetry metric.
///
/// Shared between the persisted UI settings (which key per-metric visibility
/// flags by this type, see `geotrace::settings::PlotSettings::metric`) and
/// the plot widget (which drives chip labels, colors, and series lookups off
/// the same variant set). The two previously carried independent copies of
/// this enum kept in sync by hand; a single definition here removes that
/// drift risk entirely.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, strum::EnumIter, serde::Serialize, serde::Deserialize,
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
    Velocity,
    Eph,
    HeadingDeg,
    ClockDeltaMs,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;
    use serde::de::IntoDeserializer;
    use serde::de::value::{Error as DeError, StrDeserializer};

    /// Locks the on-disk spelling of every variant. `Settings` files persist
    /// these strings under `[plot.metric]`; a silent rename here would orphan
    /// every user's saved metric-visibility preferences on their next launch.
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
            (MetricKind::Velocity, "velocity"),
            (MetricKind::Eph, "eph"),
            (MetricKind::HeadingDeg, "heading_deg"),
            (MetricKind::ClockDeltaMs, "clock_delta_ms"),
        ];
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

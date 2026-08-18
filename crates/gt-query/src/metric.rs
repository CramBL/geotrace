//! The metrics a query can reference, and their quantity kinds.

use gt_types::MetricKind;

use crate::dimension::Dimension;

/// Every metric addressable from a query.
///
/// Covers all of [`MetricKind`] (names are the wire names with unit suffixes
/// stripped, since units live in the type system here) plus the per-point
/// fields that are not plot metrics: `time`, `sys_time`, `lat`, `lon`, and the
/// derived `accel`. The mapping is pinned by tests below.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    strum::Display,
    strum::EnumString,
    strum::IntoStaticStr,
    strum::EnumIter,
    strum::EnumCount,
)]
#[strum(serialize_all = "snake_case")]
pub enum QueryMetric {
    Time,
    SysTime,
    Lat,
    Lon,
    Velocity,
    Heading,
    Accel,
    Eph,
    ClockDelta,
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
    UtilAll,
    UtilGps,
    UtilGlonass,
    UtilGalileo,
    UtilBeidou,
    UtilNavic,
    UtilQzss,
    SlipAll,
    SlipGps,
    SlipGlonass,
    SlipGalileo,
    SlipBeidou,
    SlipNavic,
    SlipQzss,
    SnapError,
    Jamming,
    Hp30,
    Kp,
    Tec,
}

/// The dimension of a value, checked statically before a run.
///
/// `Condition` is the type of comparisons and `and`/`or`/`not` - it never
/// belongs to a metric.
#[derive(Debug, Clone, Copy, PartialEq, Eq, strum::Display, strum::EnumIter, strum::EnumCount)]
#[strum(serialize_all = "lowercase")]
pub enum Quantity {
    Timestamp,
    Angle,
    Direction,
    Speed,
    Acceleration,
    Length,
    Duration,
    Count,
    Ratio,
    Rate,
    /// Compares only against a bare number, never a count or a ratio: a
    /// number on a published scale (the geomagnetic Kp scale, TEC units),
    /// dimensionless like a count but neither discrete nor a share.
    Index,
    Condition,
}

impl Quantity {
    /// The physical dimension of this quantity, or `None` for the quantities
    /// that never take part in dimensional arithmetic ([`Quantity::Timestamp`]
    /// and [`Quantity::Condition`]).
    ///
    /// [`Quantity::Count`] and [`Quantity::Ratio`] are both dimensionless. The
    /// exponents cannot distinguish them: the checker's `Kind` tag does.
    pub fn dimension(self) -> Option<Dimension> {
        Some(match self {
            Quantity::Angle | Quantity::Direction => Dimension::ANGLE,
            Quantity::Length => Dimension::LENGTH,
            Quantity::Speed => Dimension::SPEED,
            Quantity::Acceleration => Dimension::ACCELERATION,
            Quantity::Duration => Dimension::TIME,
            Quantity::Rate => Dimension::RATE,
            Quantity::Count | Quantity::Ratio | Quantity::Index => Dimension::DIMENSIONLESS,
            Quantity::Timestamp | Quantity::Condition => return None,
        })
    }
}

impl QueryMetric {
    pub fn quantity(self) -> Quantity {
        match self {
            QueryMetric::Time | QueryMetric::SysTime => Quantity::Timestamp,
            QueryMetric::Lat | QueryMetric::Lon => Quantity::Angle,
            QueryMetric::Velocity => Quantity::Speed,
            QueryMetric::Heading => Quantity::Direction,
            QueryMetric::Accel => Quantity::Acceleration,
            QueryMetric::Eph | QueryMetric::SnapError => Quantity::Length,
            QueryMetric::ClockDelta => Quantity::Duration,
            QueryMetric::SatsSeen
            | QueryMetric::SatsFix
            | QueryMetric::GpsSeen
            | QueryMetric::GpsFix
            | QueryMetric::GlonassSeen
            | QueryMetric::GlonassFix
            | QueryMetric::GalileoSeen
            | QueryMetric::GalileoFix
            | QueryMetric::BeidouSeen
            | QueryMetric::BeidouFix
            | QueryMetric::NavicSeen
            | QueryMetric::NavicFix
            | QueryMetric::QzssSeen
            | QueryMetric::QzssFix => Quantity::Count,
            QueryMetric::UtilAll
            | QueryMetric::UtilGps
            | QueryMetric::UtilGlonass
            | QueryMetric::UtilGalileo
            | QueryMetric::UtilBeidou
            | QueryMetric::UtilNavic
            | QueryMetric::UtilQzss
            | QueryMetric::Jamming => Quantity::Ratio,
            QueryMetric::SlipAll
            | QueryMetric::SlipGps
            | QueryMetric::SlipGlonass
            | QueryMetric::SlipGalileo
            | QueryMetric::SlipBeidou
            | QueryMetric::SlipNavic
            | QueryMetric::SlipQzss => Quantity::Rate,
            QueryMetric::Hp30 | QueryMetric::Kp | QueryMetric::Tec => Quantity::Index,
        }
    }

    /// The plot metric this maps to, if any.
    pub fn metric_kind(self) -> Option<MetricKind> {
        match self {
            QueryMetric::Time
            | QueryMetric::SysTime
            | QueryMetric::Lat
            | QueryMetric::Lon
            | QueryMetric::Accel => None,
            QueryMetric::Velocity => Some(MetricKind::Velocity),
            QueryMetric::Heading => Some(MetricKind::HeadingDeg),
            QueryMetric::Eph => Some(MetricKind::Eph),
            QueryMetric::ClockDelta => Some(MetricKind::ClockDeltaMs),
            QueryMetric::SatsSeen => Some(MetricKind::SatsSeen),
            QueryMetric::SatsFix => Some(MetricKind::SatsFix),
            QueryMetric::GpsSeen => Some(MetricKind::GpsSeen),
            QueryMetric::GpsFix => Some(MetricKind::GpsFix),
            QueryMetric::GlonassSeen => Some(MetricKind::GlonassSeen),
            QueryMetric::GlonassFix => Some(MetricKind::GlonassFix),
            QueryMetric::GalileoSeen => Some(MetricKind::GalileoSeen),
            QueryMetric::GalileoFix => Some(MetricKind::GalileoFix),
            QueryMetric::BeidouSeen => Some(MetricKind::BeidouSeen),
            QueryMetric::BeidouFix => Some(MetricKind::BeidouFix),
            QueryMetric::NavicSeen => Some(MetricKind::NavicSeen),
            QueryMetric::NavicFix => Some(MetricKind::NavicFix),
            QueryMetric::QzssSeen => Some(MetricKind::QzssSeen),
            QueryMetric::QzssFix => Some(MetricKind::QzssFix),
            QueryMetric::UtilAll => Some(MetricKind::UtilAll),
            QueryMetric::UtilGps => Some(MetricKind::UtilGps),
            QueryMetric::UtilGlonass => Some(MetricKind::UtilGlonass),
            QueryMetric::UtilGalileo => Some(MetricKind::UtilGalileo),
            QueryMetric::UtilBeidou => Some(MetricKind::UtilBeidou),
            QueryMetric::UtilNavic => Some(MetricKind::UtilNavic),
            QueryMetric::UtilQzss => Some(MetricKind::UtilQzss),
            QueryMetric::SlipAll => Some(MetricKind::SlipAll),
            QueryMetric::SlipGps => Some(MetricKind::SlipGps),
            QueryMetric::SlipGlonass => Some(MetricKind::SlipGlonass),
            QueryMetric::SlipGalileo => Some(MetricKind::SlipGalileo),
            QueryMetric::SlipBeidou => Some(MetricKind::SlipBeidou),
            QueryMetric::SlipNavic => Some(MetricKind::SlipNavic),
            QueryMetric::SlipQzss => Some(MetricKind::SlipQzss),
            QueryMetric::SnapError => Some(MetricKind::SnapError),
            QueryMetric::Jamming => Some(MetricKind::Jamming),
            QueryMetric::Hp30 => Some(MetricKind::Hp30),
            QueryMetric::Kp => Some(MetricKind::Kp),
            QueryMetric::Tec => Some(MetricKind::Tec),
        }
    }

    pub fn is_util(self) -> bool {
        matches!(
            self,
            QueryMetric::UtilAll
                | QueryMetric::UtilGps
                | QueryMetric::UtilGlonass
                | QueryMetric::UtilGalileo
                | QueryMetric::UtilBeidou
                | QueryMetric::UtilNavic
                | QueryMetric::UtilQzss
        )
    }

    pub fn is_slip(self) -> bool {
        matches!(
            self,
            QueryMetric::SlipAll
                | QueryMetric::SlipGps
                | QueryMetric::SlipGlonass
                | QueryMetric::SlipGalileo
                | QueryMetric::SlipBeidou
                | QueryMetric::SlipNavic
                | QueryMetric::SlipQzss
        )
    }
}

#[cfg(test)]
mod tests {
    use serde::Deserialize;
    use serde::de::IntoDeserializer;
    use serde::de::value::{Error as DeError, StrDeserializer};
    use strum::{EnumCount as _, IntoEnumIterator as _};

    use super::*;

    /// Every quantity's dimension is pinned, and only `Timestamp` and
    /// `Condition` are non-dimensional. Asserted against `COUNT` so a new
    /// quantity must be classified here.
    #[test]
    fn quantity_dimensions_are_pinned() {
        let dimensioned = [
            (Quantity::Angle, Dimension::ANGLE),
            (Quantity::Direction, Dimension::ANGLE),
            (Quantity::Speed, Dimension::SPEED),
            (Quantity::Acceleration, Dimension::ACCELERATION),
            (Quantity::Length, Dimension::LENGTH),
            (Quantity::Duration, Dimension::TIME),
            (Quantity::Rate, Dimension::RATE),
            (Quantity::Count, Dimension::DIMENSIONLESS),
            (Quantity::Ratio, Dimension::DIMENSIONLESS),
            (Quantity::Index, Dimension::DIMENSIONLESS),
        ];
        for (quantity, dimension) in dimensioned {
            assert_eq!(
                quantity.dimension(),
                Some(dimension),
                "{quantity} dimension"
            );
        }

        let non_dimensional: Vec<Quantity> = Quantity::iter()
            .filter(|q| q.dimension().is_none())
            .collect();
        assert_eq!(non_dimensional, [Quantity::Timestamp, Quantity::Condition]);
        assert_eq!(dimensioned.len() + non_dimensional.len(), Quantity::COUNT);
    }

    /// The exponent algebra reproduces the named derivations
    /// (`length / time = speed`, `speed / time = acceleration`).
    #[test]
    fn dimensional_algebra_reproduces_the_named_derivations() {
        let dim = |q: Quantity| q.dimension().unwrap();
        assert_eq!(
            dim(Quantity::Length) / dim(Quantity::Duration),
            dim(Quantity::Speed)
        );
        assert_eq!(
            dim(Quantity::Speed) / dim(Quantity::Duration),
            dim(Quantity::Acceleration)
        );
        assert_eq!(
            dim(Quantity::Speed) * dim(Quantity::Duration),
            dim(Quantity::Length)
        );
        assert_eq!(
            dim(Quantity::Acceleration) * dim(Quantity::Duration),
            dim(Quantity::Speed)
        );
    }

    /// Every plot metric is reachable from a query, none is covered twice,
    /// and the extra query-only metrics are exactly the five per-point
    /// fields. A new `MetricKind` variant fails here until it is wired up.
    #[test]
    fn covers_every_metric_kind() {
        let mapped: Vec<MetricKind> = QueryMetric::iter()
            .filter_map(QueryMetric::metric_kind)
            .collect();
        let mut deduped = mapped.clone();
        deduped.dedup();
        assert_eq!(mapped.len(), deduped.len(), "a MetricKind is mapped twice");
        let unmapped: Vec<MetricKind> = MetricKind::iter()
            .filter(|kind| !mapped.contains(kind))
            .collect();
        assert!(unmapped.is_empty(), "no query metric for {unmapped:?}");
        assert_eq!(QueryMetric::COUNT, MetricKind::COUNT + 5);
    }

    /// The DSL name is the `MetricKind` wire name with the unit suffix
    /// stripped - `heading_deg` and `clock_delta_ms` are the only renames.
    /// Locked by deserializing the reconstructed wire name back to the mapped
    /// variant (same idiom as `gt_types::metrics::tests::wire_names_are_stable`).
    #[test]
    fn names_match_wire_names() {
        for qm in QueryMetric::iter() {
            let Some(kind) = qm.metric_kind() else {
                continue;
            };
            let suffix = match qm {
                QueryMetric::Heading => "_deg",
                QueryMetric::ClockDelta => "_ms",
                _ => "",
            };
            let wire = format!("{qm}{suffix}");
            let de: StrDeserializer<'_, DeError> = wire.as_str().into_deserializer();
            assert_eq!(MetricKind::deserialize(de), Ok(kind), "wire name {wire:?}");
        }
    }
}

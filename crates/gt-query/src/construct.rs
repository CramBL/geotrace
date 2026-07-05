//! The catalog of query-language constructs and their documentation.
//!
//! One [`Construct`] per language element (source, stage, function, metric,
//! unit, parameter, display mode), each carrying the text the editor shows:
//! a one-line `summary` for the completion popup, and a fuller `doc` plus
//! `examples` for the hover tooltip.
//!
//! The catalog is the single source of the assistance text - the completion
//! popup and the hover doc read the same entry, so they cannot disagree. It
//! is built off the language's own enums (`QueryMetric`, `Func`, `Unit`,
//! `ParamName`, gt_types::DisplayMode) so a new variant cannot ship without
//! a catalog entry (enforced by `catalog_is_exhaustive`).

use gt_types::DisplayMode;
use strum::IntoEnumIterator as _;

use crate::ast::{Func, ParamName};
use crate::metric::{Quantity, QueryMetric};
use crate::unit::Unit;

/// The category of a construct - drives grouping and the hover header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConstructKind {
    /// The `points` source.
    Source,
    /// A pipeline stage keyword (`with`, `window`, `where`, `table`).
    Stage,
    /// A display-mode stage (`draw`, `keep`, `hide`).
    Mode,
    /// An expression function (`avg`, `spread`, …).
    Function,
    /// A per-point metric (`velocity`, `util_gps`, …).
    Metric,
    /// A unit literal suffix (`km/h`, `deg`, …).
    Unit,
    /// A `with` parameter (`mask`, `snr_drop`, `slip_window`).
    Param,
}

impl ConstructKind {
    /// Human label for the hover header.
    pub fn label(self) -> &'static str {
        match self {
            ConstructKind::Source => "source",
            ConstructKind::Stage => "stage",
            ConstructKind::Mode => "display mode",
            ConstructKind::Function => "function",
            ConstructKind::Metric => "metric",
            ConstructKind::Unit => "unit",
            ConstructKind::Param => "parameter",
        }
    }
}

/// One documented language element.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Construct {
    /// The literal token, e.g. `avg`, `util_gps`, `km/h`.
    pub name: &'static str,
    pub kind: ConstructKind,
    /// One line, shown in the completion popup and as the hover's first line.
    pub summary: &'static str,
    /// Fuller explanation for hover, Rust-doc style. Empty for constructs
    /// whose summary already says everything (`min`/`max`/`avg`).
    pub doc: &'static str,
    /// Example snippets, shown on hover.
    pub examples: &'static [&'static str],
}

/// Every construct in the language, in a stable order (sources, stages,
/// modes, functions, metrics, units, parameters).
pub fn catalog() -> Vec<Construct> {
    let mut out = vec![Construct {
        name: "points",
        kind: ConstructKind::Source,
        summary: "the loaded track points",
        doc: "The data a query reads: the points of every visible track, \
              after the global filter. Every query starts with `points`.",
        examples: &["points | where velocity > 30 km/h"],
    }];
    out.extend(STAGES.iter().copied());
    out.extend(DisplayMode::iter().map(mode_construct));
    out.extend(Func::iter().map(func_construct));
    out.extend(QueryMetric::iter().map(metric_construct));
    out.extend(Unit::iter().map(unit_construct));
    out.extend(ParamName::iter().map(param_construct));
    out
}

/// The pipeline stage keywords (the display modes are catalogued separately
/// via [`DisplayMode`]).
const STAGES: &[Construct] = &[
    Construct {
        name: "with",
        kind: ConstructKind::Stage,
        summary: "set satellite-analysis parameters",
        doc: "Supplies the parameters the util/slip metrics need (`mask`, \
              `snr_drop`, `slip_window`). Must come directly after `points`.",
        examples: &["points | with mask 15 deg | where util_gps < 50 %"],
    },
    Construct {
        name: "window",
        kind: ConstructKind::Stage,
        summary: "slide an N-point window along the track",
        doc: "Evaluates the `where` condition over a sliding group of N \
              consecutive points instead of one point at a time. Aggregates \
              (`avg`, `spread`, …) operate over the window.",
        examples: &["points | window 10 | where avg(velocity) > 30 km/h"],
    },
    Construct {
        name: "where",
        kind: ConstructKind::Stage,
        summary: "keep points (or windows) matching a condition",
        doc: "The condition that defines a match. Combine terms with `and`, \
              `or`, `not`. Several `where` stages combine as if joined with \
              `and`.",
        examples: &["points | where eph > 20 m and velocity > 5 km/h"],
    },
    Construct {
        name: "table",
        kind: ConstructKind::Stage,
        summary: "choose the columns of the match table",
        doc: "Sets which metrics appear, in order, in each match's point \
              table. `time` is always the first column. Without it, the \
              table shows every metric the query referenced.",
        examples: &["points | where velocity > 30 km/h | table time, velocity, heading"],
    },
];

fn mode_construct(mode: DisplayMode) -> Construct {
    let (summary, doc, examples): (_, _, &[&str]) = match mode {
        DisplayMode::Draw => (
            "draw matches as halos (the default)",
            "Draws each match as a halo over the track. This is the default \
             when no display stage is written.",
            &["points | where velocity > 30 km/h | draw"],
        ),
        DisplayMode::Keep => (
            "show only matching points",
            "Hides every non-matching point, leaving only the matches on the \
             map. The polyline breaks where points were removed.",
            &["points | where velocity > 30 km/h | keep"],
        ),
        DisplayMode::Hide => (
            "remove matching points",
            "Hides the matching points, leaving the rest of the track - the \
             query acts as a filter that removes what it matches.",
            &["points | where velocity < 2 km/h | hide"],
        ),
    };
    Construct {
        name: mode.into(),
        kind: ConstructKind::Mode,
        summary,
        doc,
        examples,
    }
}

fn func_construct(func: Func) -> Construct {
    // min/max/avg are self-evident: summary only, no doc body (Marc).
    let (summary, doc, examples): (_, _, &[&str]) = match func {
        Func::Avg => ("average over the window", "", &["avg(velocity) > 30 km/h"]),
        Func::Min => (
            "smallest value in the window",
            "",
            &["min(velocity) > 5 km/h"],
        ),
        Func::Max => ("largest value in the window", "", &["max(eph) < 15 m"]),
        Func::Spread => (
            "range of values across the window",
            "The largest value minus the smallest. On a direction (`heading`) \
             it is circular: the smallest arc containing all the headings, so \
             350 deg and 10 deg have a spread of 20 deg, not 340 deg.",
            &["spread(heading) <= 10 deg"],
        ),
        Func::Std => (
            "standard deviation over the window",
            "The population standard deviation, divided by N. Same unit as the \
             values, so it compares directly against a threshold, unlike a \
             variance. On a direction (`heading`) it is circular, robust across \
             the 0/360 wrap.",
            &["std(heading) <= 3 deg", "std(velocity) < 2 km/h"],
        ),
        Func::Var => (
            "variance over the window",
            "The population variance, divided by N. Its unit is the square of \
             the values' unit, so it has no direct threshold: compare with \
             `std` (its square root) instead, or form a ratio of two \
             variances. Not defined on a direction (`heading`) - use `std`.",
            &["var(sats_fix) < 4"],
        ),
        Func::First => (
            "value at the window's first point",
            "The value at the earliest point of the window.",
            &["first(velocity) < 5 km/h"],
        ),
        Func::Last => (
            "value at the window's last point",
            "The value at the latest point of the window.",
            &["last(velocity) > 30 km/h"],
        ),
        Func::Delta => (
            "change across the window (last minus first)",
            "Last value minus first. On a direction (`heading`) it is the \
             signed shortest turn, in (-180, 180] degrees. On timestamps it \
             yields a duration.",
            &["delta(velocity) > 10 km/h", "delta(time) <= 15 s"],
        ),
        Func::Abs => (
            "absolute value",
            "Drops the sign of a value. Works with or without a window.",
            &["abs(accel) > 1 m/s2"],
        ),
        Func::Sqrt => (
            "square root",
            "The square root of a value; its unit is the square root of the \
             value's unit, so it reduces a squared quantity - `sqrt(velocity²)` \
             is a speed. Pair it with `²` for magnitudes, e.g. \
             `sqrt(lat² + lon²)`. Works with or without a window.",
            &["sqrt(lat² + lon²) > 1 deg"],
        ),
    };
    Construct {
        name: func.into(),
        kind: ConstructKind::Function,
        summary,
        doc,
        examples,
    }
}

fn param_construct(param: ParamName) -> Construct {
    let (summary, doc, examples): (_, _, &[&str]) = match param {
        ParamName::Mask => (
            "elevation mask for util/slip metrics",
            "Satellites below this elevation do not count as in view. \
             Required by every `util_*` and `slip_*` metric.",
            &["with mask 15 deg"],
        ),
        ParamName::SnrDrop => (
            "SNR drop counted as a slip",
            "An above-mask satellite whose SNR falls by more than this (in \
             dB-Hz) between epochs counts as a loss of lock. A bare number. \
             Required by the `slip_*` metrics.",
            &["with mask 15 deg, snr_drop 10, slip_window 5 min"],
        ),
        ParamName::SlipWindow => (
            "trailing window for the slip rate",
            "The trailing duration over which the per-minute slip rate is \
             averaged. Required by the `slip_*` metrics.",
            &["with mask 15 deg, snr_drop 10, slip_window 5 min"],
        ),
    };
    Construct {
        name: param.into(),
        kind: ConstructKind::Param,
        summary,
        doc,
        examples,
    }
}

fn unit_construct(unit: Unit) -> Construct {
    let (summary, doc): (_, &str) = match unit.quantity() {
        Quantity::Angle | Quantity::Direction => ("degrees", ""),
        Quantity::Length => ("length", ""),
        Quantity::Speed => ("speed", ""),
        Quantity::Acceleration => ("acceleration", ""),
        Quantity::Duration => ("duration", ""),
        Quantity::Ratio => (
            "percent (a 0-100 ratio)",
            "Ratios such as `util_gps` are written with `%`, e.g. `50 %`.",
        ),
        Quantity::Rate => (
            "events per unit time",
            "Rates such as `slip_all` are written with `per`, e.g. `2 per min`.",
        ),
        Quantity::Count | Quantity::Timestamp | Quantity::Condition => ("", ""),
    };
    Construct {
        name: unit.text(),
        kind: ConstructKind::Unit,
        summary,
        doc,
        examples: &[],
    }
}

fn metric_construct(metric: QueryMetric) -> Construct {
    let (summary, doc, examples) = metric_docs(metric);
    Construct {
        name: metric.into(),
        kind: ConstructKind::Metric,
        summary,
        doc,
        examples,
    }
}

/// Per-metric summary, doc body, and examples. Grouped by family so the
/// per-constellation metrics share their explanation.
fn metric_docs(metric: QueryMetric) -> (&'static str, &'static str, &'static [&'static str]) {
    match metric {
        QueryMetric::Time => (
            "GPS receiver clock time",
            "The fix timestamp from the receiver. Restrict a time range with \
             the global filter, not in the query.",
            &["table time, velocity"],
        ),
        QueryMetric::SysTime => (
            "host system-clock time",
            "The host computer's clock at the fix, when recorded. Missing \
             when the host did not timestamp the fix.",
            &[],
        ),
        QueryMetric::Lat => ("latitude, degrees", "", &[]),
        QueryMetric::Lon => ("longitude, degrees", "", &[]),
        QueryMetric::Velocity => ("ground speed", "", &["velocity > 30 km/h"]),
        QueryMetric::Heading => (
            "compass heading, degrees",
            "The direction of travel in [0, 360) degrees. A direction, so \
             `spread`/`delta` treat it circularly. Missing on ghost fixes.",
            &["spread(heading) <= 10 deg"],
        ),
        QueryMetric::Accel => (
            "acceleration along the track",
            "Change in speed over time between consecutive points, signed \
             (negative is decelerating). Missing on the first point of a \
             track and wherever velocity is missing.",
            &["accel >= 0.3 m/s2"],
        ),
        QueryMetric::Eph => (
            "estimated horizontal accuracy",
            "The receiver's own estimate of horizontal position error, in \
             metres. Larger is worse. Missing when the receiver did not \
             report it.",
            &["eph > 20 m"],
        ),
        QueryMetric::ClockDelta => (
            "GPS/system clock offset",
            "The difference between the GPS and host system clocks at the \
             fix. Needs a recorded system timestamp.",
            &[],
        ),
        QueryMetric::SatsSeen => ("satellites in view (all constellations)", "", &[]),
        QueryMetric::SatsFix => (
            "satellites used in the fix (all constellations)",
            "How many satellites the receiver used to compute the fix.",
            &["sats_fix < 6"],
        ),
        QueryMetric::GpsSeen
        | QueryMetric::GlonassSeen
        | QueryMetric::GalileoSeen
        | QueryMetric::BeidouSeen
        | QueryMetric::NavicSeen
        | QueryMetric::QzssSeen => (
            "satellites in view, this constellation",
            "Satellites of one constellation the receiver could see. Missing \
             for a constellation absent from the loaded data.",
            &[],
        ),
        QueryMetric::GpsFix
        | QueryMetric::GlonassFix
        | QueryMetric::GalileoFix
        | QueryMetric::BeidouFix
        | QueryMetric::NavicFix
        | QueryMetric::QzssFix => (
            "satellites used in the fix, this constellation",
            "Satellites of one constellation the receiver used in the fix.",
            &[],
        ),
        QueryMetric::UtilAll
        | QueryMetric::UtilGps
        | QueryMetric::UtilGlonass
        | QueryMetric::UtilGalileo
        | QueryMetric::UtilBeidou
        | QueryMetric::UtilNavic
        | QueryMetric::UtilQzss => (
            "satellite utilization rate",
            "The share of in-view satellites (above the elevation mask) the \
             receiver actually used in the fix. Needs `with mask`. Written \
             as a percentage.",
            &["with mask 15 deg | where util_gps < 50 %"],
        ),
        QueryMetric::SlipAll
        | QueryMetric::SlipGps
        | QueryMetric::SlipGlonass
        | QueryMetric::SlipGalileo
        | QueryMetric::SlipBeidou
        | QueryMetric::SlipNavic
        | QueryMetric::SlipQzss => (
            "loss-of-lock (slip) rate per minute",
            "How often satellites drop out or lose lock, per minute. A slip \
             is a satellite lost while still trackable above the mask, or a \
             steep SNR drop between epochs. Needs `with mask, snr_drop, \
             slip_window`.",
            &["with mask 15 deg, snr_drop 10, slip_window 5 min | where slip_all > 2 per min"],
        ),
    }
}

#[cfg(test)]
mod tests {
    use strum::EnumCount as _;

    use super::*;

    #[test]
    fn catalog_is_exhaustive() {
        let entries = catalog();
        // 1 source + stages + every mode/func/metric/unit/param variant.
        let expected = 1
            + STAGES.len()
            + DisplayMode::COUNT
            + Func::COUNT
            + QueryMetric::COUNT
            + Unit::COUNT
            + ParamName::COUNT;
        assert_eq!(entries.len(), expected);
    }

    #[test]
    fn catalog_names_are_unique_per_kind_and_nonempty() {
        // Names are unique within a kind. Across kinds only `min` repeats -
        // the aggregate and the minute unit - which the parser and hover
        // disambiguate by position.
        let entries = catalog();
        assert!(entries.iter().all(|c| !c.name.is_empty()));
        let mut per_kind: Vec<(ConstructKind, &str)> =
            entries.iter().map(|c| (c.kind, c.name)).collect();
        per_kind.sort_by_key(|(kind, name)| (format!("{kind:?}"), *name));
        let count = per_kind.len();
        per_kind.dedup();
        assert_eq!(per_kind.len(), count, "names must be unique within a kind");

        let repeated: Vec<&str> = {
            let mut names: Vec<&str> = entries.iter().map(|c| c.name).collect();
            names.sort_unstable();
            names
                .windows(2)
                .filter(|w| w[0] == w[1])
                .map(|w| w[0])
                .collect()
        };
        assert_eq!(repeated, vec!["min"], "only `min` is shared across kinds");
    }

    #[test]
    fn every_construct_has_a_summary() {
        for c in catalog() {
            assert!(!c.summary.is_empty(), "{} needs a summary", c.name);
        }
    }

    #[test]
    fn doc_and_examples_have_balanced_backticks() {
        // The hover renderer colors `backticked` spans by toggling on each
        // backtick, so an odd count would color the rest of the string wrong.
        for c in catalog() {
            assert_eq!(
                c.doc.matches('`').count() % 2,
                0,
                "{}'s doc has unbalanced backticks",
                c.name
            );
            for example in c.examples {
                assert_eq!(
                    example.matches('`').count(),
                    0,
                    "{}'s example should be plain query text, no backticks",
                    c.name
                );
            }
        }
    }

    #[test]
    fn catalog_names_match_the_parser() {
        // A metric/func/unit/param name from the catalog must be exactly what
        // the parser accepts - i.e. round-trip through the language.
        for c in catalog() {
            match c.kind {
                ConstructKind::Metric => {
                    let q = format!("points | where {} > 0 km/h", c.name);
                    // Parse only: some comparisons are type errors, but the
                    // name itself must be recognised (no "unknown metric").
                    let parsed = crate::parse(&q);
                    assert!(parsed.is_ok(), "metric {} did not parse", c.name);
                }
                ConstructKind::Function => {
                    assert!(
                        crate::parse(&format!(
                            "points | window 3 | where {}(velocity) > 0 km/h",
                            c.name
                        ))
                        .is_ok(),
                        "function {} did not parse",
                        c.name
                    );
                }
                _ => {}
            }
        }
    }
}

//! Canonical units used by GeoTrace channels and queries.
//!
//! A channel declares an optional [`ChannelUnit`], which is one of three kinds
//! ([`ChannelUnitKind`]):
//!
//! - Recognized: a [`Unit`] from the catalog in this crate. It has a
//!   [`PhysicalQuantity`] and a factor to that quantity's base unit, so a query
//!   can compare the channel against a literal written in any unit of the same
//!   quantity.
//! - Custom: a [`CustomUnit`], a label GeoTrace stores and displays verbatim.
//!   Its values stay dimensionless in queries, because a conversion cannot be
//!   inferred from a label alone.
//! - Legacy: a label read from a file that is neither of those, kept byte for
//!   byte and never accepted as writer input.
//!
//! # Attaching a unit to a channel you write
//!
//! Name a catalog unit through one of the [`Unit`] constants, or call
//! [`ChannelUnit::custom`] for a label the catalog does not cover. Both are
//! writable metadata and both are accepted wherever a channel builder takes a
//! unit.
//!
//! ```
//! use geotrace_sdk_units::{ChannelUnit, Unit};
//!
//! let acceleration = ChannelUnit::recognized(Unit::MG);
//! assert_eq!(acceleration.to_string(), "mg");
//!
//! let score = ChannelUnit::custom("vendor score")?;
//! assert!(score.as_recognized().is_none());
//! # Ok::<(), geotrace_sdk_units::UnitParseError>(())
//! ```
//!
//! Parsing a string yields a recognized unit and nothing else:
//! `"rpm".parse::<ChannelUnit>()` fails with
//! [`UnitParseError::Unrecognized`], the signal to call
//! [`ChannelUnit::custom`] when a display-only label is what you meant.
//!
//! # Reading a unit back
//!
//! A reader turns file metadata into a [`ChannelUnit`] with
//! [`ChannelUnit::from_file_label`], which accepts every label. It resolves
//! aliases such as `degrees`, `kph`, `m/s²` and `µg` to catalog units, keeps
//! any other single-line label as custom, and preserves the rest as legacy.
//! [`ChannelUnit::is_writable`] is false for exactly the legacy case, so a tool
//! that rewrites a file learns which labels it cannot declare again.
//!
//! ```
//! use geotrace_sdk_units::{ChannelUnit, ChannelUnitKind, Unit};
//!
//! assert_eq!(ChannelUnit::from_file_label("m/s²").as_recognized(), Some(Unit::M_PER_S2));
//! assert_eq!(ChannelUnit::from_file_label("rpm").kind(), ChannelUnitKind::Custom);
//! assert_eq!(ChannelUnit::from_file_label("bad\nunit").kind(), ChannelUnitKind::Legacy);
//! ```
//!
//! # What queries do with each kind
//!
//! A query converts a recognized channel to its quantity's base unit with
//! [`Unit::to_base`], compares it there, and formats each result back to the
//! declared scale with [`Unit::from_base`]. Custom and legacy values are read
//! as plain numbers, so a unit literal cannot be compared against them. Stored
//! and plotted values always stay in the scale the recorder declared.

use std::{fmt, str::FromStr};

use uom::si::acceleration::{meter_per_second_squared, standard_gravity};
use uom::si::f64::{Acceleration, Velocity};
use uom::si::velocity::{kilometer_per_hour, knot, meter_per_second};

const S_PER_MIN: f64 = 60.0;
const MIN_PER_H: f64 = 60.0;
const PERCENT: f64 = 100.0;
const PER_S_TO_PER_MIN: f64 = 60.0;

/// The physical quantity represented by a recognized [`Unit`].
///
/// Every quantity has one base unit, which is what [`Unit::to_base`] converts
/// to: degrees for [`Angle`](Self::Angle), meters for [`Length`](Self::Length),
/// meters per second for [`Speed`](Self::Speed), meters per second squared for
/// [`Acceleration`](Self::Acceleration), seconds for
/// [`Duration`](Self::Duration), the unit fraction for [`Ratio`](Self::Ratio)
/// (`100 %` is `1.0`), and per minute for [`Rate`](Self::Rate).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PhysicalQuantity {
    Angle,
    Length,
    Speed,
    Acceleration,
    Duration,
    Ratio,
    Rate,
}

/// An SI prefix scaling a [`BaseUnit`].
///
/// A prefix and a base form a unit only where [`BaseUnit::accepts_prefix`]
/// pairs them, so the catalog holds `mm` and `mg` but no `kg` or `cs`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, strum::EnumIter, strum::EnumCount)]
pub enum SiPrefix {
    Nano,
    Micro,
    Milli,
    Centi,
    Kilo,
}

impl SiPrefix {
    /// The scale applied to the base unit.
    pub fn factor(self) -> f64 {
        match self {
            Self::Nano => 1e-9,
            Self::Micro => 1e-6,
            Self::Milli => 1e-3,
            Self::Centi => 1e-2,
            Self::Kilo => 1e3,
        }
    }

    /// Canonical spelling before the base unit.
    ///
    /// Micro is written `u`; parsing also accepts `µ` and `μ`.
    pub const fn text(self) -> &'static str {
        match self {
            Self::Nano => "n",
            Self::Micro => "u",
            Self::Milli => "m",
            Self::Centi => "c",
            Self::Kilo => "k",
        }
    }

    fn split(ident: &str) -> Option<(Self, &str)> {
        let micro = ident
            .strip_prefix('u')
            .or_else(|| ident.strip_prefix('µ'))
            .or_else(|| ident.strip_prefix('μ'))
            .map(|rest| (Self::Micro, rest));
        micro.or_else(|| {
            let (first, rest) = ident.split_at_checked(1)?;
            let prefix = match first {
                "n" => Self::Nano,
                "m" => Self::Milli,
                "c" => Self::Centi,
                "k" => Self::Kilo,
                _ => return None,
            };
            Some((prefix, rest))
        })
    }
}

/// An unprefixed recognized unit.
///
/// A [`Unit`] is one of these, optionally scaled by an [`SiPrefix`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, strum::EnumIter, strum::EnumCount)]
pub enum BaseUnit {
    Deg,
    M,
    KmPerH,
    MPerS,
    Kn,
    MPerS2,
    G,
    KmPerHPerS,
    S,
    Min,
    H,
    Percent,
    PerS,
    PerMin,
    PerH,
}

impl BaseUnit {
    /// The physical quantity this unit measures.
    pub fn quantity(self) -> PhysicalQuantity {
        match self {
            Self::Deg => PhysicalQuantity::Angle,
            Self::M => PhysicalQuantity::Length,
            Self::KmPerH | Self::MPerS | Self::Kn => PhysicalQuantity::Speed,
            Self::MPerS2 | Self::G | Self::KmPerHPerS => PhysicalQuantity::Acceleration,
            Self::S | Self::Min | Self::H => PhysicalQuantity::Duration,
            Self::Percent => PhysicalQuantity::Ratio,
            Self::PerS | Self::PerMin | Self::PerH => PhysicalQuantity::Rate,
        }
    }

    /// Factor converting one value of this unit to its quantity's base unit.
    ///
    /// [`PhysicalQuantity`] lists the base unit of each quantity.
    pub fn to_base(self) -> f64 {
        match self {
            Self::Deg | Self::M | Self::MPerS | Self::MPerS2 | Self::S | Self::PerMin => 1.0,
            Self::KmPerH | Self::KmPerHPerS => {
                Velocity::new::<kilometer_per_hour>(1.0).get::<meter_per_second>()
            }
            Self::Kn => Velocity::new::<knot>(1.0).get::<meter_per_second>(),
            Self::G => Acceleration::new::<standard_gravity>(1.0).get::<meter_per_second_squared>(),
            Self::Min => S_PER_MIN,
            Self::H => S_PER_MIN * MIN_PER_H,
            Self::Percent => 1.0 / PERCENT,
            Self::PerS => PER_S_TO_PER_MIN,
            Self::PerH => 1.0 / MIN_PER_H,
        }
    }

    /// Canonical wire spelling.
    ///
    /// A squared unit uses an ASCII `2` (`m/s2`) and a rate is spelled out
    /// (`per min`), so every catalog spelling is ASCII.
    pub const fn text(self) -> &'static str {
        match self {
            Self::Deg => "deg",
            Self::M => "m",
            Self::KmPerH => "km/h",
            Self::MPerS => "m/s",
            Self::Kn => "kn",
            Self::MPerS2 => "m/s2",
            Self::G => "g",
            Self::KmPerHPerS => "km/h/s",
            Self::S => "s",
            Self::Min => "min",
            Self::H => "h",
            Self::Percent => "%",
            Self::PerS => "per s",
            Self::PerMin => "per min",
            Self::PerH => "per h",
        }
    }

    /// Whether this base accepts the given prefix.
    ///
    /// The pairs are curated to spellings sensors report and readers cannot
    /// misread: meters take every prefix, seconds take `n`/`u`/`m`, standard
    /// gravity takes `u`/`m`, and `m/s` and `m/s2` take `m`/`c`. Neither `kg`
    /// nor `cs` parses: `kg` reads as a kilogram, and `cs` is a scale nobody
    /// writes.
    pub fn accepts_prefix(self, prefix: SiPrefix) -> bool {
        match self {
            Self::M => true,
            Self::S => matches!(prefix, SiPrefix::Nano | SiPrefix::Micro | SiPrefix::Milli),
            Self::G => matches!(prefix, SiPrefix::Micro | SiPrefix::Milli),
            Self::MPerS | Self::MPerS2 => matches!(prefix, SiPrefix::Milli | SiPrefix::Centi),
            Self::Deg
            | Self::KmPerH
            | Self::Kn
            | Self::KmPerHPerS
            | Self::Min
            | Self::H
            | Self::Percent
            | Self::PerS
            | Self::PerMin
            | Self::PerH => false,
        }
    }

    fn from_ident(ident: &str) -> Option<Self> {
        match ident {
            "deg" => Some(Self::Deg),
            "m" => Some(Self::M),
            "kmh" => Some(Self::KmPerH),
            "kn" => Some(Self::Kn),
            "g" => Some(Self::G),
            "s" => Some(Self::S),
            "min" => Some(Self::Min),
            "h" => Some(Self::H),
            _ => None,
        }
    }
}

/// A recognized unit, optionally scaled by an SI prefix.
///
/// The catalog is fixed: a value comes from an associated constant, from one of
/// the parsing entry points, or from [`Unit::recognized`]. A constant is named
/// after the canonical spelling with `/` written as `_PER_`, so [`Unit::KM_PER_H`]
/// is `km/h` and [`Unit::PER_MIN`] is `per min`.
///
/// # Parsing
///
/// [`Unit::from_label`] and the [`FromStr`] impl take a whole label as a file or
/// a person writes it: they trim it, resolve aliases such as `degrees`, `°`,
/// `kph`, `mps2` and `mG`, rewrite `²` and `^2` to `2`, and split a compound
/// spelling on `/`. [`FromStr`] reports a [`UnitParseError`] where
/// [`Unit::from_label`] returns [`None`].
///
/// [`Unit::from_ident`], [`Unit::from_pair`] and [`Unit::from_triple`] take one,
/// two or three identifiers a lexer has already split apart (`m` and `s2`, or
/// `km`, `h` and `s`) and match them as written, without trimming or alias
/// rewriting. The query parser reads unit literals through them because its
/// lexer has already cut `km/h` into three tokens.
///
/// ```
/// use geotrace_sdk_units::Unit;
///
/// assert_eq!(Unit::from_label(" degrees "), Some(Unit::DEG));
/// assert_eq!(Unit::from_label("m/s²"), Some(Unit::M_PER_S2));
/// assert_eq!(Unit::from_ident("degrees"), None);
/// assert_eq!(Unit::from_pair("m", "s2"), Some(Unit::M_PER_S2));
/// ```
///
/// # Spelling
///
/// [`Unit::label`] returns the canonical spelling as a `&'static str`, which is
/// what a `.gtd` file stores. [`Unit::text`] and the [`fmt::Display`] impl write
/// that same spelling into a formatter. [`Unit::canonical_text`] returns it only
/// for the units in [`Unit::CANONICAL`], which is how the query editor holds its
/// suggestions to one unit per scale.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Unit {
    prefix: Option<SiPrefix>,
    base: BaseUnit,
    label: &'static str,
}

#[expect(
    clippy::missing_fields_in_debug,
    reason = "the internal label repeats the established prefix and base debug representation"
)]
impl fmt::Debug for Unit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Unit")
            .field("prefix", &self.prefix)
            .field("base", &self.base)
            .finish()
    }
}

/// Stable language-binding names for a recognized [Unit].
///
/// `examples/generate_bindings.rs` renders the C++ `RecognizedUnit` enumerators
/// and the Python `Unit` class attributes from [`Unit::BINDINGS`], so a unit
/// added to the catalog needs a binding here and a regenerated catalog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnitBinding {
    pub unit: Unit,
    pub rust: &'static str,
    pub cpp: &'static str,
    pub python: &'static str,
}

impl Unit {
    pub const DEG: Self = Self::base(BaseUnit::Deg);
    pub const M: Self = Self::base(BaseUnit::M);
    pub const NM: Self = Self::with_label(SiPrefix::Nano, BaseUnit::M, "nm");
    pub const UM: Self = Self::with_label(SiPrefix::Micro, BaseUnit::M, "um");
    pub const MM: Self = Self::with_label(SiPrefix::Milli, BaseUnit::M, "mm");
    pub const CM: Self = Self::with_label(SiPrefix::Centi, BaseUnit::M, "cm");
    pub const KM: Self = Self::with_label(SiPrefix::Kilo, BaseUnit::M, "km");
    pub const KM_PER_H: Self = Self::base(BaseUnit::KmPerH);
    pub const M_PER_S: Self = Self::base(BaseUnit::MPerS);
    pub const MM_PER_S: Self = Self::with_label(SiPrefix::Milli, BaseUnit::MPerS, "mm/s");
    pub const CM_PER_S: Self = Self::with_label(SiPrefix::Centi, BaseUnit::MPerS, "cm/s");
    pub const KN: Self = Self::base(BaseUnit::Kn);
    pub const M_PER_S2: Self = Self::base(BaseUnit::MPerS2);
    pub const MM_PER_S2: Self = Self::with_label(SiPrefix::Milli, BaseUnit::MPerS2, "mm/s2");
    pub const CM_PER_S2: Self = Self::with_label(SiPrefix::Centi, BaseUnit::MPerS2, "cm/s2");
    /// Standard gravity, 9.80665 m/s².
    pub const G: Self = Self::base(BaseUnit::G);
    /// A millionth of standard gravity.
    pub const UG: Self = Self::with_label(SiPrefix::Micro, BaseUnit::G, "ug");
    /// A thousandth of standard gravity.
    pub const MG: Self = Self::with_label(SiPrefix::Milli, BaseUnit::G, "mg");
    pub const KM_PER_H_PER_S: Self = Self::base(BaseUnit::KmPerHPerS);
    pub const NS: Self = Self::with_label(SiPrefix::Nano, BaseUnit::S, "ns");
    pub const US: Self = Self::with_label(SiPrefix::Micro, BaseUnit::S, "us");
    pub const MS: Self = Self::with_label(SiPrefix::Milli, BaseUnit::S, "ms");
    pub const S: Self = Self::base(BaseUnit::S);
    pub const MIN: Self = Self::base(BaseUnit::Min);
    pub const H: Self = Self::base(BaseUnit::H);
    pub const PERCENT: Self = Self::base(BaseUnit::Percent);
    pub const PER_S: Self = Self::base(BaseUnit::PerS);
    pub const PER_MIN: Self = Self::base(BaseUnit::PerMin);
    pub const PER_H: Self = Self::base(BaseUnit::PerH);

    /// The units the query editor suggests, one per scale, in catalog order.
    ///
    /// A query accepts every unit in [`Unit::recognized`]. This subset is what
    /// completions and diagnostics list, which keeps the prefixed sensor scales
    /// (`mm`, `ug`, `cm/s`) out of a list a person reads.
    /// [`Unit::canonical_text`] returns [`Some`] for exactly these units.
    pub const CANONICAL: [Self; 17] = [
        Self::DEG,
        Self::M,
        Self::KM,
        Self::KM_PER_H,
        Self::M_PER_S,
        Self::KN,
        Self::M_PER_S2,
        Self::G,
        Self::KM_PER_H_PER_S,
        Self::MS,
        Self::S,
        Self::MIN,
        Self::H,
        Self::PERCENT,
        Self::PER_S,
        Self::PER_MIN,
        Self::PER_H,
    ];

    /// Canonical catalog used to generate the C++ and Python bindings.
    ///
    /// This table also fixes the order [`Unit::recognized`] iterates in.
    pub const BINDINGS: [UnitBinding; 29] = [
        UnitBinding {
            unit: Self::DEG,
            rust: "DEG",
            cpp: "Deg",
            python: "DEG",
        },
        UnitBinding {
            unit: Self::M,
            rust: "M",
            cpp: "M",
            python: "M",
        },
        UnitBinding {
            unit: Self::NM,
            rust: "NM",
            cpp: "Nm",
            python: "NM",
        },
        UnitBinding {
            unit: Self::UM,
            rust: "UM",
            cpp: "Um",
            python: "UM",
        },
        UnitBinding {
            unit: Self::MM,
            rust: "MM",
            cpp: "Mm",
            python: "MM",
        },
        UnitBinding {
            unit: Self::CM,
            rust: "CM",
            cpp: "Cm",
            python: "CM",
        },
        UnitBinding {
            unit: Self::KM,
            rust: "KM",
            cpp: "Km",
            python: "KM",
        },
        UnitBinding {
            unit: Self::KM_PER_H,
            rust: "KM_PER_H",
            cpp: "KmPerH",
            python: "KM_PER_H",
        },
        UnitBinding {
            unit: Self::M_PER_S,
            rust: "M_PER_S",
            cpp: "MPerS",
            python: "M_PER_S",
        },
        UnitBinding {
            unit: Self::MM_PER_S,
            rust: "MM_PER_S",
            cpp: "MmPerS",
            python: "MM_PER_S",
        },
        UnitBinding {
            unit: Self::CM_PER_S,
            rust: "CM_PER_S",
            cpp: "CmPerS",
            python: "CM_PER_S",
        },
        UnitBinding {
            unit: Self::KN,
            rust: "KN",
            cpp: "Kn",
            python: "KN",
        },
        UnitBinding {
            unit: Self::M_PER_S2,
            rust: "M_PER_S2",
            cpp: "MPerS2",
            python: "M_PER_S2",
        },
        UnitBinding {
            unit: Self::MM_PER_S2,
            rust: "MM_PER_S2",
            cpp: "MmPerS2",
            python: "MM_PER_S2",
        },
        UnitBinding {
            unit: Self::CM_PER_S2,
            rust: "CM_PER_S2",
            cpp: "CmPerS2",
            python: "CM_PER_S2",
        },
        UnitBinding {
            unit: Self::G,
            rust: "G",
            cpp: "G",
            python: "G",
        },
        UnitBinding {
            unit: Self::UG,
            rust: "UG",
            cpp: "Ug",
            python: "UG",
        },
        UnitBinding {
            unit: Self::MG,
            rust: "MG",
            cpp: "Mg",
            python: "MG",
        },
        UnitBinding {
            unit: Self::KM_PER_H_PER_S,
            rust: "KM_PER_H_PER_S",
            cpp: "KmPerHPerS",
            python: "KM_PER_H_PER_S",
        },
        UnitBinding {
            unit: Self::NS,
            rust: "NS",
            cpp: "Ns",
            python: "NS",
        },
        UnitBinding {
            unit: Self::US,
            rust: "US",
            cpp: "Us",
            python: "US",
        },
        UnitBinding {
            unit: Self::MS,
            rust: "MS",
            cpp: "Ms",
            python: "MS",
        },
        UnitBinding {
            unit: Self::S,
            rust: "S",
            cpp: "S",
            python: "S",
        },
        UnitBinding {
            unit: Self::MIN,
            rust: "MIN",
            cpp: "Min",
            python: "MIN",
        },
        UnitBinding {
            unit: Self::H,
            rust: "H",
            cpp: "H",
            python: "H",
        },
        UnitBinding {
            unit: Self::PERCENT,
            rust: "PERCENT",
            cpp: "Percent",
            python: "PERCENT",
        },
        UnitBinding {
            unit: Self::PER_S,
            rust: "PER_S",
            cpp: "PerS",
            python: "PER_S",
        },
        UnitBinding {
            unit: Self::PER_MIN,
            rust: "PER_MIN",
            cpp: "PerMin",
            python: "PER_MIN",
        },
        UnitBinding {
            unit: Self::PER_H,
            rust: "PER_H",
            cpp: "PerH",
            python: "PER_H",
        },
    ];

    /// Canonical metadata spelling for a recognized unit, as stored in a
    /// `.gtd` file.
    pub fn label(self) -> &'static str {
        self.label
    }

    /// Iterate over every recognized channel unit in stable catalog order.
    pub fn recognized() -> impl ExactSizeIterator<Item = Self> {
        Self::BINDINGS.iter().map(|binding| binding.unit)
    }

    const fn base(base: BaseUnit) -> Self {
        Self {
            prefix: None,
            base,
            label: base.text(),
        }
    }

    const fn with_label(prefix: SiPrefix, base: BaseUnit, label: &'static str) -> Self {
        Self {
            prefix: Some(prefix),
            base,
            label,
        }
    }

    fn from_parts(prefix: SiPrefix, base: BaseUnit) -> Option<Self> {
        Self::recognized().find(|unit| unit.prefix == Some(prefix) && unit.base == base)
    }

    /// The quantity this unit measures, which decides what a query may compare
    /// it against.
    pub fn quantity(self) -> PhysicalQuantity {
        self.base.quantity()
    }

    /// Factor from a value in this unit to its quantity's base unit.
    ///
    /// [`PhysicalQuantity`] lists the base unit of each quantity. A rate is
    /// based on per minute and a ratio on the unit fraction:
    ///
    /// ```
    /// use geotrace_sdk_units::Unit;
    ///
    /// assert_eq!(Unit::PER_S.to_base(), 60.0);
    /// assert_eq!(Unit::PERCENT.to_base(), 0.01);
    /// ```
    pub fn to_base(self) -> f64 {
        self.prefix.map_or(1.0, SiPrefix::factor) * self.base.to_base()
    }

    /// Factor from a value in the quantity's base unit to this unit, the
    /// reciprocal of [`Unit::to_base`].
    pub fn from_base(self) -> f64 {
        1.0 / self.to_base()
    }

    /// Canonical spelling as a lightweight display value.
    ///
    /// Formats exactly like the [`fmt::Display`] impl. [`Unit::label`] gives
    /// the same spelling as a `&'static str`.
    pub fn text(self) -> UnitText {
        UnitText(self)
    }

    /// The suggestion spelling of this unit, or [`None`] for a unit outside
    /// [`Unit::CANONICAL`].
    ///
    /// The query editor filters its unit completions through this, which leaves
    /// out the prefixed sensor scales it still parses.
    pub fn canonical_text(self) -> Option<&'static str> {
        match (self.prefix, self.base) {
            (None, base) => Some(base.text()),
            (Some(SiPrefix::Kilo), BaseUnit::M) => Some("km"),
            (Some(SiPrefix::Milli), BaseUnit::S) => Some("ms"),
            _ => None,
        }
    }

    /// Parse one already-lexed identifier: a base spelling (`deg`, `kn`,
    /// `kmh`), or an SI prefix on a base that accepts it (`mm`, `ug`, `µs`).
    ///
    /// The identifier is matched as written. [`Unit::from_label`] handles
    /// trimming, aliases, and compound spellings.
    pub fn from_ident(ident: &str) -> Option<Self> {
        if let Some(base) = BaseUnit::from_ident(ident) {
            return Some(Self::base(base));
        }
        let (prefix, rest) = SiPrefix::split(ident)?;
        let base = BaseUnit::from_ident(rest)?;
        base.accepts_prefix(prefix)
            .then(|| Self::from_parts(prefix, base))
            .flatten()
    }

    /// Parse a compound spelling from two already-lexed identifiers: `km` and
    /// `h`, `m` and `s`, `m` and `s2`, or an SI prefix on the first where the
    /// catalog has that scale (`mm` and `s`, `cm` and `s2`).
    pub fn from_pair(first: &str, second: &str) -> Option<Self> {
        let exact = match (first, second) {
            ("km", "h") => Some(BaseUnit::KmPerH),
            ("m", "s") => Some(BaseUnit::MPerS),
            ("m", "s2") => Some(BaseUnit::MPerS2),
            _ => None,
        };
        if let Some(base) = exact {
            return Some(Self::base(base));
        }
        let (prefix, rest) = SiPrefix::split(first)?;
        let base = match (rest, second) {
            ("m", "s") => BaseUnit::MPerS,
            ("m", "s2") => BaseUnit::MPerS2,
            _ => return None,
        };
        base.accepts_prefix(prefix)
            .then(|| Self::from_parts(prefix, base))
            .flatten()
    }

    /// Parse the one three-identifier spelling in the catalog: `km`, `h` and
    /// `s`, which is [`Unit::KM_PER_H_PER_S`].
    pub fn from_triple(first: &str, second: &str, third: &str) -> Option<Self> {
        match (first, second, third) {
            ("km", "h", "s") => Some(Self::base(BaseUnit::KmPerHPerS)),
            _ => None,
        }
    }

    /// Parse a whole channel label: a canonical spelling or an accepted alias.
    ///
    /// Trims the label, maps aliases (`degree`, `metres`, `sec`, `kph`, `mps2`,
    /// `°`, `mG`) to their catalog unit, rewrites `²` and `^2` to `2`, and
    /// splits a compound spelling on `/`. A file reader parses through this
    /// entry point. The [`FromStr`] impl is the same parse with a
    /// [`UnitParseError`] in place of [`None`].
    pub fn from_label(label: &str) -> Option<Self> {
        let normalized = normalize_label(label);
        let exact = match normalized.as_str() {
            "%" => Some(Self::PERCENT),
            "per s" => Some(Self::PER_S),
            "per min" => Some(Self::PER_MIN),
            "per h" => Some(Self::PER_H),
            _ => None,
        };
        if exact.is_some() {
            return exact;
        }
        let parts: Vec<&str> = normalized.split('/').collect();
        match parts.as_slice() {
            [a] => Self::from_ident(a),
            [a, b] => Self::from_pair(a, b),
            [a, b, c] => Self::from_triple(a, b, c),
            _ => None,
        }
    }
}

impl fmt::Display for Unit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label)
    }
}

/// The canonical spelling of a [`Unit`], returned by [`Unit::text`].
#[derive(Debug, Clone, Copy)]
pub struct UnitText(Unit);

impl fmt::Display for UnitText {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// Parses with [`Unit::from_label`], reporting
/// [`UnitParseError::Unrecognized`] for a label outside the catalog.
impl FromStr for Unit {
    type Err = UnitParseError;

    fn from_str(label: &str) -> Result<Self, Self::Err> {
        Self::from_label(label).ok_or_else(|| UnitParseError::Unrecognized {
            label: label.to_owned(),
        })
    }
}

/// A deliberate display-only unit label GeoTrace does not understand.
///
/// It is written and read like any unit, but it has no quantity and no
/// conversion factor, so a query reads its channel's values as plain numbers.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CustomUnit(String);

impl CustomUnit {
    /// Validate a label as a custom unit, storing it with surrounding
    /// whitespace trimmed.
    ///
    /// ```
    /// use geotrace_sdk_units::CustomUnit;
    ///
    /// let score = CustomUnit::new("  vendor score  ")?;
    /// assert_eq!(score.as_str(), "vendor score");
    /// # Ok::<(), geotrace_sdk_units::UnitParseError>(())
    /// ```
    ///
    /// # Errors
    ///
    /// [`UnitParseError::EmptyCustom`] when nothing is left after trimming,
    /// [`UnitParseError::CustomControlCharacter`] when the label holds a
    /// control character, and [`UnitParseError::CustomIsRecognized`] when the
    /// label spells a catalog unit through any accepted alias: `m/s²` has to be
    /// declared as [`Unit::M_PER_S2`].
    pub fn new(label: impl Into<String>) -> Result<Self, UnitParseError> {
        let label = label.into();
        let trimmed = label.trim();
        if trimmed.is_empty() {
            return Err(UnitParseError::EmptyCustom);
        }
        if trimmed.chars().any(char::is_control) {
            return Err(UnitParseError::CustomControlCharacter);
        }
        if Unit::from_label(trimmed).is_some() {
            return Err(UnitParseError::CustomIsRecognized {
                label: trimmed.to_owned(),
            });
        }
        Ok(Self(trimmed.to_owned()))
    }

    /// The label as stored, already trimmed.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for CustomUnit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum ChannelUnitValue {
    Recognized(Unit),
    Custom(CustomUnit),
    Legacy(String),
}

/// The classification of a channel unit read from a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChannelUnitKind {
    /// A [`Unit`] from the catalog, with a quantity and a conversion factor.
    Recognized,
    /// A [`CustomUnit`], displayed verbatim and dimensionless in queries.
    Custom,
    /// Losslessly preserved metadata that is not valid writer input.
    Legacy,
}

/// A recognized, convertible unit or an explicit display-only custom label.
///
/// The write path has three constructors: [`ChannelUnit::recognized`] (and the
/// equivalent `From<Unit>`), [`ChannelUnit::custom`], and the [`FromStr`] impl,
/// which yields a recognized unit only. The read path adds
/// [`ChannelUnit::from_file_label`], the one constructor that accepts every
/// label and the only source of [`ChannelUnitKind::Legacy`].
///
/// ```
/// use geotrace_sdk_units::{ChannelUnit, ChannelUnitKind, Unit};
///
/// assert_eq!(ChannelUnit::from(Unit::KM_PER_H).label(), "km/h");
/// assert_eq!(ChannelUnit::from_file_label("kph").kind(), ChannelUnitKind::Recognized);
///
/// let legacy = ChannelUnit::from_file_label(" trailing ");
/// assert_eq!(legacy.kind(), ChannelUnitKind::Legacy);
/// assert_eq!(legacy.label(), " trailing ");
/// assert!(!legacy.is_writable());
/// ```
///
/// Malformed legacy metadata can only be produced by [ChannelUnit::from_file_label].
/// Its private representation cannot be constructed as writable metadata.
///
/// ```compile_fail
/// use geotrace_sdk_units::ChannelUnit;
/// let _ = ChannelUnit("legacy".to_owned());
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ChannelUnit(ChannelUnitValue);

impl ChannelUnit {
    /// A catalog unit, which a query converts and compares.
    pub fn recognized(unit: Unit) -> Self {
        Self(ChannelUnitValue::Recognized(unit))
    }

    /// A display-only label for a unit the catalog does not cover.
    ///
    /// # Errors
    ///
    /// The [`CustomUnit::new`] errors, including
    /// [`UnitParseError::CustomIsRecognized`] for a label that names a catalog
    /// unit once aliases are resolved.
    pub fn custom(label: impl Into<String>) -> Result<Self, UnitParseError> {
        CustomUnit::new(label)
            .map(ChannelUnitValue::Custom)
            .map(Self)
    }

    /// Read existing file metadata, keeping every label.
    ///
    /// A label naming a catalog unit through any accepted alias becomes
    /// [`ChannelUnitKind::Recognized`], another label that survives
    /// [`CustomUnit::new`] unchanged becomes [`ChannelUnitKind::Custom`], and
    /// the rest - untrimmed, empty, or holding a control character - becomes
    /// [`ChannelUnitKind::Legacy`] with its bytes intact.
    pub fn from_file_label(label: impl Into<String>) -> Self {
        let label = label.into();
        match Unit::from_label(&label) {
            Some(unit) => Self::recognized(unit),
            None => match CustomUnit::new(label.clone()) {
                Ok(unit) if unit.as_str() == label => Self(ChannelUnitValue::Custom(unit)),
                Ok(_) | Err(_) => Self(ChannelUnitValue::Legacy(label)),
            },
        }
    }

    /// Which of the three kinds this unit is.
    pub fn kind(&self) -> ChannelUnitKind {
        match self.0 {
            ChannelUnitValue::Recognized(_) => ChannelUnitKind::Recognized,
            ChannelUnitValue::Custom(_) => ChannelUnitKind::Custom,
            ChannelUnitValue::Legacy(_) => ChannelUnitKind::Legacy,
        }
    }

    /// The spelling stored in file metadata, whatever the kind.
    pub fn label(&self) -> &str {
        match &self.0 {
            ChannelUnitValue::Recognized(unit) => unit.label(),
            ChannelUnitValue::Custom(unit) => unit.as_str(),
            ChannelUnitValue::Legacy(label) => label,
        }
    }

    /// Whether this unit may be attached to a channel being written.
    ///
    /// False for exactly [`ChannelUnitKind::Legacy`], which a channel builder
    /// rejects.
    pub fn is_writable(&self) -> bool {
        self.kind() != ChannelUnitKind::Legacy
    }

    /// The catalog unit behind this, or [`None`] for a custom or legacy label.
    pub fn as_recognized(&self) -> Option<Unit> {
        match self.0 {
            ChannelUnitValue::Recognized(unit) => Some(unit),
            ChannelUnitValue::Custom(_) | ChannelUnitValue::Legacy(_) => None,
        }
    }
}

impl fmt::Display for ChannelUnit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// Equivalent to [`ChannelUnit::recognized`].
impl From<Unit> for ChannelUnit {
    fn from(unit: Unit) -> Self {
        Self::recognized(unit)
    }
}

/// Parses a recognized unit label only. A label outside the catalog is a
/// [`UnitParseError::Unrecognized`]: pass it to [`ChannelUnit::custom`] to
/// store it verbatim.
impl FromStr for ChannelUnit {
    type Err = UnitParseError;

    fn from_str(label: &str) -> Result<Self, Self::Err> {
        label.parse::<Unit>().map(Self::recognized)
    }
}

/// Why a label cannot be used as a channel unit.
///
/// [`Unit::from_str`] and [`ChannelUnit::from_str`] report
/// [`Unrecognized`](Self::Unrecognized). The other three come from
/// [`CustomUnit::new`] and [`ChannelUnit::custom`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum UnitParseError {
    #[error("unrecognized channel unit {label:?}; use ChannelUnit::custom for a display-only unit")]
    Unrecognized { label: String },
    #[error("a custom channel unit cannot be empty")]
    EmptyCustom,
    #[error("a custom channel unit cannot contain control characters")]
    CustomControlCharacter,
    #[error("channel unit {label:?} is recognized; use ChannelUnit::recognized instead")]
    CustomIsRecognized { label: String },
}

fn normalize_label(label: &str) -> String {
    match label.trim() {
        "°" | "degree" | "degrees" => "deg".to_owned(),
        "meter" | "meters" | "metre" | "metres" => "m".to_owned(),
        "second" | "seconds" | "sec" => "s".to_owned(),
        "hour" | "hours" | "hr" => "h".to_owned(),
        "percent" => "%".to_owned(),
        "kph" | "kmph" => "km/h".to_owned(),
        "mps" => "m/s".to_owned(),
        "mps2" | "m/s/s" => "m/s2".to_owned(),
        "G" => "g".to_owned(),
        "mG" => "mg".to_owned(),
        // Collapse the superscript first, so a second pass over an already
        // normalized label is a no-op (idempotent): `²` -> `^2` -> `2` in one
        // pass rather than leaving a `^2` for the next call to reduce.
        value => value.replace('²', "2").replace("^2", "2"),
    }
}

#[cfg(test)]
mod tests {
    use std::fmt::Write as _;

    use proptest::prelude::*;
    use strum::{EnumCount as _, IntoEnumIterator as _};

    use super::*;

    #[test]
    fn recognized_channel_units_scale_to_base() {
        let mg = Unit::from_label("mg").expect("mg is recognized");
        assert_eq!(mg.quantity(), PhysicalQuantity::Acceleration);
        assert!((mg.to_base() - 0.009_806_65).abs() < 1e-12);
        assert_eq!(mg.to_string(), "mg");
    }

    #[test]
    fn file_labels_parse_aliases_before_using_the_escape_hatch() {
        for (label, expected) in [
            ("µg", "ug"),
            ("μg", "ug"),
            ("m/s²", "m/s2"),
            ("degrees", "deg"),
            ("metres", "m"),
            ("m/s^2", "m/s2"),
            ("m/s/s", "m/s2"),
            ("mG", "mg"),
            ("kph", "km/h"),
        ] {
            let parsed = ChannelUnit::from_file_label(label);
            assert_eq!(parsed.to_string(), expected);
            assert_eq!(parsed.kind(), ChannelUnitKind::Recognized);
        }
    }

    #[test]
    fn custom_units_are_explicit_and_dimensionless() {
        assert!(matches!(
            "rpm".parse::<ChannelUnit>(),
            Err(UnitParseError::Unrecognized { .. })
        ));
        let custom = ChannelUnit::custom("rpm").expect("valid custom label");
        assert_eq!(custom.to_string(), "rpm");
        assert_eq!(custom.as_recognized(), None);
        assert_eq!(ChannelUnit::from_file_label("rpm"), custom);
    }

    #[test]
    fn file_reader_preserves_unsupported_labels_losslessly() {
        let label = " future\nunit ";
        let parsed = ChannelUnit::from_file_label(label);
        assert_eq!(parsed.to_string(), label);
        assert_eq!(parsed.kind(), ChannelUnitKind::Legacy);
    }

    #[test]
    fn invalid_custom_labels_are_rejected() {
        assert!(matches!(
            ChannelUnit::custom("  "),
            Err(UnitParseError::EmptyCustom)
        ));
        assert!(matches!(
            ChannelUnit::custom("bad\nunit"),
            Err(UnitParseError::CustomControlCharacter)
        ));
        assert!(matches!(
            ChannelUnit::custom("m/s²"),
            Err(UnitParseError::CustomIsRecognized { .. })
        ));
    }

    #[test]
    fn every_base_has_a_canonical_round_trip() {
        let prefixed_count = BaseUnit::iter()
            .map(|base| {
                SiPrefix::iter()
                    .filter(|prefix| base.accepts_prefix(*prefix))
                    .count()
            })
            .sum::<usize>();
        assert_eq!(Unit::recognized().len(), BaseUnit::COUNT + prefixed_count);
        for unit in Unit::recognized() {
            let text = unit.to_string();
            assert_eq!(Unit::from_label(&text), Some(unit), "{text}");
        }
    }

    #[test]
    fn canonical_catalog_holds_exactly_the_units_with_a_suggestion_spelling() {
        let with_suggestion: Vec<Unit> = Unit::recognized()
            .filter(|unit| unit.canonical_text().is_some())
            .collect();
        assert_eq!(with_suggestion, Unit::CANONICAL.to_vec());
    }

    #[test]
    fn binding_catalog_is_exhaustive_and_canonical() {
        assert_eq!(Unit::BINDINGS.len(), Unit::recognized().len());
        for binding in Unit::BINDINGS {
            assert_eq!(Unit::from_label(binding.unit.label()), Some(binding.unit));
            assert!(!binding.rust.is_empty());
            assert!(!binding.cpp.is_empty());
            assert!(!binding.python.is_empty());
        }
    }

    #[test]
    fn conversions_cover_speed_acceleration_duration_ratio_and_rate() {
        assert!((Unit::KM_PER_H.to_base() - 1.0 / 3.6).abs() < 1e-12);
        assert!((Unit::KN.to_base() - 0.514_444_444_444).abs() < 1e-9);
        assert!((Unit::G.to_base() - 9.806_65).abs() < 1e-9);
        assert!((Unit::MIN.to_base() - 60.0).abs() < f64::EPSILON);
        assert!((Unit::H.to_base() - 3600.0).abs() < f64::EPSILON);
        assert!((Unit::PERCENT.to_base() - 0.01).abs() < f64::EPSILON);
        assert!((Unit::PER_S.to_base() - 60.0).abs() < f64::EPSILON);
        assert!((Unit::PER_MIN.to_base() - 1.0).abs() < f64::EPSILON);
        assert!((Unit::PER_H.to_base() - 1.0 / 60.0).abs() < 1e-12);
    }

    #[test]
    fn curated_prefixes_accept_sensor_scales_and_reject_ambiguous_nonsense() {
        for (label, factor) in [
            ("nm", 1e-9),
            ("um", 1e-6),
            ("µm", 1e-6),
            ("mm", 1e-3),
            ("cm", 1e-2),
            ("km", 1e3),
            ("ns", 1e-9),
            ("us", 1e-6),
            ("ms", 1e-3),
            ("ug", 1e-6 * 9.806_65),
            ("mg", 1e-3 * 9.806_65),
            ("mm/s", 1e-3),
            ("cm/s2", 1e-2),
        ] {
            let parsed = Unit::from_label(label).expect("curated unit");
            assert!((parsed.to_base() - factor).abs() < 1e-12, "{label}");
        }
        for label in ["kg", "cs", "ks", "ng", "cg", "km/s", "nm/s2"] {
            assert_eq!(Unit::from_label(label), None, "{label}");
        }
    }

    #[test]
    fn recognized_unit_catalog_is_stable() {
        let mut catalog = String::new();
        for binding in Unit::BINDINGS {
            let unit = binding.unit;
            writeln!(
                &mut catalog,
                "{} | {} | {} | {} | {:?} | {:.12}",
                unit.label(),
                binding.rust,
                binding.cpp,
                binding.python,
                unit.quantity(),
                unit.to_base()
            )
            .expect("writing to a String cannot fail");
        }
        insta::assert_snapshot!(catalog);
    }

    proptest! {
        #[test]
        fn arbitrary_file_labels_never_panic_and_preserve_unknown_text(label in any::<String>()) {
            let parsed = ChannelUnit::from_file_label(label.clone());
            if Unit::from_label(&label).is_none() {
                prop_assert_eq!(parsed.to_string(), label);
            }
        }

        #[test]
        fn normalization_is_idempotent(label in any::<String>()) {
            let once = normalize_label(&label);
            prop_assert_eq!(normalize_label(&once), once);
        }
    }
}

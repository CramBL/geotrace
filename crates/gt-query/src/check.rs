//! Static checking: dimensions, aggregate placement, parameter requirements.
//!
//! Successful checking produces a [`CheckedQuery`] whose expression tree has
//! literals already converted to base units, so evaluation is plain f64
//! arithmetic. Several error messages here are user-facing UX pinned verbatim
//! by tests - change them deliberately.

use std::collections::HashMap;

use gt_types::DisplayMode;

use crate::Diagnostic;
use crate::ast::{
    BinaryOp, ChannelRef, Expr, Func, NumberLit, ParamDecl, ParamName, Query, Source, Span,
    UnaryOp, Window as AstWindow,
};
use crate::dimension::Dimension;
use crate::fmt::Superscript;
use crate::metric::{Quantity, QueryMetric};
use crate::unit::{self, Unit, example_literal, unit_list};

/// What a query needs to know about one ad-hoc channel to type-check a
/// reference to it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ChannelInfo {
    /// Canonical recognized unit (`"g"`, `"m/s2"`), a custom display label,
    /// or `None`. Custom labels are treated as bare numbers.
    pub unit: Option<String>,
    /// Wrap period in degrees for an angular channel, or `None` for a linear
    /// value. A present period marks the channel circular (a direction).
    pub period_deg: Option<f64>,
    /// Vector component labels (`["x", "y", "z"]`), or empty for a scalar
    /// channel.
    pub components: Vec<String>,
    /// Distinct unit labels found for this name when loaded files disagree.
    /// Empty for a usable schema entry.
    pub conflicting_units: Vec<String>,
}

/// The channels a query may reference, keyed by name. The app builds this from
/// the loaded files; [`check`] resolves each `@name` against it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ChannelSchema {
    channels: HashMap<String, ChannelInfo>,
}

impl ChannelSchema {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a channel. A later insert with the same name replaces the
    /// earlier one.
    pub fn insert(&mut self, name: impl Into<String>, info: ChannelInfo) {
        self.channels.insert(name.into(), info);
    }

    /// Whether the schema holds no channels at all (nothing is loaded).
    pub fn is_empty(&self) -> bool {
        self.channels.is_empty()
    }

    /// Metadata currently registered for `name`.
    pub fn get(&self, name: &str) -> Option<&ChannelInfo> {
        self.channels.get(name)
    }

    /// Every channel as a `(name, info)` pair, in the map's arbitrary order.
    /// Callers that show the channels (completion, hover) sort as they see fit.
    pub(crate) fn iter_unsorted(&self) -> impl Iterator<Item = (&str, &ChannelInfo)> {
        self.channels
            .iter()
            .map(|(name, info)| (name.as_str(), info))
    }
}

/// Resolved `with` parameters, in base units.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Params {
    pub mask_deg: Option<f64>,
    pub snr_drop_db_hz: Option<f64>,
    pub slip_window_s: Option<f64>,
}

/// The window a query aggregates over, resolved from the `window` stage: a fixed
/// point count or a fixed time span. Either way it is a time span anchored at
/// each nav point over which an aggregate reduces the metric's native samples.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Window {
    /// A span of `n` consecutive points: at anchor `i` the points `[i, i+n)` and
    /// the time extent `[t(i), t(i+n-1)]`.
    Count(usize),
    /// A span of a fixed duration in seconds: at anchor `i` the points whose
    /// time lands in `[t(i), t(i) + secs)`. Requires the full duration to fit.
    Duration(f64),
}

/// The resolved pipeline source: the nav points, or a channel's samples by
/// name. The evaluator dispatches its timeline and match granularity on this.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CheckedSource {
    Points,
    Channel(String),
}

/// A query that passed all static checks, ready to run.
#[derive(Debug, Clone)]
pub struct CheckedQuery {
    pub(crate) source: CheckedSource,
    pub(crate) window: Option<Window>,
    pub(crate) predicates: Vec<CExpr>,
    params: Params,
    columns: Vec<QueryMetric>,
    referenced: Vec<QueryMetric>,
    unused_params: Vec<ParamName>,
    mode: DisplayMode,
}

impl CheckedQuery {
    /// The resolved source: the nav points or a channel's samples.
    pub(crate) fn source(&self) -> &CheckedSource {
        &self.source
    }

    /// Whether the source is a channel (its samples are the timeline), rather
    /// than the nav points. The app uses this to route or gate a run.
    pub fn is_channel_source(&self) -> bool {
        matches!(self.source, CheckedSource::Channel(_))
    }

    /// The source channel's name, or `None` for the points source. The app
    /// reads it to gather that channel's timeline for a channel-source run.
    pub fn source_channel(&self) -> Option<&str> {
        match &self.source {
            CheckedSource::Channel(name) => Some(name),
            CheckedSource::Points => None,
        }
    }

    /// The window a windowed query aggregates over, if any.
    pub fn window(&self) -> Option<Window> {
        self.window
    }

    /// The `with` parameters the provider must compute util/slip series with.
    pub fn params(&self) -> Params {
        self.params
    }

    /// Match-table columns, `time` first.
    pub fn columns(&self) -> &[QueryMetric] {
        &self.columns
    }

    /// Every metric the query touches (predicates and table columns), in
    /// first-mention order. Lets the runner compute expensive derived series
    /// (util/slip) only when actually used.
    pub fn referenced_metrics(&self) -> &[QueryMetric] {
        &self.referenced
    }

    /// Declared parameters no referenced metric needs (run-summary note).
    pub fn unused_params(&self) -> &[ParamName] {
        &self.unused_params
    }

    /// How the matches change the map: `draw` halos (the default), `keep`
    /// (only matching points shown), or `hide` (matching points hidden).
    pub fn mode(&self) -> DisplayMode {
        self.mode
    }
}

/// A resolved channel reference: the channel name, and for a vector channel the
/// column of the referenced component (`@accel.x`). `None` is a scalar channel
/// or a whole vector; the checker resolves the component label to its index so
/// evaluation works by column.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ChannelKey {
    pub name: String,
    pub component: Option<usize>,
}

/// Which timeline an aggregate reduces: the window's nav points, or one
/// channel's native samples over the window's time span. A vector channel's
/// components share a clock, so the timeline is the channel by name; each
/// [`CExpr::Channel`]/[`CExpr::Norm`] node projects the column(s) it needs per
/// sample. The checker resolves this once (see [`aggregate_source`]).
#[derive(Debug, Clone)]
pub(crate) enum AggSource {
    Points,
    Channel(String),
}

/// Checked expression: literals in base units, aggregates tagged circular
/// where they run on a direction.
#[derive(Debug, Clone)]
pub(crate) enum CExpr {
    Const(f64),
    Metric(QueryMetric),
    /// A scalar channel or one component of a vector channel. Only ever appears
    /// inside an aggregate, which reduces its native samples over the window
    /// span.
    Channel(ChannelKey),
    /// The Euclidean magnitude of a whole vector channel (`norm(@accel)`), by
    /// name. A per-sample scalar in the channel's own unit, reduced by the
    /// enclosing aggregate.
    Norm(String),
    Agg {
        func: Func,
        circular: bool,
        source: AggSource,
        arg: Box<CExpr>,
    },
    Abs(Box<CExpr>),
    Sqrt(Box<CExpr>),
    Neg(Box<CExpr>),
    Not(Box<CExpr>),
    Cmp {
        op: CmpOp,
        lhs: Box<CExpr>,
        rhs: Box<CExpr>,
    },
    Logic {
        and: bool,
        lhs: Box<CExpr>,
        rhs: Box<CExpr>,
    },
    Arith {
        op: ArithOp,
        lhs: Box<CExpr>,
        rhs: Box<CExpr>,
    },
    Power {
        base: Box<CExpr>,
        exponent: i8,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CmpOp {
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
    Ne,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ArithOp {
    Add,
    Sub,
    Mul,
    Div,
}

/// The static type the checker gives an expression. Dimensionless values carry
/// a [`Kind`] so a count, a ratio, and a bare number stay distinct despite
/// sharing the zero dimension; `Timestamp` and `Condition` stand outside
/// dimensional arithmetic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ValueType {
    Condition,
    Timestamp,
    /// A dimensioned value. The dimension is never dimensionless - that case is
    /// [`ValueType::Dimensionless`]. `circular` marks a direction (a bearing
    /// that wraps at 360) rather than a plain angle, and is only ever set when
    /// the dimension is [`Dimension::ANGLE`].
    Dimensioned {
        dim: Dimension,
        circular: bool,
    },
    Dimensionless(Kind),
}

/// How the language treats a dimensionless value. All three share the zero
/// dimension; the tag keeps them from comparing nonsensically across
/// categories (a count is not a ratio is not a bare number).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// A bare number with no unit, and the result of dimensionless arithmetic.
    Number,
    /// A discrete tally (satellite counts). The only kind `==`/`!=` accept.
    Count,
    /// A percentage-denominated share (satellite utilization).
    Ratio,
}

impl ValueType {
    /// A dimensioned value that never wraps.
    fn linear(dim: Dimension) -> ValueType {
        ValueType::Dimensioned {
            dim,
            circular: false,
        }
    }

    fn is_condition(self) -> bool {
        self == ValueType::Condition
    }
}

/// The `ValueType` of a metric's or parameter's declared [`Quantity`].
fn value_type(quantity: Quantity) -> ValueType {
    match quantity {
        Quantity::Condition => ValueType::Condition,
        Quantity::Timestamp => ValueType::Timestamp,
        Quantity::Count => ValueType::Dimensionless(Kind::Count),
        Quantity::Ratio => ValueType::Dimensionless(Kind::Ratio),
        // Every remaining quantity is a dimensioned value; `Direction` is the
        // only one that wraps. Its dimension is taken from the single source
        // in `Quantity::dimension`.
        _ => match quantity.dimension() {
            Some(dim) => ValueType::Dimensioned {
                dim,
                circular: quantity == Quantity::Direction,
            },
            None => {
                debug_assert!(
                    false,
                    "only Timestamp and Condition lack a dimension, and both matched above"
                );
                ValueType::Dimensionless(Kind::Number)
            }
        },
    }
}

/// The [`Quantity`] a value type names, when it names one. Exotic dimensions (a
/// squared speed, say) and the bare [`Kind::Number`] have no quantity. Lets the
/// error wording and the aggregate result rule reuse [`Quantity`]-keyed logic.
fn named_quantity(vt: ValueType) -> Option<Quantity> {
    Some(match vt {
        ValueType::Condition => Quantity::Condition,
        ValueType::Timestamp => Quantity::Timestamp,
        ValueType::Dimensionless(Kind::Count) => Quantity::Count,
        ValueType::Dimensionless(Kind::Ratio) => Quantity::Ratio,
        ValueType::Dimensionless(Kind::Number) => return None,
        ValueType::Dimensioned { dim, circular } => return named_dimension(dim, circular),
    })
}

fn named_dimension(dim: Dimension, circular: bool) -> Option<Quantity> {
    Some(if dim == Dimension::ANGLE {
        if circular {
            Quantity::Direction
        } else {
            Quantity::Angle
        }
    } else if dim == Dimension::SPEED {
        Quantity::Speed
    } else if dim == Dimension::ACCELERATION {
        Quantity::Acceleration
    } else if dim == Dimension::LENGTH {
        Quantity::Length
    } else if dim == Dimension::TIME {
        Quantity::Duration
    } else if dim == Dimension::RATE {
        Quantity::Rate
    } else {
        return None;
    })
}

/// A readable name for a value type in an error message. Every value has one:
/// named quantities use their name, a bare number is "number", and an exotic
/// dimension is described from its exponents (see [`dimension_label`]).
fn type_label(vt: ValueType) -> String {
    match vt {
        ValueType::Dimensionless(Kind::Number) => "number".to_owned(),
        ValueType::Dimensioned { dim, circular } => dimension_label(dim, circular),
        // Count, Ratio, Timestamp, and Condition all name a quantity.
        other => named_quantity(other).map_or_else(|| "value".to_owned(), |q| q.to_string()),
    }
}

/// A label for a dimension: its quantity name (`speed`), a whole power of one
/// (`speed²`, the shape `var` and the power operator produce), or a fraction of
/// the base dimensions (`length²/time`).
fn dimension_label(dim: Dimension, circular: bool) -> String {
    if let Some(quantity) = named_dimension(dim, circular) {
        return quantity.to_string();
    }
    power_of_named(dim).unwrap_or_else(|| base_dimension_fraction(dim))
}

/// `speed²` or `acceleration³` when the dimension is a square or cube of a
/// named quantity.
fn power_of_named(dim: Dimension) -> Option<String> {
    for n in [2i8, 3] {
        if dim.length % n == 0 && dim.time % n == 0 && dim.angle % n == 0 {
            let root = Dimension {
                length: dim.length / n,
                time: dim.time / n,
                angle: dim.angle / n,
            };
            if let Some(quantity) = named_dimension(root, false) {
                return Some(format!("{quantity}{}", Superscript(n)));
            }
        }
    }
    None
}

/// The base-dimension fraction, e.g. `length²/time`: positive exponents in the
/// numerator, negative in the denominator.
fn base_dimension_fraction(dim: Dimension) -> String {
    let bases = [
        ("length", dim.length),
        ("time", dim.time),
        ("angle", dim.angle),
    ];
    let group = |numerator: bool| -> Vec<String> {
        bases
            .iter()
            .filter_map(|&(name, exp)| {
                let included = if numerator { exp > 0 } else { exp < 0 };
                included.then(|| base_with_exponent(name, exp.saturating_abs()))
            })
            .collect()
    };
    let numerator = group(true);
    let denominator = group(false);
    let numerator = if numerator.is_empty() {
        "1".to_owned()
    } else {
        numerator.join("·")
    };
    if denominator.is_empty() {
        numerator
    } else {
        format!("{numerator}/{}", denominator.join("·"))
    }
}

fn base_with_exponent(name: &str, exponent: i8) -> String {
    if exponent == 1 {
        name.to_owned()
    } else {
        format!("{name}{}", Superscript(exponent))
    }
}

/// The value type of a scalar channel from its schema entry. The unit fixes the
/// dimension; an angular channel with a period wraps, so it is a direction
/// rather than a plain angle.
///
/// A custom or absent unit resolves to a bare number. SDK writers require the
/// custom-unit escape hatch explicitly; legacy file metadata can still arrive
/// here as a preserved custom label.
fn channel_value_type(info: &ChannelInfo) -> ValueType {
    let Some(quantity) = info
        .unit
        .as_deref()
        .and_then(Unit::from_label)
        .map(unit::quantity)
    else {
        return ValueType::Dimensionless(Kind::Number);
    };
    let vt = value_type(quantity);
    // A period only means "wraps" for an angle; ignore it otherwise. This keeps
    // the invariant that `circular` is set only when the dimension is ANGLE.
    if info.period_deg.is_some()
        && matches!(vt, ValueType::Dimensioned { dim, .. } if dim == Dimension::ANGLE)
    {
        ValueType::Dimensioned {
            dim: Dimension::ANGLE,
            circular: true,
        }
    } else {
        vt
    }
}

/// Normalise a computed dimension into a value type: a dimensionless result is
/// a bare [`Kind::Number`], anything else a linear dimensioned value.
fn dimensioned(dim: Dimension) -> ValueType {
    if dim.is_dimensionless() {
        ValueType::Dimensionless(Kind::Number)
    } else {
        ValueType::linear(dim)
    }
}

/// The column a channel reference selects, validated against the channel's
/// shape. `None` for a scalar channel (`@name`); `Some(i)` for a vector
/// component (`@name.x`). A component on a scalar, an unknown component, or a
/// bare vector (which has no scalar value) each error with a hint.
fn resolve_component(c: &ChannelRef, info: &ChannelInfo) -> Result<Option<usize>, Diagnostic> {
    match &c.component {
        Some(label) => {
            if info.components.is_empty() {
                return Err(err_hint(
                    c.span,
                    format!("@{} is not a vector channel", c.name),
                    format!("reference the scalar @{} without a component", c.name),
                ));
            }
            match info.components.iter().position(|comp| comp == label) {
                Some(index) => Ok(Some(index)),
                None => Err(err_hint(
                    c.span,
                    format!("@{} has no component {label}", c.name),
                    format!("its components are {}", info.components.join(", ")),
                )),
            }
        }
        // A scalar channel has one value; a whole vector has none until a
        // component is named (norm over the vector lands later).
        None if info.components.is_empty() => Ok(None),
        None => {
            let first = info.components.first().map_or("x", String::as_str);
            Err(err_hint(
                c.span,
                format!("@{} is a vector channel", c.name),
                format!("reference a component like @{}.{first}", c.name),
            ))
        }
    }
}

/// Which timeline an aggregate argument reduces, rejecting anything that mixes
/// two. An aggregate reduces one clock at a time: the window's nav points, or a
/// single channel's samples. A vector channel's components share a clock, so
/// several components of one channel are fine; two different channels, or a
/// channel alongside a per-point metric, are on independent clocks and cannot
/// be combined per element.
fn aggregate_source(arg: &CExpr, span: Span) -> Result<AggSource, Diagnostic> {
    let mut channels: Vec<&str> = Vec::new();
    let mut has_metric = false;
    collect_timelines(arg, &mut channels, &mut has_metric);
    match channels.as_slice() {
        [] => Ok(AggSource::Points),
        [name] if has_metric => Err(err_hint(
            span,
            format!("cannot mix @{name} with a per-point metric"),
            "a channel and a nav-point metric are on separate clocks",
        )),
        [name] => Ok(AggSource::Channel((*name).to_owned())),
        _ => Err(err_hint(
            span,
            "an aggregate reduces one channel at a time",
            "split it into separate aggregates, one per channel",
        )),
    }
}

/// Collect the distinct channel names an expression reads and whether it reads
/// any per-point metric, walking every value-carrying node. Components of one
/// vector channel collapse to that channel's single name (one timeline).
fn collect_timelines<'a>(expr: &'a CExpr, channels: &mut Vec<&'a str>, has_metric: &mut bool) {
    match expr {
        CExpr::Channel(key) => {
            if !channels.contains(&key.name.as_str()) {
                channels.push(&key.name);
            }
        }
        CExpr::Norm(name) => {
            if !channels.contains(&name.as_str()) {
                channels.push(name);
            }
        }
        CExpr::Metric(_) => *has_metric = true,
        CExpr::Abs(inner) | CExpr::Sqrt(inner) | CExpr::Neg(inner) | CExpr::Not(inner) => {
            collect_timelines(inner, channels, has_metric);
        }
        CExpr::Power { base, .. } => collect_timelines(base, channels, has_metric),
        CExpr::Arith { lhs, rhs, .. }
        | CExpr::Cmp { lhs, rhs, .. }
        | CExpr::Logic { lhs, rhs, .. } => {
            collect_timelines(lhs, channels, has_metric);
            collect_timelines(rhs, channels, has_metric);
        }
        // A constant carries no timeline, and aggregates never nest (rejected
        // before this runs), so neither contributes a channel or metric.
        CExpr::Const(_) | CExpr::Agg { .. } => {}
    }
}

fn err(span: Span, message: impl Into<String>) -> Diagnostic {
    Diagnostic::new(span, message)
}

fn err_hint(span: Span, message: impl Into<String>, help: impl Into<String>) -> Diagnostic {
    Diagnostic::with_hint(span, message, help)
}

pub fn check(query: &Query, schema: &ChannelSchema) -> Result<CheckedQuery, Diagnostic> {
    let params = resolve_params(&query.params)?;
    let window = match query.window {
        None => None,
        // The conversion only fails on targets where usize is narrower than
        // u64; kept as a defensive branch rather than an assumption.
        Some(AstWindow::Count { len, span }) => {
            let n = usize::try_from(len).map_err(|_overflow| err(span, "window is too large"))?;
            Some(Window::Count(n))
        }
        Some(AstWindow::Duration { value, unit, span }) => {
            if unit::quantity(unit) != Quantity::Duration {
                return Err(err_hint(
                    span,
                    "a window duration needs a time unit",
                    "try s, min, or h, e.g. window 15 s",
                ));
            }
            // Reject zero, negative, and the infinity an overlong digit run
            // lexes to, so a duration is always a real positive span.
            if !(value > 0.0 && value.is_finite()) {
                return Err(err(span, "a window duration must be a positive number"));
            }
            Some(Window::Duration(value * unit.to_base()))
        }
    };

    // Resolve the source: `points`, or a whole channel whose samples become the
    // timeline. A component (`@accel.x`) is not a source - the whole channel is.
    let source = match &query.source {
        Source::Points => CheckedSource::Points,
        Source::Channel(c) => {
            if c.component.is_some() {
                return Err(err_hint(
                    c.span,
                    "a channel source is a whole channel",
                    format!("use @{} as the source", c.name),
                ));
            }
            let Some(info) = schema.get(&c.name) else {
                return Err(err(c.span, format!("no such channel @{}", c.name)));
            };
            reject_unit_conflict(&c.name, info, c.span)?;
            CheckedSource::Channel(c.name.clone())
        }
    };

    let mut checker = Checker {
        windowed: window.is_some(),
        source_channel: match &source {
            CheckedSource::Points => None,
            CheckedSource::Channel(name) => Some(name.as_str()),
        },
        referenced: Vec::new(),
        schema,
    };
    let mut predicates = Vec::new();
    for predicate in &query.predicates {
        let (value_type, cexpr) = checker.expr(predicate, false)?;
        if !value_type.is_condition() {
            return Err(err(
                predicate.span(),
                "where needs a condition, e.g. velocity > 30 km/h",
            ));
        }
        predicates.push(cexpr);
    }

    if let Some(table) = &query.table {
        for column in &table.columns {
            checker.reject_metric_on_channel_source(column.metric, column.span)?;
            checker.referenced.push((column.metric, column.span));
        }
    }
    let occurrences = checker.referenced;
    require_params(&occurrences, params)?;

    let referenced: Vec<QueryMetric> = dedup_metrics(occurrences.iter().map(|(m, _)| *m));
    let columns = build_columns(query, &referenced);
    let unused_params = unused_params(&query.params, &referenced);

    Ok(CheckedQuery {
        source,
        window,
        predicates,
        params,
        columns,
        referenced,
        unused_params,
        // No display stage means the implicit `draw`.
        mode: query.mode.map(|stage| stage.mode).unwrap_or_default(),
    })
}

fn resolve_params(decls: &[ParamDecl]) -> Result<Params, Diagnostic> {
    let mut params = Params::default();
    for decl in decls {
        // Duplicate is reported before any unit mismatch, matching the pinned
        // error order.
        let already_set = match decl.name {
            ParamName::Mask => params.mask_deg.is_some(),
            ParamName::SnrDrop => params.snr_drop_db_hz.is_some(),
            ParamName::SlipWindow => params.slip_window_s.is_some(),
        };
        if already_set {
            return Err(declared_twice(decl));
        }
        let base = param_value_base(decl)?;
        match decl.name {
            ParamName::Mask => params.mask_deg = Some(base),
            ParamName::SnrDrop => params.snr_drop_db_hz = Some(base),
            ParamName::SlipWindow => params.slip_window_s = Some(base),
        }
    }
    Ok(params)
}

/// Resolve a parameter's literal to its base-unit value, checking the unit
/// against the parameter's [`ParamName::value_quantity`] (the single source
/// shared with autocomplete).
fn param_value_base(decl: &ParamDecl) -> Result<f64, Diagnostic> {
    let lit = decl.value;
    match decl.name.value_quantity() {
        // A bare number, like snr_drop: a unit is a mistake.
        None => {
            if lit.unit.is_some() {
                return Err(err(decl.span, format!("{} takes a bare number", decl.name)));
            }
            Ok(lit.value)
        }
        Some(quantity) => {
            let unit = lit
                .unit
                .filter(|u| unit::quantity(*u) == quantity)
                .ok_or_else(|| err(decl.span, param_unit_help(decl.name)))?;
            Ok(lit.value * unit.to_base())
        }
    }
}

/// The "needs a …" message when a parameter's value is missing its unit.
fn param_unit_help(name: ParamName) -> &'static str {
    match name {
        ParamName::Mask => "mask needs an angle, e.g. mask 15 deg",
        ParamName::SlipWindow => "slip_window needs a duration, e.g. slip_window 5 min",
        // snr_drop has no quantity, so it never reaches this branch.
        ParamName::SnrDrop => "snr_drop takes a bare number",
    }
}

fn declared_twice(decl: &ParamDecl) -> Diagnostic {
    err(decl.span, format!("{} is declared twice", decl.name))
}

/// util_* needs mask; slip_* needs mask, snr_drop, and slip_window.
fn require_params(occurrences: &[(QueryMetric, Span)], params: Params) -> Result<(), Diagnostic> {
    for &(metric, span) in occurrences {
        let mut missing = Vec::new();
        if (metric.is_util() || metric.is_slip()) && params.mask_deg.is_none() {
            missing.push(ParamName::Mask);
        }
        if metric.is_slip() {
            if params.snr_drop_db_hz.is_none() {
                missing.push(ParamName::SnrDrop);
            }
            if params.slip_window_s.is_none() {
                missing.push(ParamName::SlipWindow);
            }
        }
        if missing.is_empty() {
            continue;
        }
        let what = match missing.as_slice() {
            [ParamName::Mask] => "an elevation mask".to_owned(),
            names => human_list(names),
        };
        let examples: Vec<&str> = missing
            .iter()
            .map(|name| match name {
                ParamName::Mask => "mask 15 deg",
                ParamName::SnrDrop => "snr_drop 10",
                ParamName::SlipWindow => "slip_window 5 min",
            })
            .collect();
        return Err(Diagnostic {
            span,
            message: format!("{metric} needs {what}"),
            help: Some(format!("add: | with {}", examples.join(", "))),
        });
    }
    Ok(())
}

fn human_list(names: &[ParamName]) -> String {
    let names: Vec<String> = names.iter().map(ToString::to_string).collect();
    match names.as_slice() {
        [] => String::new(),
        [single] => single.clone(),
        [first, second] => format!("{first} and {second}"),
        [head @ .., tail] => format!("{}, and {tail}", head.join(", ")),
    }
}

/// Ordered dedup, preserving first mention.
fn dedup_metrics(metrics: impl Iterator<Item = QueryMetric>) -> Vec<QueryMetric> {
    let mut out = Vec::new();
    for metric in metrics {
        if !out.contains(&metric) {
            out.push(metric);
        }
    }
    out
}

/// `time` always comes first; explicit `table` columns win over the default
/// of every referenced metric in first-mention order.
fn build_columns(query: &Query, referenced: &[QueryMetric]) -> Vec<QueryMetric> {
    let listed: Vec<QueryMetric> = match &query.table {
        Some(table) => table.columns.iter().map(|c| c.metric).collect(),
        None => referenced.to_vec(),
    };
    dedup_metrics(std::iter::once(QueryMetric::Time).chain(listed))
}

fn unused_params(decls: &[ParamDecl], referenced: &[QueryMetric]) -> Vec<ParamName> {
    let uses_util = referenced.iter().any(|m| m.is_util());
    let uses_slip = referenced.iter().any(|m| m.is_slip());
    decls
        .iter()
        .map(|d| d.name)
        .filter(|name| match name {
            ParamName::Mask => !uses_util && !uses_slip,
            ParamName::SnrDrop | ParamName::SlipWindow => !uses_slip,
        })
        .collect()
}

struct Checker<'a> {
    windowed: bool,
    /// The source channel's name on a channel source, else `None` for the
    /// points source. On a channel source the timeline is this channel's
    /// samples, so a reference to it is per-sample (the match granularity).
    source_channel: Option<&'a str>,
    /// First occurrence of each referenced metric, in source order.
    referenced: Vec<(QueryMetric, Span)>,
    schema: &'a ChannelSchema,
}

impl Checker<'_> {
    /// Reject a nav-point metric on a channel source, wherever it appears (a
    /// `where` expression or a `table` column). Interpolating a nav metric onto
    /// the channel's sample times is a later step.
    fn reject_metric_on_channel_source(
        &self,
        metric: QueryMetric,
        span: Span,
    ) -> Result<(), Diagnostic> {
        if let Some(source) = self.source_channel {
            return Err(err_hint(
                span,
                format!("{metric} is not available on a channel source"),
                format!("query the points source, or drop {metric} from @{source}"),
            ));
        }
        Ok(())
    }

    fn expr(&mut self, expr: &Expr, in_agg: bool) -> Result<(ValueType, CExpr), Diagnostic> {
        match expr {
            Expr::Number(lit) => Ok((literal_type(lit), CExpr::Const(literal_base(lit)))),
            Expr::Metric(m) => {
                // Nav-point metrics on a channel source would need interpolating
                // onto the channel's sample times, which is a later step.
                self.reject_metric_on_channel_source(m.metric, m.span)?;
                if self.windowed && !in_agg {
                    return Err(err_hint(
                        m.span,
                        format!("{metric} is per point", metric = m.metric),
                        format!(
                            "wrap it in an aggregate like avg({metric})",
                            metric = m.metric
                        ),
                    ));
                }
                if !self.referenced.iter().any(|(seen, _)| *seen == m.metric) {
                    self.referenced.push((m.metric, m.span));
                }
                Ok((value_type(m.metric.quantity()), CExpr::Metric(m.metric)))
            }
            Expr::Channel(c) => self.channel(c, in_agg),
            Expr::Unary { op, operand, span } => self.unary(*op, operand, *span, in_agg),
            Expr::Call { func, arg, span } => self.call(*func, arg, *span, in_agg),
            Expr::Binary { op, lhs, rhs, span } => self.binary(*op, lhs, rhs, *span, in_agg),
            Expr::Power {
                base,
                exponent,
                span,
            } => self.power(base, *exponent, *span, in_agg),
        }
    }

    fn power(
        &mut self,
        base: &Expr,
        exponent: i8,
        span: Span,
        in_agg: bool,
    ) -> Result<(ValueType, CExpr), Diagnostic> {
        let (base_type, cbase) = self.expr(base, in_agg)?;
        let result = match base_type {
            ValueType::Condition => return Err(err(span, "cannot raise a condition to a power")),
            ValueType::Timestamp => return Err(err(span, "timestamps do not support powers")),
            // The power scales the dimension; the wrap flag drops, since a
            // squared angle is no longer a direction.
            ValueType::Dimensioned { dim, .. } => dimensioned(dim.powi(exponent)),
            // Any power of a dimensionless value is a bare number.
            ValueType::Dimensionless(_) => ValueType::Dimensionless(Kind::Number),
        };
        Ok((
            result,
            CExpr::Power {
                base: Box::new(cbase),
                exponent,
            },
        ))
    }

    /// Validate a per-sample reference to channel `name`, labelled as it reads
    /// in source (`@accel.x`, `norm(@accel)`). A per-sample value must be
    /// aggregated, except on a channel source where a reference to the source
    /// channel is itself the match granularity - unless a window groups samples,
    /// which then needs an aggregate. On a channel source, naming any other
    /// channel is off the source's timeline.
    fn per_sample_ok(
        &self,
        name: &str,
        in_agg: bool,
        span: Span,
        label: &str,
    ) -> Result<(), Diagnostic> {
        if let Some(source) = self.source_channel {
            if source != name {
                return Err(err_hint(
                    span,
                    format!("@{name} is not the source channel"),
                    format!("a query on @{source} reads only @{source}"),
                ));
            }
            // The source channel is per-sample: a bare use matches per sample,
            // unless a window groups samples and an aggregate must reduce them.
            if in_agg || !self.windowed {
                return Ok(());
            }
        } else if in_agg {
            return Ok(());
        }
        Err(err_hint(
            span,
            format!("{label} is per sample"),
            format!("wrap it in an aggregate like max({label})"),
        ))
    }

    /// Resolve a channel reference against the schema. A `@name.component`
    /// selects one column of a vector channel as a scalar; a bare `@name` is a
    /// scalar channel, or an error for a vector (which has no scalar value on
    /// its own). A per-sample reference must be aggregated (see
    /// [`per_sample_ok`](Self::per_sample_ok)).
    fn channel(&self, c: &ChannelRef, in_agg: bool) -> Result<(ValueType, CExpr), Diagnostic> {
        let Some(info) = self.schema.get(&c.name) else {
            return Err(err(c.span, format!("no such channel @{}", c.name)));
        };
        reject_unit_conflict(&c.name, info, c.span)?;
        let component = resolve_component(c, info)?;
        self.per_sample_ok(&c.name, in_agg, c.span, &c.to_string())?;
        Ok((
            channel_value_type(info),
            CExpr::Channel(ChannelKey {
                name: c.name.clone(),
                component,
            }),
        ))
    }

    /// Resolve `norm(@vector)`: the Euclidean magnitude of a whole vector
    /// channel, in the channel's own dimension. The argument must be a bare
    /// vector reference (no component); like a bare channel, the result is
    /// per-sample and must sit inside an aggregate.
    fn norm(&self, arg: &Expr, span: Span, in_agg: bool) -> Result<(ValueType, CExpr), Diagnostic> {
        let Expr::Channel(c) = arg else {
            return Err(err_hint(
                span,
                "norm needs a vector channel",
                "give it a whole vector, e.g. norm(@accel)",
            ));
        };
        let Some(info) = self.schema.get(&c.name) else {
            return Err(err(c.span, format!("no such channel @{}", c.name)));
        };
        reject_unit_conflict(&c.name, info, c.span)?;
        if c.component.is_some() {
            return Err(err_hint(
                c.span,
                "norm takes a whole vector, not a component",
                format!("use norm(@{})", c.name),
            ));
        }
        if info.components.is_empty() {
            return Err(err_hint(
                c.span,
                format!("@{} is not a vector channel", c.name),
                "norm needs a vector like @accel",
            ));
        }
        self.per_sample_ok(&c.name, in_agg, span, &format!("norm(@{})", c.name))?;
        Ok((channel_value_type(info), CExpr::Norm(c.name.clone())))
    }

    fn unary(
        &mut self,
        op: UnaryOp,
        operand: &Expr,
        span: Span,
        in_agg: bool,
    ) -> Result<(ValueType, CExpr), Diagnostic> {
        let (value_type, cexpr) = self.expr(operand, in_agg)?;
        match op {
            UnaryOp::Not => {
                if !value_type.is_condition() {
                    return Err(err(span, "not needs a condition"));
                }
                Ok((ValueType::Condition, CExpr::Not(Box::new(cexpr))))
            }
            UnaryOp::Neg => {
                let rejected = match value_type {
                    ValueType::Condition => Some("cannot negate a condition"),
                    ValueType::Timestamp => Some("cannot negate a timestamp"),
                    ValueType::Dimensioned { circular: true, .. } => {
                        Some("cannot negate a direction")
                    }
                    _ => None,
                };
                if let Some(message) = rejected {
                    return Err(err(span, message));
                }
                Ok((value_type, CExpr::Neg(Box::new(cexpr))))
            }
        }
    }

    fn call(
        &mut self,
        func: Func,
        arg: &Expr,
        span: Span,
        in_agg: bool,
    ) -> Result<(ValueType, CExpr), Diagnostic> {
        if func == Func::Abs {
            let (value_type, cexpr) = self.expr(arg, in_agg)?;
            if value_type.is_condition() {
                return Err(err(span, "abs needs a value, not a condition"));
            }
            return Ok((value_type, CExpr::Abs(Box::new(cexpr))));
        }
        if func == Func::Sqrt {
            let (value_type, cexpr) = self.expr(arg, in_agg)?;
            let result = match value_type {
                ValueType::Condition => {
                    return Err(err(span, "sqrt needs a value, not a condition"));
                }
                ValueType::Timestamp => {
                    return Err(err(span, "cannot take the square root of a timestamp"));
                }
                // The square root of any dimensionless value is a bare number.
                ValueType::Dimensionless(_) => ValueType::Dimensionless(Kind::Number),
                // A square root halves the exponents, so the dimension must be a
                // perfect square (`sqrt(velocity²)` is a speed).
                ValueType::Dimensioned { dim, .. } => match dim.sqrt() {
                    Some(root) => dimensioned(root),
                    None => {
                        return Err(err_hint(
                            span,
                            "sqrt needs a square",
                            "square the values first, e.g. sqrt(x² + y²)",
                        ));
                    }
                },
            };
            return Ok((result, CExpr::Sqrt(Box::new(cexpr))));
        }
        if func == Func::Norm {
            return self.norm(arg, span, in_agg);
        }

        if !self.windowed {
            return Err(err(span, format!("{func} needs a window")));
        }
        if in_agg {
            return Err(err(span, "aggregates cannot be nested"));
        }
        let (value_type, cexpr) = self.expr(arg, true)?;
        if value_type.is_condition() {
            return Err(err(span, format!("{func} needs a value, not a condition")));
        }
        let circular = matches!(value_type, ValueType::Dimensioned { circular: true, .. });
        // avg/min/max collapse a direction ambiguously; the rest are fine.
        if circular && matches!(func, Func::Avg | Func::Min | Func::Max) {
            return Err(err_hint(
                span,
                format!("{func} on a direction is ambiguous"),
                "use spread, std, first, last, or delta",
            ));
        }
        // Circular variance is a unitless quantity in [0, 1], not a squared
        // angle, so `var` on a direction is a category error rather than a value.
        if circular && func == Func::Var {
            return Err(err_hint(
                span,
                "var is not defined for a direction",
                "circular variance is unitless, not a squared angle - use std",
            ));
        }
        let source = aggregate_source(&cexpr, span)?;
        Ok((
            agg_result(func, value_type),
            CExpr::Agg {
                func,
                circular,
                source,
                arg: Box::new(cexpr),
            },
        ))
    }

    fn binary(
        &mut self,
        op: BinaryOp,
        lhs: &Expr,
        rhs: &Expr,
        span: Span,
        in_agg: bool,
    ) -> Result<(ValueType, CExpr), Diagnostic> {
        let (lt, cl) = self.expr(lhs, in_agg)?;
        let (rt, cr) = self.expr(rhs, in_agg)?;
        match op {
            BinaryOp::And | BinaryOp::Or => {
                if !lt.is_condition() || !rt.is_condition() {
                    let side = if lt.is_condition() { rhs } else { lhs };
                    return Err(err(
                        side.span(),
                        format!("{} needs conditions on both sides", op.text()),
                    ));
                }
                Ok((
                    ValueType::Condition,
                    CExpr::Logic {
                        and: op == BinaryOp::And,
                        lhs: Box::new(cl),
                        rhs: Box::new(cr),
                    },
                ))
            }
            BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge => {
                check_comparable(lhs, lt, rhs, rt, span)?;
                let cmp = match op {
                    BinaryOp::Lt => CmpOp::Lt,
                    BinaryOp::Le => CmpOp::Le,
                    BinaryOp::Gt => CmpOp::Gt,
                    _ => CmpOp::Ge,
                };
                Ok((
                    ValueType::Condition,
                    CExpr::Cmp {
                        op: cmp,
                        lhs: Box::new(cl),
                        rhs: Box::new(cr),
                    },
                ))
            }
            BinaryOp::Eq | BinaryOp::Ne => {
                check_comparable(lhs, lt, rhs, rt, span)?;
                if !equality_allowed(lt, rt) {
                    return Err(equality_needs_range(lhs, rhs, span));
                }
                Ok((
                    ValueType::Condition,
                    CExpr::Cmp {
                        op: if op == BinaryOp::Eq {
                            CmpOp::Eq
                        } else {
                            CmpOp::Ne
                        },
                        lhs: Box::new(cl),
                        rhs: Box::new(cr),
                    },
                ))
            }
            BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div => {
                let arith_op = match op {
                    BinaryOp::Add => ArithOp::Add,
                    BinaryOp::Sub => ArithOp::Sub,
                    BinaryOp::Mul => ArithOp::Mul,
                    _ => ArithOp::Div,
                };
                let value_type = arith(arith_op, lhs, lt, rhs, rt, span)?;
                Ok((
                    value_type,
                    CExpr::Arith {
                        op: arith_op,
                        lhs: Box::new(cl),
                        rhs: Box::new(cr),
                    },
                ))
            }
        }
    }
}

fn reject_unit_conflict(name: &str, info: &ChannelInfo, span: Span) -> Result<(), Diagnostic> {
    if info.conflicting_units.is_empty() {
        return Ok(());
    }
    Err(err_hint(
        span,
        format!("@{name} has incompatible units across loaded files"),
        format!("found {}", info.conflicting_units.join(", ")),
    ))
}

/// The value type an aggregate produces from its argument. Where the argument
/// names a quantity, reuses [`Func::result_quantity`] - the single definition
/// of the collapse rule (`spread`/`std`/`delta` turn a direction into a plain
/// angle and a timestamp into a duration), shared with autocomplete. An exotic
/// dimension has no quantity and passes straight through, since an aggregate
/// keeps its argument's dimension.
fn agg_result(func: Func, arg: ValueType) -> ValueType {
    if func == Func::Var {
        return var_result(arg);
    }
    match named_quantity(arg) {
        Some(quantity) => value_type(func.result_quantity(quantity)),
        None => arg,
    }
}

/// The result type of `var`: the argument's dimension squared. A count or ratio
/// squares to a bare number, and a timestamp's variance is a squared duration.
/// The checker rejects `var` on a direction before this, so the argument is
/// never circular.
fn var_result(arg: ValueType) -> ValueType {
    match arg {
        ValueType::Dimensioned { dim, circular } => {
            debug_assert!(!circular, "call rejects a direction before agg_result");
            dimensioned(dim.powi(2))
        }
        ValueType::Timestamp => dimensioned(Dimension::TIME.powi(2)),
        ValueType::Dimensionless(_) => ValueType::Dimensionless(Kind::Number),
        ValueType::Condition => {
            debug_assert!(false, "call rejects a condition argument before agg_result");
            arg
        }
    }
}

/// `==`/`!=` compare counts only: both sides dimensionless and discrete, with
/// at least one a genuine count. So `sats_fix == 6` is fine, while `3 == 3` and
/// the float-equality trap `velocity == 30 km/h` are not.
fn equality_allowed(lt: ValueType, rt: ValueType) -> bool {
    let discrete = |vt| matches!(vt, ValueType::Dimensionless(Kind::Count | Kind::Number));
    let is_count = |vt| vt == ValueType::Dimensionless(Kind::Count);
    discrete(lt) && discrete(rt) && (is_count(lt) || is_count(rt))
}

/// Shared compatibility rules for comparisons (`<`, `==`, …).
fn check_comparable(
    lhs: &Expr,
    lt: ValueType,
    rhs: &Expr,
    rt: ValueType,
    span: Span,
) -> Result<(), Diagnostic> {
    if lt.is_condition() || rt.is_condition() {
        let side = if lt.is_condition() { lhs } else { rhs };
        return Err(err(side.span(), "cannot compare conditions"));
    }
    if compatible(lt, rt) {
        return Ok(());
    }
    if let Some(diagnostic) = unit_mismatch(lhs, lt, rhs, rt) {
        return Err(diagnostic);
    }
    let message = compare_error(lt, rt);
    Err(match squared_hint(lt, rt) {
        Some(help) => err_hint(span, message, help),
        None => err(span, message),
    })
}

/// When a comparison fails only because one side is a squared quantity whose
/// root matches the other side, suggest reducing it with `sqrt` - e.g.
/// `velocity² > 30 km/h` becomes comparable as `sqrt(velocity²) > 30 km/h`.
fn squared_hint(lt: ValueType, rt: ValueType) -> Option<String> {
    let reduces_to = |square: ValueType, other: ValueType| match square {
        ValueType::Dimensioned { dim, .. } => dim
            .sqrt()
            .is_some_and(|root| compatible(dimensioned(root), other)),
        _ => false,
    };
    (reduces_to(lt, rt) || reduces_to(rt, lt)).then(|| "take its square root with sqrt".to_owned())
}

/// `+ - * /` on the value types. Replaces the old hand-written quantity table
/// with dimensional algebra: multiplication adds dimensions, division
/// subtracts, and a dimensionless result becomes a bare number. Products and
/// quotients of dimensioned values are therefore always well-formed - a wrong
/// combination surfaces when the result is compared, not here.
fn arith(
    op: ArithOp,
    lhs: &Expr,
    lt: ValueType,
    rhs: &Expr,
    rt: ValueType,
    span: Span,
) -> Result<ValueType, Diagnostic> {
    if lt.is_condition() || rt.is_condition() {
        return Err(err(span, "conditions do not support arithmetic"));
    }
    match op {
        ArithOp::Add | ArithOp::Sub => add_sub(lhs, lt, rhs, rt, span),
        ArithOp::Mul | ArithOp::Div => {
            // A timestamp has no scale, so it cannot multiply or divide.
            if lt == ValueType::Timestamp || rt == ValueType::Timestamp {
                return Err(err(span, "timestamps do not support arithmetic"));
            }
            Ok(if op == ArithOp::Mul {
                multiply(lt, rt)
            } else {
                divide(lt, rt)
            })
        }
    }
}

/// Addition and subtraction: both sides must share a dimension. Timestamps and
/// directions have no meaningful sum, and are rejected with their own message.
fn add_sub(
    lhs: &Expr,
    lt: ValueType,
    rhs: &Expr,
    rt: ValueType,
    span: Span,
) -> Result<ValueType, Diagnostic> {
    for value_type in [lt, rt] {
        let rejected = match value_type {
            ValueType::Timestamp => Some("timestamps do not support + and -"),
            ValueType::Dimensioned { circular: true, .. } => {
                Some("directions do not support + and -")
            }
            _ => None,
        };
        if let Some(message) = rejected {
            return Err(err(span, message));
        }
    }
    if compatible(lt, rt) {
        return Ok(add_sub_result(lt, rt));
    }
    if let Some(diagnostic) = unit_mismatch(lhs, lt, rhs, rt) {
        return Err(diagnostic);
    }
    Err(err(span, unsupported_arith(lt, rt)))
}

/// The sum's type once the operands are known compatible. Dimensioned operands
/// share a dimension and keep it; dimensionless operands combine their kinds.
fn add_sub_result(lt: ValueType, rt: ValueType) -> ValueType {
    match (lt, rt) {
        (ValueType::Dimensionless(a), ValueType::Dimensionless(b)) => {
            ValueType::Dimensionless(combine_kinds(a, b))
        }
        (ValueType::Dimensioned { dim, .. }, _) => ValueType::linear(dim),
        _ => {
            // Compatible operands are either both dimensionless or both the
            // same dimension; timestamps and conditions are rejected upstream.
            debug_assert!(false, "add_sub_result reached an incompatible operand pair");
            lt
        }
    }
}

/// The kind of a dimensionless sum. A bare number takes on the other kind; two
/// like kinds stay themselves.
fn combine_kinds(a: Kind, b: Kind) -> Kind {
    match (a, b) {
        (Kind::Number, other) | (other, Kind::Number) => other,
        _ => a,
    }
}

/// Product: dimensions multiply. Scaling a dimensioned value by a dimensionless
/// one keeps its dimension (and its wrap flag); two dimensionless values give a
/// bare number.
fn multiply(lt: ValueType, rt: ValueType) -> ValueType {
    match (lt, rt) {
        (ValueType::Dimensioned { dim, circular }, ValueType::Dimensionless(_))
        | (ValueType::Dimensionless(_), ValueType::Dimensioned { dim, circular }) => {
            ValueType::Dimensioned { dim, circular }
        }
        (ValueType::Dimensioned { dim: a, .. }, ValueType::Dimensioned { dim: b, .. }) => {
            dimensioned(a * b)
        }
        (ValueType::Dimensionless(_), ValueType::Dimensionless(_)) => {
            ValueType::Dimensionless(Kind::Number)
        }
        _ => {
            // Timestamps and conditions are rejected by `arith` before this.
            debug_assert!(false, "multiply reached a timestamp or condition");
            ValueType::Dimensionless(Kind::Number)
        }
    }
}

/// Quotient: dimensions divide. Dividing by a dimensionless value keeps the
/// numerator's dimension; other combinations subtract the exponents, with a
/// dimensionless result collapsing to a bare number.
fn divide(lt: ValueType, rt: ValueType) -> ValueType {
    match (lt, rt) {
        (ValueType::Dimensioned { dim, circular }, ValueType::Dimensionless(_)) => {
            ValueType::Dimensioned { dim, circular }
        }
        (ValueType::Dimensioned { dim: a, .. }, ValueType::Dimensioned { dim: b, .. }) => {
            dimensioned(a / b)
        }
        (ValueType::Dimensionless(_), ValueType::Dimensioned { dim, .. }) => {
            dimensioned(Dimension::DIMENSIONLESS / dim)
        }
        (ValueType::Dimensionless(_), ValueType::Dimensionless(_)) => {
            ValueType::Dimensionless(Kind::Number)
        }
        _ => {
            // Timestamps and conditions are rejected by `arith` before this.
            debug_assert!(false, "divide reached a timestamp or condition");
            ValueType::Dimensionless(Kind::Number)
        }
    }
}

/// Whether two value types can be compared. Dimensioned values compare when
/// their dimensions match (so a plain angle and a direction, both `A`, do);
/// dimensionless values compare when their kinds line up.
fn compatible(a: ValueType, b: ValueType) -> bool {
    match (a, b) {
        (ValueType::Timestamp, ValueType::Timestamp) => true,
        (ValueType::Dimensioned { dim: da, .. }, ValueType::Dimensioned { dim: db, .. }) => {
            da == db
        }
        (ValueType::Dimensionless(ka), ValueType::Dimensionless(kb)) => {
            dimensionless_compatible(ka, kb)
        }
        _ => false,
    }
}

/// A bare number compares with a count (`sats_fix > 6`), but a count and a
/// ratio never mix, and a bare number never stands in for a ratio (which must
/// carry `%`).
fn dimensionless_compatible(a: Kind, b: Kind) -> bool {
    a == b
        || matches!(
            (a, b),
            (Kind::Number, Kind::Count) | (Kind::Count, Kind::Number)
        )
}

fn compare_error(lt: ValueType, rt: ValueType) -> String {
    format!("cannot compare {} with {}", type_label(lt), type_label(rt))
}

fn unsupported_arith(a: ValueType, b: ValueType) -> String {
    format!(
        "unsupported arithmetic between {} and {}",
        type_label(a),
        type_label(b)
    )
}

/// Errors for a literal whose (missing or wrong) unit clashes with the other
/// side, e.g. `velocity > 30` or `velocity > 30 deg`. `None` when neither
/// side is a literal, i.e. the mismatch is not unit-shaped.
fn unit_mismatch(lhs: &Expr, lt: ValueType, rhs: &Expr, rt: ValueType) -> Option<Diagnostic> {
    for (lit_expr, other_expr, other_t) in [(lhs, rhs, rt), (rhs, lhs, lt)] {
        let Expr::Number(lit) = lit_expr else {
            continue;
        };
        let desc = describe(other_expr).unwrap_or_else(|| "this value".to_owned());
        if other_t == ValueType::Timestamp {
            return Some(err_hint(
                lit.span,
                "timestamps have no literals",
                "restrict time with the global filter",
            ));
        }
        let other_q = named_quantity(other_t);
        match lit.unit {
            None => {
                let example = other_q.and_then(example_literal)?;
                return Some(err(
                    lit.span,
                    format!("{desc} needs a unit, e.g. {example}"),
                ));
            }
            Some(unit) => {
                if other_t == ValueType::Dimensionless(Kind::Count) {
                    return Some(err_hint(
                        lit.span,
                        format!("{desc} is a count"),
                        "compare against a bare number",
                    ));
                }
                let quantity = other_q?;
                let list = unit_list(quantity)?;
                return Some(err(
                    lit.span,
                    format!("expected a {quantity} unit ({list}), found {}", unit.text()),
                ));
            }
        }
    }
    None
}

/// A number literal's value type: `%` is a ratio, another unit its dimension, a
/// bare number the neutral [`Kind::Number`].
fn literal_type(lit: &NumberLit) -> ValueType {
    match lit.unit {
        // `%` is the one dimensionless unit, and it denominates a ratio.
        Some(unit) if unit::dimension(unit).is_dimensionless() => {
            ValueType::Dimensionless(Kind::Ratio)
        }
        Some(unit) => ValueType::linear(unit::dimension(unit)),
        None => ValueType::Dimensionless(Kind::Number),
    }
}

/// A number literal's value in base units.
fn literal_base(lit: &NumberLit) -> f64 {
    match lit.unit {
        Some(unit) => lit.value * unit.to_base(),
        None => lit.value,
    }
}

/// The pinned `==` message: with a unit literal on one side, suggest the
/// concrete range; otherwise the generic form.
fn equality_needs_range(lhs: &Expr, rhs: &Expr, span: Span) -> Diagnostic {
    for (lit_expr, other_expr) in [(lhs, rhs), (rhs, lhs)] {
        let Expr::Number(NumberLit {
            value,
            unit: Some(unit),
            span: lit_span,
        }) = lit_expr
        else {
            continue;
        };
        let desc = describe(other_expr).unwrap_or_else(|| "this value".to_owned());
        let unit = unit.text();
        return err(
            *lit_span,
            format!(
                "use a range, e.g. {lo} {unit} < {desc} and {desc} < {hi} {unit}",
                lo = value - 1.0,
                hi = value + 1.0,
            ),
        );
    }
    err_hint(span, "== compares counts only", "use a range")
}

/// A short name for the value being compared, for error messages: the metric
/// itself, seen through any wrapping calls.
fn describe(expr: &Expr) -> Option<String> {
    match expr {
        Expr::Metric(m) => Some(m.metric.to_string()),
        Expr::Call { arg, .. } => describe(arg),
        Expr::Unary { operand, .. } => describe(operand),
        Expr::Power { base, .. } => describe(base),
        Expr::Channel(c) => Some(c.to_string()),
        Expr::Number(_) | Expr::Binary { .. } => None,
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;
    use strum::{EnumCount as _, IntoEnumIterator as _};

    use super::Kind::{Count, Number, Ratio};
    use super::*;

    /// Every quantity maps to a value type and back, so `named_dimension` stays
    /// in step with `Quantity::dimension`. A new quantity not wired into
    /// `named_dimension` fails here instead of silently becoming an unnamed
    /// exotic value that degrades every error message.
    #[test]
    fn value_type_round_trips_through_named_quantity() {
        let mut covered = 0;
        for quantity in Quantity::iter() {
            assert_eq!(
                named_quantity(value_type(quantity)),
                Some(quantity),
                "{quantity}"
            );
            covered += 1;
        }
        assert_eq!(covered, Quantity::COUNT);
    }

    /// A bare number pairs with a count, never a ratio; a count and a ratio
    /// never mix.
    #[rstest]
    #[case::number_number(Number, Number, true)]
    #[case::count_count(Count, Count, true)]
    #[case::ratio_ratio(Ratio, Ratio, true)]
    #[case::number_count(Number, Count, true)]
    #[case::count_number(Count, Number, true)]
    #[case::number_ratio(Number, Ratio, false)]
    #[case::ratio_number(Ratio, Number, false)]
    #[case::count_ratio(Count, Ratio, false)]
    #[case::ratio_count(Ratio, Count, false)]
    fn dimensionless_compatibility(#[case] a: Kind, #[case] b: Kind, #[case] expected: bool) {
        assert_eq!(dimensionless_compatible(a, b), expected);
    }

    /// `==`/`!=` are counts only: both sides discrete, at least one a real
    /// count.
    #[rstest]
    #[case::count_count(Count, Count, true)]
    #[case::count_number(Count, Number, true)]
    #[case::number_count(Number, Count, true)]
    #[case::number_number(Number, Number, false)]
    #[case::count_ratio(Count, Ratio, false)]
    #[case::ratio_ratio(Ratio, Ratio, false)]
    fn equality_requires_a_count(#[case] a: Kind, #[case] b: Kind, #[case] expected: bool) {
        let dimensionless = |kind| ValueType::Dimensionless(kind);
        assert_eq!(
            equality_allowed(dimensionless(a), dimensionless(b)),
            expected
        );
    }

    /// A bare number takes on the other kind when summed; like kinds stay.
    #[rstest]
    #[case::number_count(Number, Count, Count)]
    #[case::ratio_number(Ratio, Number, Ratio)]
    #[case::number_number(Number, Number, Number)]
    #[case::count_count(Count, Count, Count)]
    fn combining_kinds(#[case] a: Kind, #[case] b: Kind, #[case] expected: Kind) {
        assert_eq!(combine_kinds(a, b), expected);
    }
}

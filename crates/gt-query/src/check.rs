//! Static checking: dimensions, aggregate placement, parameter requirements.
//!
//! Successful checking produces a [`CheckedQuery`] whose expression tree has
//! literals already converted to base units, so evaluation is plain f64
//! arithmetic. Several error messages here are user-facing UX pinned verbatim
//! by tests - change them deliberately.

use gt_types::DisplayMode;

use crate::Diagnostic;
use crate::ast::{BinaryOp, Expr, Func, NumberLit, ParamDecl, ParamName, Query, Span, UnaryOp};
use crate::dimension::Dimension;
use crate::metric::{Quantity, QueryMetric};
use crate::unit::{example_literal, unit_list};

/// Resolved `with` parameters, in base units.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Params {
    pub mask_deg: Option<f64>,
    pub snr_drop_db_hz: Option<f64>,
    pub slip_window_s: Option<f64>,
}

/// A query that passed all static checks, ready to run.
#[derive(Debug, Clone)]
pub struct CheckedQuery {
    pub(crate) window: Option<usize>,
    pub(crate) predicates: Vec<CExpr>,
    params: Params,
    columns: Vec<QueryMetric>,
    referenced: Vec<QueryMetric>,
    unused_params: Vec<ParamName>,
    mode: DisplayMode,
}

impl CheckedQuery {
    /// Window length in points, when the query has a `window` stage.
    pub fn window(&self) -> Option<usize> {
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

/// Checked expression: literals in base units, aggregates tagged circular
/// where they run on a direction.
#[derive(Debug, Clone)]
pub(crate) enum CExpr {
    Const(f64),
    Metric(QueryMetric),
    Agg {
        func: Func,
        circular: bool,
        arg: Box<CExpr>,
    },
    Abs(Box<CExpr>),
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

/// The static type the checker gives an expression. Dimensioned values carry
/// their [`Dimension`]; dimensionless values carry a [`Kind`] so a count, a
/// ratio, and a bare number stay distinct even though their dimension is the
/// same zero. `Timestamp` and `Condition` stand outside dimensional arithmetic.
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
            // Unreachable: only Timestamp and Condition lack a dimension, and
            // both matched above.
            None => ValueType::Dimensionless(Kind::Number),
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

/// A short label for a value type in an error, or `None` when it has no concise
/// name (an exotic dimension such as a squared speed).
fn type_label(vt: ValueType) -> Option<String> {
    match named_quantity(vt) {
        Some(quantity) => Some(quantity.to_string()),
        None => matches!(vt, ValueType::Dimensionless(Kind::Number)).then(|| "number".to_owned()),
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

fn err(span: Span, message: impl Into<String>) -> Diagnostic {
    Diagnostic::new(span, message)
}

fn err_hint(span: Span, message: impl Into<String>, help: impl Into<String>) -> Diagnostic {
    Diagnostic::with_hint(span, message, help)
}

pub fn check(query: &Query) -> Result<CheckedQuery, Diagnostic> {
    let params = resolve_params(&query.params)?;
    let window = match query.window {
        None => None,
        // The conversion only fails on targets where usize is narrower than
        // u64; kept as a defensive branch rather than an assumption.
        Some(w) => {
            Some(usize::try_from(w.len).map_err(|_overflow| err(w.span, "window is too large"))?)
        }
    };

    let mut checker = Checker {
        windowed: window.is_some(),
        referenced: Vec::new(),
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

    let mut occurrences = checker.referenced;
    if let Some(table) = &query.table {
        for column in &table.columns {
            occurrences.push((column.metric, column.span));
        }
    }
    require_params(&occurrences, params)?;

    let referenced: Vec<QueryMetric> = dedup_metrics(occurrences.iter().map(|(m, _)| *m));
    let columns = build_columns(query, &referenced);
    let unused_params = unused_params(&query.params, &referenced);

    Ok(CheckedQuery {
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
                .filter(|u| u.quantity() == quantity)
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

struct Checker {
    windowed: bool,
    /// First occurrence of each referenced metric, in source order.
    referenced: Vec<(QueryMetric, Span)>,
}

impl Checker {
    fn expr(&mut self, expr: &Expr, in_agg: bool) -> Result<(ValueType, CExpr), Diagnostic> {
        match expr {
            Expr::Number(lit) => Ok((literal_type(lit), CExpr::Const(literal_base(lit)))),
            Expr::Metric(m) => {
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
            Expr::Unary { op, operand, span } => self.unary(*op, operand, *span, in_agg),
            Expr::Call { func, arg, span } => self.call(*func, arg, *span, in_agg),
            Expr::Binary { op, lhs, rhs, span } => self.binary(*op, lhs, rhs, *span, in_agg),
        }
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
        Ok((
            agg_result(func, value_type),
            CExpr::Agg {
                func,
                circular,
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
                let value_type = arith(op, lhs, lt, rhs, rt, span)?;
                let arith_op = match op {
                    BinaryOp::Add => ArithOp::Add,
                    BinaryOp::Sub => ArithOp::Sub,
                    BinaryOp::Mul => ArithOp::Mul,
                    _ => ArithOp::Div,
                };
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

/// The value type an aggregate produces from its argument. Where the argument
/// names a quantity, reuses [`Func::result_quantity`] - the single definition
/// of the collapse rule (`spread`/`std`/`delta` turn a direction into a plain
/// angle and a timestamp into a duration), shared with autocomplete. An exotic
/// dimension has no quantity and passes straight through, since an aggregate
/// keeps its argument's dimension.
fn agg_result(func: Func, arg: ValueType) -> ValueType {
    match named_quantity(arg) {
        Some(quantity) => value_type(func.result_quantity(quantity)),
        None => arg,
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
    Err(err(span, compare_error(lt, rt)))
}

/// `+ - * /` on the value types. Replaces the old hand-written quantity table
/// with dimensional algebra: multiplication adds dimensions, division
/// subtracts, and a dimensionless result becomes a bare number. Products and
/// quotients of dimensioned values are therefore always well-formed - a wrong
/// combination surfaces when the result is compared, not here.
fn arith(
    op: BinaryOp,
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
        BinaryOp::Add | BinaryOp::Sub => add_sub(lhs, lt, rhs, rt, span),
        BinaryOp::Mul | BinaryOp::Div => {
            // A timestamp has no scale, so it cannot multiply or divide.
            if lt == ValueType::Timestamp || rt == ValueType::Timestamp {
                return Err(err(span, "timestamps do not support arithmetic"));
            }
            Ok(if op == BinaryOp::Mul {
                multiply(lt, rt)
            } else {
                divide(lt, rt)
            })
        }
        // `binary` routes only the four arithmetic operators here.
        _ => Err(err(span, "conditions do not support arithmetic")),
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
        // Compatible operands that are neither pair do not occur (timestamps
        // are rejected above); fall back to the left type.
        _ => lt,
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
        // Timestamps and conditions never reach here (rejected by `arith`).
        _ => ValueType::Dimensionless(Kind::Number),
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
        // Timestamps and conditions never reach here (rejected by `arith`).
        _ => ValueType::Dimensionless(Kind::Number),
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
    match (type_label(lt), type_label(rt)) {
        (Some(a), Some(b)) => format!("cannot compare {a} with {b}"),
        _ => "cannot compare these values".to_owned(),
    }
}

fn unsupported_arith(a: ValueType, b: ValueType) -> String {
    match (type_label(a), type_label(b)) {
        (Some(x), Some(y)) => format!("unsupported arithmetic between {x} and {y}"),
        _ => "unsupported arithmetic between these values".to_owned(),
    }
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
        Some(unit) if unit.dimension().is_dimensionless() => ValueType::Dimensionless(Kind::Ratio),
        Some(unit) => ValueType::linear(unit.dimension()),
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
        Expr::Number(_) | Expr::Binary { .. } => None,
    }
}

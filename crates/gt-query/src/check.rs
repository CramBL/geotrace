//! Static checking: dimensions, aggregate placement, parameter requirements.
//!
//! Successful checking produces a [`CheckedQuery`] whose expression tree has
//! literals already converted to base units, so evaluation is plain f64
//! arithmetic. Several error messages here are user-facing UX pinned verbatim
//! by tests - change them deliberately.

use gt_types::DisplayMode;

use crate::Diagnostic;
use crate::ast::{BinaryOp, Expr, Func, NumberLit, ParamDecl, ParamName, Query, Span, UnaryOp};
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

fn err(span: Span, message: impl Into<String>) -> Diagnostic {
    Diagnostic {
        span,
        message: message.into(),
        help: None,
    }
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
        let (quantity, cexpr) = checker.expr(predicate, false)?;
        if quantity != Quantity::Condition {
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
        let lit = decl.value;
        match decl.name {
            ParamName::Mask => {
                if params.mask_deg.is_some() {
                    return Err(declared_twice(decl));
                }
                let angle = lit
                    .unit
                    .filter(|u| u.quantity() == Quantity::Angle)
                    .ok_or_else(|| err(decl.span, "mask needs an angle, e.g. mask 15 deg"))?;
                params.mask_deg = Some(lit.value * angle.to_base());
            }
            ParamName::SnrDrop => {
                if params.snr_drop_db_hz.is_some() {
                    return Err(declared_twice(decl));
                }
                if lit.unit.is_some() {
                    return Err(err(decl.span, "snr_drop takes a bare number"));
                }
                params.snr_drop_db_hz = Some(lit.value);
            }
            ParamName::SlipWindow => {
                if params.slip_window_s.is_some() {
                    return Err(declared_twice(decl));
                }
                let duration = lit
                    .unit
                    .filter(|u| u.quantity() == Quantity::Duration)
                    .ok_or_else(|| {
                        err(
                            decl.span,
                            "slip_window needs a duration, e.g. slip_window 5 min",
                        )
                    })?;
                params.slip_window_s = Some(lit.value * duration.to_base());
            }
        }
    }
    Ok(params)
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
    fn expr(&mut self, expr: &Expr, in_agg: bool) -> Result<(Quantity, CExpr), Diagnostic> {
        match expr {
            Expr::Number(lit) => Ok(match lit.unit {
                Some(unit) => (unit.quantity(), CExpr::Const(lit.value * unit.to_base())),
                None => (Quantity::Count, CExpr::Const(lit.value)),
            }),
            Expr::Metric(m) => {
                if self.windowed && !in_agg {
                    return Err(err(
                        m.span,
                        format!(
                            "{metric} is per point - wrap it in an aggregate like avg({metric})",
                            metric = m.metric
                        ),
                    ));
                }
                if !self.referenced.iter().any(|(seen, _)| *seen == m.metric) {
                    self.referenced.push((m.metric, m.span));
                }
                Ok((m.metric.quantity(), CExpr::Metric(m.metric)))
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
    ) -> Result<(Quantity, CExpr), Diagnostic> {
        let (quantity, cexpr) = self.expr(operand, in_agg)?;
        match op {
            UnaryOp::Not => {
                if quantity != Quantity::Condition {
                    return Err(err(span, "not needs a condition"));
                }
                Ok((Quantity::Condition, CExpr::Not(Box::new(cexpr))))
            }
            UnaryOp::Neg => {
                let rejected = match quantity {
                    Quantity::Condition => Some("cannot negate a condition"),
                    Quantity::Timestamp => Some("cannot negate a timestamp"),
                    Quantity::Direction => Some("cannot negate a direction"),
                    _ => None,
                };
                if let Some(message) = rejected {
                    return Err(err(span, message));
                }
                Ok((quantity, CExpr::Neg(Box::new(cexpr))))
            }
        }
    }

    fn call(
        &mut self,
        func: Func,
        arg: &Expr,
        span: Span,
        in_agg: bool,
    ) -> Result<(Quantity, CExpr), Diagnostic> {
        if func == Func::Abs {
            let (quantity, cexpr) = self.expr(arg, in_agg)?;
            if quantity == Quantity::Condition {
                return Err(err(span, "abs needs a value, not a condition"));
            }
            return Ok((quantity, CExpr::Abs(Box::new(cexpr))));
        }

        if !self.windowed {
            return Err(err(span, format!("{func} needs a window")));
        }
        if in_agg {
            return Err(err(span, "aggregates cannot be nested"));
        }
        let (quantity, cexpr) = self.expr(arg, true)?;
        if quantity == Quantity::Condition {
            return Err(err(span, format!("{func} needs a value, not a condition")));
        }
        let result = match func {
            Func::Avg | Func::Min | Func::Max => {
                if quantity == Quantity::Direction {
                    return Err(err(
                        span,
                        format!(
                            "{func} on a direction is ambiguous - use spread, first, last, or delta"
                        ),
                    ));
                }
                quantity
            }
            Func::Spread | Func::Delta => match quantity {
                Quantity::Direction => Quantity::Angle,
                Quantity::Timestamp => Quantity::Duration,
                other => other,
            },
            // Abs is unreachable here (handled above), listed to stay exhaustive.
            Func::First | Func::Last | Func::Abs => quantity,
        };
        Ok((
            result,
            CExpr::Agg {
                func,
                circular: quantity == Quantity::Direction,
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
    ) -> Result<(Quantity, CExpr), Diagnostic> {
        let (ql, cl) = self.expr(lhs, in_agg)?;
        let (qr, cr) = self.expr(rhs, in_agg)?;
        match op {
            BinaryOp::And | BinaryOp::Or => {
                if ql != Quantity::Condition || qr != Quantity::Condition {
                    let side = if ql == Quantity::Condition { rhs } else { lhs };
                    return Err(err(
                        side.span(),
                        format!("{} needs conditions on both sides", op.text()),
                    ));
                }
                Ok((
                    Quantity::Condition,
                    CExpr::Logic {
                        and: op == BinaryOp::And,
                        lhs: Box::new(cl),
                        rhs: Box::new(cr),
                    },
                ))
            }
            BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge => {
                check_comparable(lhs, ql, rhs, qr, span)?;
                let cmp = match op {
                    BinaryOp::Lt => CmpOp::Lt,
                    BinaryOp::Le => CmpOp::Le,
                    BinaryOp::Gt => CmpOp::Gt,
                    _ => CmpOp::Ge,
                };
                Ok((
                    Quantity::Condition,
                    CExpr::Cmp {
                        op: cmp,
                        lhs: Box::new(cl),
                        rhs: Box::new(cr),
                    },
                ))
            }
            BinaryOp::Eq | BinaryOp::Ne => {
                check_comparable(lhs, ql, rhs, qr, span)?;
                if ql != Quantity::Count || qr != Quantity::Count {
                    return Err(equality_needs_range(lhs, rhs, span));
                }
                Ok((
                    Quantity::Condition,
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
                let quantity = arith_quantity(op, lhs, ql, rhs, qr, span)?;
                let arith = match op {
                    BinaryOp::Add => ArithOp::Add,
                    BinaryOp::Sub => ArithOp::Sub,
                    BinaryOp::Mul => ArithOp::Mul,
                    _ => ArithOp::Div,
                };
                Ok((
                    quantity,
                    CExpr::Arith {
                        op: arith,
                        lhs: Box::new(cl),
                        rhs: Box::new(cr),
                    },
                ))
            }
        }
    }
}

/// Shared unit-compatibility rules for comparisons (`<`, `==`, …).
fn check_comparable(
    lhs: &Expr,
    ql: Quantity,
    rhs: &Expr,
    qr: Quantity,
    span: Span,
) -> Result<(), Diagnostic> {
    if ql == Quantity::Condition || qr == Quantity::Condition {
        let side = if ql == Quantity::Condition { lhs } else { rhs };
        return Err(err(side.span(), "cannot compare conditions"));
    }
    if compatible(ql, qr) {
        return Ok(());
    }
    if let Some(diagnostic) = unit_mismatch(lhs, ql, rhs, qr) {
        return Err(diagnostic);
    }
    Err(err(span, format!("cannot compare {ql} with {qr}")))
}

/// The dimensional truth table for `+ - * /`, pinned by a parameterized test.
fn arith_quantity(
    op: BinaryOp,
    lhs: &Expr,
    ql: Quantity,
    rhs: &Expr,
    qr: Quantity,
    span: Span,
) -> Result<Quantity, Diagnostic> {
    if ql == Quantity::Condition || qr == Quantity::Condition {
        return Err(err(span, "conditions do not support arithmetic"));
    }
    match op {
        BinaryOp::Add | BinaryOp::Sub => {
            for quantity in [ql, qr] {
                let rejected = match quantity {
                    Quantity::Timestamp => Some("timestamps do not support + and -"),
                    Quantity::Direction => Some("directions do not support + and -"),
                    _ => None,
                };
                if let Some(message) = rejected {
                    return Err(err(span, message));
                }
            }
            if ql == qr {
                return Ok(ql);
            }
            if let Some(diagnostic) = unit_mismatch(lhs, ql, rhs, qr) {
                return Err(diagnostic);
            }
            Err(unsupported_arith(ql, qr, span))
        }
        BinaryOp::Mul => {
            if ql.is_scalar() && qr.is_scalar() {
                return Ok(if ql == qr { ql } else { Quantity::Ratio });
            }
            if ql.is_scalar() {
                return Ok(qr);
            }
            if qr.is_scalar() {
                return Ok(ql);
            }
            Err(unsupported_arith(ql, qr, span))
        }
        BinaryOp::Div => {
            if qr.is_scalar() {
                return Ok(ql);
            }
            if ql == qr {
                return Ok(Quantity::Ratio);
            }
            match (ql, qr) {
                (Quantity::Length, Quantity::Duration) => Ok(Quantity::Speed),
                (Quantity::Speed, Quantity::Duration) => Ok(Quantity::Acceleration),
                _ => Err(unsupported_arith(ql, qr, span)),
            }
        }
        _ => Err(unsupported_arith(ql, qr, span)),
    }
}

fn compatible(a: Quantity, b: Quantity) -> bool {
    a == b
        || matches!(
            (a, b),
            (Quantity::Angle, Quantity::Direction) | (Quantity::Direction, Quantity::Angle)
        )
}

fn unsupported_arith(a: Quantity, b: Quantity, span: Span) -> Diagnostic {
    err(span, format!("unsupported arithmetic between {a} and {b}"))
}

/// Errors for a literal whose (missing or wrong) unit clashes with the other
/// side, e.g. `velocity > 30` or `velocity > 30 deg`. `None` when neither
/// side is a literal, i.e. the mismatch is not unit-shaped.
fn unit_mismatch(lhs: &Expr, ql: Quantity, rhs: &Expr, qr: Quantity) -> Option<Diagnostic> {
    for (lit_expr, other_expr, other_q) in [(lhs, rhs, qr), (rhs, lhs, ql)] {
        let Expr::Number(lit) = lit_expr else {
            continue;
        };
        let desc = describe(other_expr).unwrap_or_else(|| "this value".to_owned());
        if other_q == Quantity::Timestamp {
            return Some(err(
                lit.span,
                "timestamps have no literals - restrict time with the global filter",
            ));
        }
        match lit.unit {
            None => {
                let example = example_literal(other_q)?;
                return Some(err(
                    lit.span,
                    format!("{desc} needs a unit, e.g. {example}"),
                ));
            }
            Some(unit) => {
                if other_q == Quantity::Count {
                    return Some(err(
                        lit.span,
                        format!("{desc} is a count - compare against a bare number"),
                    ));
                }
                let list = unit_list(other_q)?;
                return Some(err(
                    lit.span,
                    format!("expected a {other_q} unit ({list}), found {}", unit.text()),
                ));
            }
        }
    }
    None
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
    err(span, "== compares counts only - use a range")
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

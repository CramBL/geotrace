//! Syntax tree for a parsed query, produced by [`crate::parse`].

use gt_types::DisplayMode;

use crate::metric::{Quantity, QueryMetric};
use crate::unit::Unit;

/// Byte range into the query source, for error underlining.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

impl Span {
    pub fn new(start: usize, end: usize) -> Self {
        Self { start, end }
    }

    /// Smallest span covering both inputs.
    pub fn to(self, other: Span) -> Span {
        Span {
            start: self.start.min(other.start),
            end: self.end.max(other.end),
        }
    }
}

/// A parsed query: `points` followed by its stages, in canonical order.
#[derive(Debug, Clone, PartialEq)]
pub struct Query {
    pub params: Vec<ParamDecl>,
    pub window: Option<Window>,
    /// One entry per `where` stage; they combine as if joined with `and`.
    pub predicates: Vec<Expr>,
    /// The `draw`/`keep`/`hide` display stage, when written. Absent means
    /// the implicit `draw`.
    pub mode: Option<ModeStage>,
    pub table: Option<TableSpec>,
}

/// A `draw`, `keep`, or `hide` stage as written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModeStage {
    pub mode: DisplayMode,
    pub span: Span,
}

/// `window N` - a sliding group of N consecutive points.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Window {
    pub len: u64,
    pub span: Span,
}

/// One `name value` entry of the `with` stage.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ParamDecl {
    pub name: ParamName,
    pub value: NumberLit,
    pub span: Span,
}

/// The fixed set of `with` parameters.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    strum::Display,
    strum::EnumString,
    strum::IntoStaticStr,
    strum::EnumIter,
    strum::EnumCount,
)]
#[strum(serialize_all = "snake_case")]
pub enum ParamName {
    Mask,
    SnrDrop,
    SlipWindow,
}

impl ParamName {
    /// The quantity of this parameter's value, or `None` when it is a bare
    /// number (`snr_drop`). The single source for both unit checking
    /// ([`crate::check`]) and unit autocomplete ([`crate::completions_at`]).
    pub fn value_quantity(self) -> Option<Quantity> {
        match self {
            ParamName::Mask => Some(Quantity::Angle),
            ParamName::SnrDrop => None,
            ParamName::SlipWindow => Some(Quantity::Duration),
        }
    }
}

/// A numeric literal, optionally carrying a unit (`30 km/h`, `6`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NumberLit {
    pub value: f64,
    pub unit: Option<Unit>,
    pub span: Span,
}

/// `table col, col, …`
#[derive(Debug, Clone, PartialEq)]
pub struct TableSpec {
    pub columns: Vec<MetricRef>,
    pub span: Span,
}

/// A metric name occurrence in the source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MetricRef {
    pub metric: QueryMetric,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Number(NumberLit),
    Metric(MetricRef),
    Unary {
        op: UnaryOp,
        operand: Box<Expr>,
        span: Span,
    },
    Binary {
        op: BinaryOp,
        lhs: Box<Expr>,
        rhs: Box<Expr>,
        span: Span,
    },
    Call {
        func: Func,
        arg: Box<Expr>,
        span: Span,
    },
}

impl Expr {
    pub fn span(&self) -> Span {
        match self {
            Expr::Number(n) => n.span,
            Expr::Metric(m) => m.span,
            Expr::Unary { span, .. } | Expr::Binary { span, .. } | Expr::Call { span, .. } => *span,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    Not,
    Neg,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryOp {
    Or,
    And,
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
    Ne,
    Add,
    Sub,
    Mul,
    Div,
}

impl BinaryOp {
    /// Source spelling, used by error messages and the canonical formatter.
    pub fn text(self) -> &'static str {
        match self {
            BinaryOp::Or => "or",
            BinaryOp::And => "and",
            BinaryOp::Lt => "<",
            BinaryOp::Le => "<=",
            BinaryOp::Gt => ">",
            BinaryOp::Ge => ">=",
            BinaryOp::Eq => "==",
            BinaryOp::Ne => "!=",
            BinaryOp::Add => "+",
            BinaryOp::Sub => "-",
            BinaryOp::Mul => "*",
            BinaryOp::Div => "/",
        }
    }
}

/// Functions callable in expressions. All except `abs` are window aggregates.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    strum::Display,
    strum::EnumString,
    strum::IntoStaticStr,
    strum::EnumIter,
    strum::EnumCount,
)]
#[strum(serialize_all = "lowercase")]
pub enum Func {
    Avg,
    Min,
    Max,
    Spread,
    Std,
    First,
    Last,
    Delta,
    Abs,
}

impl Func {
    pub fn is_aggregate(self) -> bool {
        !matches!(self, Func::Abs)
    }

    /// The quantity this function produces from an argument of `arg`. `spread`,
    /// `std`, and `delta` collapse a direction to a plain angle and a timestamp
    /// to a duration; the others pass the quantity through. The single source
    /// for this rule, shared by [`crate::check`] and [`crate::completions_at`].
    pub fn result_quantity(self, arg: Quantity) -> Quantity {
        match self {
            Func::Spread | Func::Std | Func::Delta => match arg {
                Quantity::Direction => Quantity::Angle,
                Quantity::Timestamp => Quantity::Duration,
                other => other,
            },
            Func::Avg | Func::Min | Func::Max | Func::First | Func::Last | Func::Abs => arg,
        }
    }
}

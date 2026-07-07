//! Canonical query printing: `Display` for [`Query`].
//!
//! Exists for the format/reparse property test and a possible tidy-up action
//! later. History entries always keep the user's text verbatim - this is
//! never applied to stored queries. Expressions print fully parenthesized so
//! the output reparses to the same tree regardless of precedence.

use std::fmt;

use crate::ast::{Expr, Query, UnaryOp, Window};

impl fmt::Display for Query {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "points")?;
        if let Some((head, tail)) = self.params.split_first() {
            write!(f, "\n| with {} {}", head.name, Lit(&head.value))?;
            for param in tail {
                write!(f, ", {} {}", param.name, Lit(&param.value))?;
            }
        }
        match self.window {
            Some(Window::Count { len, .. }) => write!(f, "\n| window {len}")?,
            Some(Window::Duration { value, unit, .. }) => {
                write!(f, "\n| window {value} {}", unit.text())?;
            }
            None => {}
        }
        for predicate in &self.predicates {
            write!(f, "\n| where {predicate}")?;
        }
        if let Some(stage) = self.mode {
            write!(f, "\n| {}", stage.mode)?;
        }
        if let Some(table) = &self.table {
            write!(f, "\n| table ")?;
            for (index, column) in table.columns.iter().enumerate() {
                if index > 0 {
                    write!(f, ", ")?;
                }
                write!(f, "{}", column.metric)?;
            }
        }
        Ok(())
    }
}

/// A number literal with its unit as written.
struct Lit<'a>(&'a crate::ast::NumberLit);

impl fmt::Display for Lit<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0.value)?;
        if let Some(unit) = self.0.unit {
            write!(f, " {}", unit.text())?;
        }
        Ok(())
    }
}

impl fmt::Display for Expr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Expr::Number(lit) => write!(f, "{}", Lit(lit)),
            Expr::Metric(m) => write!(f, "{}", m.metric),
            Expr::Unary {
                op: UnaryOp::Not,
                operand,
                ..
            } => write!(f, "(not {operand})"),
            Expr::Unary {
                op: UnaryOp::Neg,
                operand,
                ..
            } => write!(f, "(-{operand})"),
            Expr::Binary { op, lhs, rhs, .. } => write!(f, "({lhs} {} {rhs})", op.text()),
            Expr::Call { func, arg, .. } => write!(f, "{func}({arg})"),
            Expr::Power { base, exponent, .. } => write!(f, "({base}{})", Superscript(*exponent)),
            Expr::Channel(c) => write!(f, "{c}"),
        }
    }
}

/// An `i8` exponent as superscript digits (`-3` prints `⁻³`), the canonical
/// form for a power. Also used by the checker to label squared dimensions.
pub(crate) struct Superscript(pub(crate) i8);

impl fmt::Display for Superscript {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.0 < 0 {
            write!(f, "⁻")?;
        }
        for digit in self.0.unsigned_abs().to_string().chars() {
            let glyph = match digit {
                '0' => '⁰',
                '1' => '¹',
                '2' => '²',
                '3' => '³',
                '4' => '⁴',
                '5' => '⁵',
                '6' => '⁶',
                '7' => '⁷',
                '8' => '⁸',
                '9' => '⁹',
                other => other,
            };
            write!(f, "{glyph}")?;
        }
        Ok(())
    }
}

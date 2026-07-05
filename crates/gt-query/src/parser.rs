//! Recursive-descent parser from tokens to the [`Query`] syntax tree.
//!
//! Hand-rolled so every error carries the exact wording and span the editor
//! shows; several messages are pinned verbatim by tests.

use std::str::FromStr as _;

use gt_types::DisplayMode;

use crate::Diagnostic;
use crate::ast::{
    BinaryOp, Expr, Func, MetricRef, ModeStage, NumberLit, ParamDecl, ParamName, Query, Span,
    TableSpec, UnaryOp, Window,
};
use crate::lexer::{Tok, Token, lex};
use crate::metric::QueryMetric;
use crate::unit::Unit;

/// Recursion cap for nested expressions, far above anything hand-written.
const MAX_DEPTH: usize = 64;

const UNIT_HELP: &str =
    "units are deg, m, km, km/h, m/s, kn, m/s2, g, km/h/s, ms, s, min, h, %, per s/min/h";

pub fn parse(src: &str) -> Result<Query, Diagnostic> {
    let toks = lex(src)?;
    Parser {
        toks,
        pos: 0,
        depth: 0,
        end: src.len(),
    }
    .query()
}

struct Parser<'src> {
    toks: Vec<Tok<'src>>,
    pos: usize,
    depth: usize,
    end: usize,
}

impl<'src> Parser<'src> {
    fn peek(&self) -> Option<Tok<'src>> {
        self.toks.get(self.pos).copied()
    }

    fn peek_at(&self, offset: usize) -> Option<Tok<'src>> {
        self.toks.get(self.pos.wrapping_add(offset)).copied()
    }

    fn advance(&mut self) -> Option<Tok<'src>> {
        let tok = self.peek();
        if tok.is_some() {
            self.pos += 1;
        }
        tok
    }

    /// Consume `n` tokens already known to exist (verified via `peek_at`).
    fn advance_n(&mut self, n: usize) {
        for _ in 0..n {
            self.advance();
        }
    }

    /// The identifier `offset + 1` tokens ahead, when the token at `offset` is
    /// a `/`. Used to look ahead across the slashes of a compound unit.
    fn peek_slashed_ident(&self, offset: usize) -> Option<Tok<'src>> {
        match (self.peek_at(offset), self.peek_at(offset + 1)) {
            (Some(slash), Some(id)) if slash.kind == Token::Slash && id.kind == Token::Ident => {
                Some(id)
            }
            _ => None,
        }
    }

    /// Span for "unexpected end" errors: the last token, or the very end.
    fn here(&self) -> Span {
        self.peek()
            .map_or(Span::new(self.end, self.end), |t| t.span)
    }

    fn error(&self, span: Span, message: impl Into<String>) -> Diagnostic {
        Diagnostic::new(span, message)
    }

    /// An error whose fix goes in the structured `help` field (shown as a
    /// separate "Hint:" line) rather than tacked onto the message.
    fn error_hint(
        &self,
        span: Span,
        message: impl Into<String>,
        help: impl Into<String>,
    ) -> Diagnostic {
        Diagnostic::with_hint(span, message, help)
    }

    fn with_depth<T>(
        &mut self,
        f: impl FnOnce(&mut Self) -> Result<T, Diagnostic>,
    ) -> Result<T, Diagnostic> {
        if self.depth >= MAX_DEPTH {
            return Err(self.error(self.here(), "expression is too deeply nested"));
        }
        self.depth += 1;
        let result = f(self);
        self.depth -= 1;
        result
    }

    fn query(&mut self) -> Result<Query, Diagnostic> {
        match self.peek() {
            Some(tok) if tok.kind == Token::Points => {
                self.advance();
            }
            _ => {
                return Err(self.error(self.here(), "a query starts with the source: points"));
            }
        }

        let mut query = Query {
            params: Vec::new(),
            window: None,
            predicates: Vec::new(),
            mode: None,
            table: None,
        };

        while let Some(tok) = self.peek() {
            if tok.kind != Token::Pipe {
                let mut err = self.error(tok.span, "expected | or end of query");
                if tok.kind.is_stage_keyword() {
                    err.help = Some("stages are separated by |".to_owned());
                }
                return Err(err);
            }
            self.advance();
            let Some(stage) = self.peek() else {
                return Err(self.error(
                    self.here(),
                    "expected a stage after |: with, window, where, draw, keep, hide, or table",
                ));
            };
            match stage.kind {
                Token::With => self.with_stage(&mut query, stage.span)?,
                Token::Window => self.window_stage(&mut query, stage.span)?,
                Token::Where => self.where_stage(&mut query, stage.span)?,
                Token::Draw => self.mode_stage(&mut query, stage.span, DisplayMode::Draw)?,
                Token::Keep => self.mode_stage(&mut query, stage.span, DisplayMode::Keep)?,
                Token::Hide => self.mode_stage(&mut query, stage.span, DisplayMode::Hide)?,
                Token::Table => self.table_stage(&mut query, stage.span)?,
                _ => {
                    return Err(self.error(
                        stage.span,
                        "expected a stage: with, window, where, draw, keep, hide, or table",
                    ));
                }
            }
        }
        Ok(query)
    }

    fn outputs_started(query: &Query) -> bool {
        query.mode.is_some() || query.table.is_some()
    }

    fn with_stage(&mut self, query: &mut Query, kw_span: Span) -> Result<(), Diagnostic> {
        self.advance();
        if !query.params.is_empty() {
            return Err(self.error(kw_span, "only one with stage is allowed"));
        }
        if query.window.is_some() || !query.predicates.is_empty() || Self::outputs_started(query) {
            return Err(self.error(kw_span, "with must come directly after the source"));
        }
        loop {
            let Some(tok) = self.peek() else {
                return Err(self.error(
                    self.here(),
                    "expected a parameter: mask, snr_drop, or slip_window",
                ));
            };
            if tok.kind != Token::Ident {
                return Err(self.error(
                    tok.span,
                    "expected a parameter: mask, snr_drop, or slip_window",
                ));
            }
            let Ok(name) = ParamName::from_str(tok.text) else {
                return Err(self.error_hint(
                    tok.span,
                    format!("unknown parameter `{}`", tok.text),
                    "parameters are mask, snr_drop, slip_window",
                ));
            };
            self.advance();
            let value = self.number_lit()?;
            query.params.push(ParamDecl {
                name,
                value,
                span: tok.span.to(value.span),
            });
            match self.peek() {
                Some(t) if t.kind == Token::Comma => {
                    self.advance();
                }
                _ => break,
            }
        }
        Ok(())
    }

    fn window_stage(&mut self, query: &mut Query, kw_span: Span) -> Result<(), Diagnostic> {
        self.advance();
        if query.window.is_some() {
            return Err(self.error(kw_span, "only one window stage is allowed"));
        }
        if !query.predicates.is_empty() {
            return Err(self.error_hint(
                kw_span,
                "window must come before where",
                "windows always see consecutive points",
            ));
        }
        if Self::outputs_started(query) {
            return Err(self.error(
                kw_span,
                "window must come before draw, keep, hide, or table",
            ));
        }
        let Some(tok) = self.peek() else {
            return Err(self.error(self.here(), "window needs a point count, e.g. window 10"));
        };
        if tok.kind != Token::Number {
            return Err(self.error(tok.span, "window needs a point count, e.g. window 10"));
        }
        if tok.text.contains('.') {
            return Err(self.error(tok.span, "window takes a whole number of points"));
        }
        let Ok(len) = tok.text.parse::<u64>() else {
            return Err(self.error(tok.span, "window is too large"));
        };
        if len == 0 {
            return Err(self.error(tok.span, "window needs at least 1 point"));
        }
        self.advance();
        if self.at_unit_start() {
            return Err(self.error_hint(
                self.here(),
                "time-based windows are not supported yet",
                "window takes a point count",
            ));
        }
        query.window = Some(Window {
            len,
            span: kw_span.to(tok.span),
        });
        Ok(())
    }

    fn where_stage(&mut self, query: &mut Query, kw_span: Span) -> Result<(), Diagnostic> {
        self.advance();
        if Self::outputs_started(query) {
            return Err(self.error(kw_span, "where must come before draw, keep, hide, or table"));
        }
        let predicate = self.expr()?;
        query.predicates.push(predicate);
        Ok(())
    }

    fn mode_stage(
        &mut self,
        query: &mut Query,
        kw_span: Span,
        mode: DisplayMode,
    ) -> Result<(), Diagnostic> {
        self.advance();
        if query.mode.is_some() {
            return Err(self.error(kw_span, "only one of draw, keep, or hide is allowed"));
        }
        query.mode = Some(ModeStage {
            mode,
            span: kw_span,
        });
        Ok(())
    }

    fn table_stage(&mut self, query: &mut Query, kw_span: Span) -> Result<(), Diagnostic> {
        self.advance();
        if query.table.is_some() {
            return Err(self.error(kw_span, "only one table stage is allowed"));
        }
        let mut columns = Vec::new();
        loop {
            let column = self.metric_ref()?;
            columns.push(column);
            match self.peek() {
                Some(t) if t.kind == Token::Comma => {
                    self.advance();
                }
                _ => break,
            }
        }
        let span = columns.last().map_or(kw_span, |c| kw_span.to(c.span));
        query.table = Some(TableSpec { columns, span });
        Ok(())
    }

    fn metric_ref(&mut self) -> Result<MetricRef, Diagnostic> {
        let Some(tok) = self.peek() else {
            return Err(self.error(self.here(), "expected a metric"));
        };
        if tok.kind != Token::Ident {
            return Err(self.error(tok.span, "expected a metric"));
        }
        let Ok(metric) = QueryMetric::from_str(tok.text) else {
            return Err(self.unknown_metric(tok));
        };
        self.advance();
        Ok(MetricRef {
            metric,
            span: tok.span,
        })
    }

    fn unknown_metric(&self, tok: Tok<'src>) -> Diagnostic {
        Diagnostic {
            span: tok.span,
            message: format!("unknown metric `{}`", tok.text),
            help: Some(
                "metrics are lowercase snake_case, e.g. velocity, heading, sats_fix".to_owned(),
            ),
        }
    }

    /// Expression entry point. Precedence, loosest first: or, and, not,
    /// comparison, `+ -`, `* /`, unary minus.
    fn expr(&mut self) -> Result<Expr, Diagnostic> {
        self.with_depth(Self::or_expr)
    }

    fn or_expr(&mut self) -> Result<Expr, Diagnostic> {
        let mut lhs = self.and_expr()?;
        while matches!(self.peek(), Some(t) if t.kind == Token::Or) {
            self.advance();
            let rhs = self.and_expr()?;
            lhs = binary(BinaryOp::Or, lhs, rhs);
        }
        Ok(lhs)
    }

    fn and_expr(&mut self) -> Result<Expr, Diagnostic> {
        let mut lhs = self.not_expr()?;
        while matches!(self.peek(), Some(t) if t.kind == Token::And) {
            self.advance();
            let rhs = self.not_expr()?;
            lhs = binary(BinaryOp::And, lhs, rhs);
        }
        Ok(lhs)
    }

    fn not_expr(&mut self) -> Result<Expr, Diagnostic> {
        if let Some(tok) = self.peek()
            && tok.kind == Token::Not
        {
            self.advance();
            return self.with_depth(|p| {
                let operand = p.not_expr()?;
                let span = tok.span.to(operand.span());
                Ok(Expr::Unary {
                    op: UnaryOp::Not,
                    operand: Box::new(operand),
                    span,
                })
            });
        }
        self.comparison()
    }

    fn comparison(&mut self) -> Result<Expr, Diagnostic> {
        let lhs = self.sum()?;
        let op = match self.peek().map(|t| t.kind) {
            Some(Token::Lt) => BinaryOp::Lt,
            Some(Token::Le) => BinaryOp::Le,
            Some(Token::Gt) => BinaryOp::Gt,
            Some(Token::Ge) => BinaryOp::Ge,
            Some(Token::EqEq) => BinaryOp::Eq,
            Some(Token::Ne) => BinaryOp::Ne,
            _ => return Ok(lhs),
        };
        self.advance();
        let rhs = self.sum()?;
        Ok(binary(op, lhs, rhs))
    }

    fn sum(&mut self) -> Result<Expr, Diagnostic> {
        let mut lhs = self.term()?;
        loop {
            let op = match self.peek().map(|t| t.kind) {
                Some(Token::Plus) => BinaryOp::Add,
                Some(Token::Minus) => BinaryOp::Sub,
                _ => return Ok(lhs),
            };
            self.advance();
            let rhs = self.term()?;
            lhs = binary(op, lhs, rhs);
        }
    }

    fn term(&mut self) -> Result<Expr, Diagnostic> {
        let mut lhs = self.factor()?;
        loop {
            let op = match self.peek().map(|t| t.kind) {
                Some(Token::Star) => BinaryOp::Mul,
                Some(Token::Slash) => BinaryOp::Div,
                _ => return Ok(lhs),
            };
            self.advance();
            let rhs = self.factor()?;
            lhs = binary(op, lhs, rhs);
        }
    }

    fn factor(&mut self) -> Result<Expr, Diagnostic> {
        if let Some(tok) = self.peek()
            && tok.kind == Token::Minus
        {
            self.advance();
            return self.with_depth(|p| {
                let operand = p.factor()?;
                let span = tok.span.to(operand.span());
                Ok(Expr::Unary {
                    op: UnaryOp::Neg,
                    operand: Box::new(operand),
                    span,
                })
            });
        }
        self.power()
    }

    /// A primary optionally raised to a postfix integer power, `base²`. Power
    /// binds tighter than unary minus (`-x²` is `-(x²)`) and than `*`/`/`.
    fn power(&mut self) -> Result<Expr, Diagnostic> {
        let base = self.primary()?;
        let Some((exponent, exp_span)) = self.exponent()? else {
            return Ok(base);
        };
        let span = base.span().to(exp_span);
        Ok(Expr::Power {
            base: Box::new(base),
            exponent,
            span,
        })
    }

    /// A postfix exponent, superscript (`²`, `⁻³`) or caret (`^2`, `^-3`), or
    /// `None` when none follows. Fractional and out-of-range powers are
    /// rejected with a pointed message.
    fn exponent(&mut self) -> Result<Option<(i8, Span)>, Diagnostic> {
        let Some(tok) = self.peek() else {
            return Ok(None);
        };
        let signed = match tok.kind {
            Token::Superscript => superscript_value(tok.text),
            // Strip the leading `^`; a fractional part means a non-whole power.
            Token::CaretPower => match tok.text.get(1..) {
                Some(digits) if digits.contains('.') => {
                    return Err(self.error(tok.span, "a power must be a whole number"));
                }
                Some(digits) => digits.parse::<i32>().ok(),
                None => None,
            },
            _ => return Ok(None),
        };
        self.advance();
        let value = signed.and_then(|v| i8::try_from(v).ok()).ok_or_else(|| {
            self.error(
                tok.span,
                "a power must be a whole number between -128 and 127",
            )
        })?;
        Ok(Some((value, tok.span)))
    }

    fn primary(&mut self) -> Result<Expr, Diagnostic> {
        let Some(tok) = self.peek() else {
            return Err(self.error(self.here(), "expected a value"));
        };
        match tok.kind {
            Token::Number => Ok(Expr::Number(self.number_lit()?)),
            Token::LParen => {
                self.advance();
                let inner = self.expr()?;
                match self.peek() {
                    Some(t) if t.kind == Token::RParen => {
                        self.advance();
                        Ok(inner)
                    }
                    _ => Err(self.error(self.here(), "expected )")),
                }
            }
            Token::Ident => self.name(tok),
            _ => Err(self.error(tok.span, "expected a value")),
        }
    }

    /// An identifier in value position: a function call or a metric.
    fn name(&mut self, tok: Tok<'src>) -> Result<Expr, Diagnostic> {
        let next_is_paren = matches!(self.peek_at(1), Some(t) if t.kind == Token::LParen);
        if let Ok(func) = Func::from_str(tok.text) {
            if !next_is_paren {
                return Err(self.error_hint(
                    tok.span,
                    format!("{func} is a function"),
                    format!("call it like {func}(velocity)"),
                ));
            }
            self.advance();
            self.advance();
            let arg = self.expr()?;
            let Some(close) = self.peek() else {
                return Err(self.error(self.here(), "expected )"));
            };
            if close.kind != Token::RParen {
                return Err(self.error(close.span, "expected )"));
            }
            self.advance();
            return Ok(Expr::Call {
                func,
                arg: Box::new(arg),
                span: tok.span.to(close.span),
            });
        }
        if let Ok(metric) = QueryMetric::from_str(tok.text) {
            self.advance();
            return Ok(Expr::Metric(MetricRef {
                metric,
                span: tok.span,
            }));
        }
        if next_is_paren {
            return Err(self.error(tok.span, format!("unknown function `{}`", tok.text)));
        }
        Err(self.unknown_metric(tok))
    }

    fn number_lit(&mut self) -> Result<NumberLit, Diagnostic> {
        let Some(tok) = self.peek() else {
            return Err(self.error(self.here(), "expected a number"));
        };
        if tok.kind != Token::Number {
            return Err(self.error(tok.span, "expected a number"));
        }
        // The lexer's number regex always parses as f64 (overlong digit runs
        // saturate to infinity, they do not error), so this branch is
        // defensive: the invariant lives in another file.
        let Ok(value) = tok.text.parse::<f64>() else {
            return Err(self.error(tok.span, "invalid number"));
        };
        self.advance();
        let unit = self.unit()?;
        let span = unit.map_or(tok.span, |(_, unit_span)| tok.span.to(unit_span));
        Ok(NumberLit {
            value,
            unit: unit.map(|(u, _)| u),
            span,
        })
    }

    /// Whether the upcoming tokens could begin a unit; used for the reserved
    /// `window 15 s` form.
    fn at_unit_start(&self) -> bool {
        match self.peek() {
            Some(t) if t.kind == Token::Percent || t.kind == Token::Per => true,
            Some(t) if t.kind == Token::Ident => Unit::from_ident(t.text).is_some(),
            _ => false,
        }
    }

    /// The optional unit after a number literal. An identifier here that is
    /// not a unit is always a mistake (the grammar has no juxtaposition), so
    /// it reports "unknown unit" rather than confusing errors downstream.
    fn unit(&mut self) -> Result<Option<(Unit, Span)>, Diagnostic> {
        let Some(tok) = self.peek() else {
            return Ok(None);
        };
        match tok.kind {
            Token::Percent => {
                self.advance();
                Ok(Some((Unit::Percent, tok.span)))
            }
            Token::Per => {
                self.advance();
                let per_unit = self.peek().and_then(|t| {
                    if t.kind != Token::Ident {
                        return None;
                    }
                    match t.text {
                        "s" => Some((Unit::PerS, t.span)),
                        "min" => Some((Unit::PerMin, t.span)),
                        "h" => Some((Unit::PerH, t.span)),
                        _ => None,
                    }
                });
                let Some((unit, unit_span)) = per_unit else {
                    return Err(self.error(
                        self.here(),
                        "per needs a duration unit: per s, per min, per h",
                    ));
                };
                self.advance();
                Ok(Some((unit, tok.span.to(unit_span))))
            }
            Token::Ident => self.unit_from_ident_chain(tok),
            _ => Ok(None),
        }
    }

    /// A unit written as an identifier, optionally with `/second` and
    /// `/second/third` slash continuations (`km/h`, `m/s2`, `km/h/s`). The
    /// longest matching compound wins; a leftover `/ident` that isn't part of
    /// a known unit is left for expression-level division.
    fn unit_from_ident_chain(
        &mut self,
        first: Tok<'src>,
    ) -> Result<Option<(Unit, Span)>, Diagnostic> {
        // Slash-separated identifiers following `first` (whitespace-agnostic,
        // like every unit form). `second`/`third` are `None` when the shape
        // doesn't continue. Token layout: first `/` second `/` third.
        let second = self.peek_slashed_ident(1);
        let third = second.and_then(|_| self.peek_slashed_ident(3));

        if let (Some(second), Some(third)) = (second, third)
            && let Some(compound) = Unit::from_triple(first.text, second.text, third.text)
        {
            // Consume `first / second / third`.
            self.advance_n(5);
            return Ok(Some((compound, first.span.to(third.span))));
        }
        // A `/ ident (` is division by a function call, never a compound unit -
        // even when the identifier also names a unit, as `min` does (the minute
        // and the aggregate). Leaving it for expression division is what makes
        // `<length> / min(x)` round-trip through the printer.
        let second_starts_call = matches!(self.peek_at(3), Some(t) if t.kind == Token::LParen);
        if let Some(second) = second
            && !second_starts_call
        {
            if let Some(compound) = Unit::from_pair(first.text, second.text) {
                // Consume `first / second`.
                self.advance_n(3);
                return Ok(Some((compound, first.span.to(second.span))));
            }
            // `30 km/s`: the right side is unit-shaped, so this is a typoed
            // compound unit, not a division by a metric.
            if Unit::from_ident(second.text).is_some() || second.text == "s2" {
                return Err(Diagnostic {
                    span: first.span.to(second.span),
                    message: format!("unknown unit `{}/{}`", first.text, second.text),
                    help: Some(UNIT_HELP.to_owned()),
                });
            }
        }
        let Some(single) = Unit::from_ident(first.text) else {
            return Err(Diagnostic {
                span: first.span,
                message: format!("unknown unit `{}`", first.text),
                help: Some(UNIT_HELP.to_owned()),
            });
        };
        self.advance();
        Ok(Some((single, first.span)))
    }
}

fn binary(op: BinaryOp, lhs: Expr, rhs: Expr) -> Expr {
    let span = lhs.span().to(rhs.span());
    Expr::Binary {
        op,
        lhs: Box::new(lhs),
        rhs: Box::new(rhs),
        span,
    }
}

/// The integer value of a superscript run (`²` is 2, `⁻¹⁰` is -10), or `None`
/// when it carries no digit (a lone `⁻`) or overflows an `i32`.
fn superscript_value(text: &str) -> Option<i32> {
    let mut chars = text.chars().peekable();
    let negative = chars.peek() == Some(&'⁻');
    if negative {
        chars.next();
    }
    let mut value: i32 = 0;
    let mut digits = 0;
    for c in chars {
        value = value.checked_mul(10)?.checked_add(superscript_digit(c)?)?;
        digits += 1;
    }
    (digits > 0).then_some(if negative { -value } else { value })
}

fn superscript_digit(c: char) -> Option<i32> {
    Some(match c {
        '⁰' => 0,
        '¹' => 1,
        '²' => 2,
        '³' => 3,
        '⁴' => 4,
        '⁵' => 5,
        '⁶' => 6,
        '⁷' => 7,
        '⁸' => 8,
        '⁹' => 9,
        _ => return None,
    })
}

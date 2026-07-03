//! Recursive-descent parser from tokens to the [`Query`] syntax tree.
//!
//! Hand-rolled so every error carries the exact wording and span the editor
//! shows; several messages are pinned verbatim by tests.

use std::str::FromStr as _;

use crate::Diagnostic;
use crate::ast::{
    BinaryOp, Expr, Func, MetricRef, NumberLit, ParamDecl, ParamName, Query, Span, TableSpec,
    UnaryOp, Window,
};
use crate::lexer::{Tok, Token, lex};
use crate::metric::QueryMetric;
use crate::unit::Unit;

/// Recursion cap for nested expressions, far above anything hand-written.
const MAX_DEPTH: usize = 64;

const UNIT_HELP: &str = "units are deg, m, km, km/h, m/s, kn, m/s2, ms, s, min, h, %, per s/min/h";

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

    /// Span for "unexpected end" errors: the last token, or the very end.
    fn here(&self) -> Span {
        self.peek()
            .map_or(Span::new(self.end, self.end), |t| t.span)
    }

    fn error(&self, span: Span, message: impl Into<String>) -> Diagnostic {
        Diagnostic {
            span,
            message: message.into(),
            help: None,
        }
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
            draw: None,
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
                    "expected a stage after |: with, window, where, draw, or table",
                ));
            };
            match stage.kind {
                Token::With => self.with_stage(&mut query, stage.span)?,
                Token::Window => self.window_stage(&mut query, stage.span)?,
                Token::Where => self.where_stage(&mut query, stage.span)?,
                Token::Draw => self.draw_stage(&mut query, stage.span)?,
                Token::Table => self.table_stage(&mut query, stage.span)?,
                _ => {
                    return Err(self.error(
                        stage.span,
                        "expected a stage: with, window, where, draw, or table",
                    ));
                }
            }
        }
        Ok(query)
    }

    fn outputs_started(query: &Query) -> bool {
        query.draw.is_some() || query.table.is_some()
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
                return Err(self.error(
                    tok.span,
                    format!(
                        "unknown parameter `{}` - parameters are mask, snr_drop, slip_window",
                        tok.text
                    ),
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
            return Err(self.error(
                kw_span,
                "window must come before where - windows always see consecutive points",
            ));
        }
        if Self::outputs_started(query) {
            return Err(self.error(kw_span, "window must come before draw and table"));
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
            return Err(self.error(
                self.here(),
                "time-based windows are not supported yet - window takes a point count",
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
            return Err(self.error(kw_span, "where must come before draw and table"));
        }
        let predicate = self.expr()?;
        query.predicates.push(predicate);
        Ok(())
    }

    fn draw_stage(&mut self, query: &mut Query, kw_span: Span) -> Result<(), Diagnostic> {
        self.advance();
        if query.draw.is_some() {
            return Err(self.error(kw_span, "only one draw stage is allowed"));
        }
        query.draw = Some(kw_span);
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
        self.primary()
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
                return Err(self.error(
                    tok.span,
                    format!("{func} is a function - call it like {func}(velocity)"),
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
            Token::Ident => {
                let Some(single) = Unit::from_ident(tok.text) else {
                    return Err(Diagnostic {
                        span: tok.span,
                        message: format!("unknown unit `{}`", tok.text),
                        help: Some(UNIT_HELP.to_owned()),
                    });
                };
                let slash_pair = match (self.peek_at(1), self.peek_at(2)) {
                    (Some(slash), Some(second))
                        if slash.kind == Token::Slash && second.kind == Token::Ident =>
                    {
                        Some(second)
                    }
                    _ => None,
                };
                if let Some(second) = slash_pair {
                    if let Some(compound) = Unit::from_pair(tok.text, second.text) {
                        self.advance();
                        self.advance();
                        self.advance();
                        return Ok(Some((compound, tok.span.to(second.span))));
                    }
                    // `30 km/s`: the right side is unit-shaped, so this is a
                    // typoed compound unit, not a division by a metric.
                    if Unit::from_ident(second.text).is_some() || second.text == "s2" {
                        return Err(Diagnostic {
                            span: tok.span.to(second.span),
                            message: format!("unknown unit `{}/{}`", tok.text, second.text),
                            help: Some(UNIT_HELP.to_owned()),
                        });
                    }
                }
                self.advance();
                Ok(Some((single, tok.span)))
            }
            _ => Ok(None),
        }
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

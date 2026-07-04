//! Tokenizer for query text, shared by the parser and (later) the editor's
//! syntax highlighting so the two cannot drift.

use logos::Logos;

use crate::Diagnostic;
use crate::ast::Span;

#[derive(Logos, Debug, Clone, Copy, PartialEq, Eq)]
#[logos(skip r"[ \t\r\n]+")]
// A comment intentionally consumes to end of line, so the greedy repetition
// logos 0.16 warns about is the desired behavior.
#[logos(skip(r"#[^\n]*", allow_greedy = true))]
pub enum Token {
    #[token("|")]
    Pipe,
    #[token(",")]
    Comma,
    #[token("(")]
    LParen,
    #[token(")")]
    RParen,
    #[token("<=")]
    Le,
    #[token("<")]
    Lt,
    #[token(">=")]
    Ge,
    #[token(">")]
    Gt,
    #[token("==")]
    EqEq,
    #[token("!=")]
    Ne,
    #[token("+")]
    Plus,
    #[token("-")]
    Minus,
    #[token("*")]
    Star,
    #[token("/")]
    Slash,
    #[token("%")]
    Percent,
    #[token("points")]
    Points,
    #[token("with")]
    With,
    #[token("window")]
    Window,
    #[token("where")]
    Where,
    #[token("draw")]
    Draw,
    #[token("table")]
    Table,
    #[token("and")]
    And,
    #[token("or")]
    Or,
    #[token("not")]
    Not,
    #[token("per")]
    Per,
    #[regex(r"[0-9]+(\.[0-9]+)?")]
    Number,
    #[regex(r"[a-z_][a-z0-9_]*")]
    Ident,
}

impl Token {
    /// Whether this token may start a stage, for "expected a stage" errors.
    pub fn is_stage_keyword(self) -> bool {
        matches!(
            self,
            Token::With | Token::Window | Token::Where | Token::Draw | Token::Table
        )
    }
}

/// A token with its source span and text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tok<'src> {
    pub kind: Token,
    pub span: Span,
    pub text: &'src str,
}

/// Tokenize the whole input, or report the first offending character.
pub fn lex(src: &str) -> Result<Vec<Tok<'_>>, Diagnostic> {
    let mut lexer = Token::lexer(src);
    let mut toks = Vec::new();
    while let Some(result) = lexer.next() {
        let range = lexer.span();
        let span = Span::new(range.start, range.end);
        match result {
            Ok(kind) => toks.push(Tok {
                kind,
                span,
                text: lexer.slice(),
            }),
            Err(()) => {
                let ch = lexer.slice().chars().next().unwrap_or('?');
                return Err(Diagnostic {
                    span,
                    message: format!("unexpected character `{ch}`"),
                    help: Some("queries are all lowercase".to_owned()),
                });
            }
        }
    }
    Ok(toks)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keywords_win_over_idents_only_on_exact_match() {
        let toks = lex("points pointsy per personal").unwrap();
        let kinds: Vec<Token> = toks.iter().map(|t| t.kind).collect();
        assert_eq!(
            kinds,
            vec![Token::Points, Token::Ident, Token::Per, Token::Ident]
        );
    }

    #[test]
    fn comments_and_whitespace_are_skipped() {
        let toks = lex("points # trailing comment\n| draw").unwrap();
        let kinds: Vec<Token> = toks.iter().map(|t| t.kind).collect();
        assert_eq!(kinds, vec![Token::Points, Token::Pipe, Token::Draw]);
    }

    #[test]
    fn uppercase_is_rejected_with_help() {
        let err = lex("Points").unwrap_err();
        assert_eq!(err.message, "unexpected character `P`");
        assert_eq!(err.span, Span::new(0, 1));
    }

    #[test]
    fn compound_units_lex_as_ident_slash_ident() {
        let toks = lex("30 km/h").unwrap();
        let kinds: Vec<Token> = toks.iter().map(|t| t.kind).collect();
        assert_eq!(
            kinds,
            vec![Token::Number, Token::Ident, Token::Slash, Token::Ident]
        );
    }
}

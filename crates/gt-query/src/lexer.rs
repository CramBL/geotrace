//! Tokenizer for query text, shared by the parser and the editor's syntax
//! highlighting so the two cannot drift.

use logos::Logos;

use crate::Diagnostic;
use crate::ast::Span;

#[derive(Logos, Debug, Clone, Copy, PartialEq, Eq)]
#[logos(skip r"[ \t\r\n]+")]
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
    #[token("keep")]
    Keep,
    #[token("hide")]
    Hide,
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
    // A real token rather than a skip so the highlighter can color comments;
    // `lex` filters it out before parsing. Consuming to end of line is the
    // point, hence the greedy-repetition opt-in logos 0.16 requires.
    #[regex(r"#[^\n]*", allow_greedy = true)]
    Comment,
}

/// Coarse token grouping for syntax highlighting. Defined here so the
/// highlighter derives directly from the one lexer instead of keeping its
/// own keyword list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenClass {
    Keyword,
    Number,
    Ident,
    Punctuation,
    Comment,
    /// A character the lexer rejects - highlighted as an error while typing.
    Error,
}

impl Token {
    /// Whether this token may start a stage, for "expected a stage" errors.
    pub fn is_stage_keyword(self) -> bool {
        matches!(
            self,
            Token::With
                | Token::Window
                | Token::Where
                | Token::Draw
                | Token::Keep
                | Token::Hide
                | Token::Table
        )
    }

    pub fn class(self) -> TokenClass {
        match self {
            Token::Points
            | Token::With
            | Token::Window
            | Token::Where
            | Token::Draw
            | Token::Keep
            | Token::Hide
            | Token::Table
            | Token::And
            | Token::Or
            | Token::Not
            | Token::Per => TokenClass::Keyword,
            Token::Number => TokenClass::Number,
            Token::Ident => TokenClass::Ident,
            Token::Comment => TokenClass::Comment,
            Token::Pipe
            | Token::Comma
            | Token::LParen
            | Token::RParen
            | Token::Le
            | Token::Lt
            | Token::Ge
            | Token::Gt
            | Token::EqEq
            | Token::Ne
            | Token::Plus
            | Token::Minus
            | Token::Star
            | Token::Slash
            | Token::Percent => TokenClass::Punctuation,
        }
    }
}

/// A token with its source span and text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tok<'src> {
    pub kind: Token,
    pub span: Span,
    pub text: &'src str,
}

/// Tokenize the whole input for parsing: comments are dropped, and the first
/// offending character is an error.
pub fn lex(src: &str) -> Result<Vec<Tok<'_>>, Diagnostic> {
    let mut lexer = Token::lexer(src);
    let mut toks = Vec::new();
    while let Some(result) = lexer.next() {
        let range = lexer.span();
        let span = Span::new(range.start, range.end);
        match result {
            Ok(Token::Comment) => {}
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

/// Tokenize for syntax highlighting: never fails, covers every non-whitespace
/// byte, and classifies rejected characters as [`TokenClass::Error`].
pub fn highlight_classes(src: &str) -> Vec<(Span, TokenClass)> {
    let mut lexer = Token::lexer(src);
    let mut out = Vec::new();
    while let Some(result) = lexer.next() {
        let range = lexer.span();
        let span = Span::new(range.start, range.end);
        let class = match result {
            Ok(token) => token.class(),
            Err(()) => TokenClass::Error,
        };
        out.push((span, class));
    }
    out
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
    fn comments_and_whitespace_are_dropped_for_parsing() {
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

    #[test]
    fn highlight_classes_cover_comments_and_errors() {
        let classes = highlight_classes("points # hi\n| Draw 3");
        assert_eq!(
            classes,
            vec![
                (Span::new(0, 6), TokenClass::Keyword),
                (Span::new(7, 11), TokenClass::Comment),
                (Span::new(12, 13), TokenClass::Punctuation),
                (Span::new(14, 15), TokenClass::Error),
                (Span::new(15, 18), TokenClass::Ident),
                (Span::new(19, 20), TokenClass::Number),
            ]
        );
    }
}

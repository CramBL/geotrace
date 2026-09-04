//! Rendering of the user-configurable recording display-name template.
//!
//! A template is free text with `{token}` placeholders. Recognised tokens are
//! substituted with the matching field. A token whose field is absent collapses,
//! taking its adjacent literal separator with it, so `"{title} — {device}"` with
//! no device renders as just the title (not `"Alpha — "`). A token may cap its
//! value at `N` characters as `{title:12}`. Unknown tokens are kept verbatim as
//! literal text, and an empty result falls back to the filename.

use std::{borrow::Cow, num::NonZeroUsize};

/// The field values a [`render_name_template`] call draws from.
///
/// `filename` is required and is the ultimate fallback. The caller strips its
/// common prefix beforehand. The rest mirror the optional SDK metadata on a
/// recording.
#[derive(Debug, Clone, Copy)]
pub struct NameFields<'a> {
    pub title: Option<&'a str>,
    pub device: Option<&'a str>,
    pub identity: Option<&'a str>,
    pub filename: &'a str,
}

/// One recognised placeholder token. The `snake_case` wire names are the tokens
/// users type inside `{...}`. Deriving them keeps the set in lockstep with the
/// variants (see the exhaustiveness test).
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    strum::Display,
    strum::EnumCount,
    strum::EnumIter,
    strum::EnumString,
)]
#[strum(serialize_all = "snake_case")]
pub enum Token {
    Title,
    Device,
    Identity,
    Filename,
}

impl Token {
    /// Resolve this token against `fields`. `Filename` never resolves to `None`
    /// (it is the fallback field). Empty values are treated as absent by the
    /// renderer via [`str::is_empty`].
    fn resolve<'a>(self, fields: &NameFields<'a>) -> Option<&'a str> {
        match self {
            Self::Title => fields.title,
            Self::Device => fields.device,
            Self::Identity => fields.identity,
            Self::Filename => Some(fields.filename),
        }
    }
}

/// A token placeholder as written in the template: the token itself and the
/// optional character limit from its `{token:N}` suffix.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TokenPlaceholder {
    token: Token,
    max_chars: Option<NonZeroUsize>,
}

impl TokenPlaceholder {
    /// Parse the text between the braces, `None` for anything that is not a
    /// known token with an optional positive limit.
    fn parse_between_braces(spec: &str) -> Option<Self> {
        let (name, max_chars) = match spec.split_once(':') {
            Some((name, limit)) => {
                // Integer parsing accepts a leading `+`: the digit check keeps
                // the limit to plain decimals.
                if !limit.bytes().all(|byte| byte.is_ascii_digit()) {
                    return None;
                }
                (name, Some(limit.parse::<NonZeroUsize>().ok()?))
            }
            None => (spec, None),
        };
        Some(Self {
            token: name.parse().ok()?,
            max_chars,
        })
    }

    fn resolve_within_limit<'a>(self, fields: &NameFields<'a>) -> Option<Cow<'a, str>> {
        let value = self.token.resolve(fields)?;
        Some(match self.max_chars {
            Some(max_chars) => crate::truncate_with_ellipsis(value, max_chars),
            None => Cow::Borrowed(value),
        })
    }
}

/// A parsed template segment: either literal text or a token placeholder.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Part<'a> {
    Literal(&'a str),
    Token(TokenPlaceholder),
}

/// Split `template` into literal and token parts.
///
/// A `{...}` group whose contents are not a recognised token, or an unmatched
/// `{`, is emitted as literal text (braces included), so a malformed template
/// degrades to showing itself.
fn parse(template: &str) -> Vec<Part<'_>> {
    let mut parts = Vec::new();
    let mut rest = template;
    // `{` is ASCII, so `open` from `find` is always a valid char boundary for
    // `split_at`. `get` is used elsewhere to satisfy the `string_slice` lint and
    // to fall back safely on any unexpected index.
    while let Some(open) = rest.find('{') {
        let (before, from_open) = rest.split_at(open);
        if !before.is_empty() {
            parts.push(Part::Literal(before));
        }
        // `from_open` starts at '{'. Look for the closing brace after it.
        let after_brace = from_open.get(1..).unwrap_or("");
        match after_brace.find('}') {
            Some(rel_close) => {
                let spec = after_brace.get(..rel_close).unwrap_or("");
                match TokenPlaceholder::parse_between_braces(spec) {
                    Some(placeholder) => parts.push(Part::Token(placeholder)),
                    // Unknown token: keep the whole `{spec}` as literal text.
                    None => {
                        let literal = from_open.get(..rel_close + 2).unwrap_or(from_open);
                        parts.push(Part::Literal(literal));
                    }
                }
                rest = after_brace.get(rel_close + 1..).unwrap_or("");
            }
            // Unmatched '{': the remainder is all literal.
            None => {
                parts.push(Part::Literal(from_open));
                rest = "";
            }
        }
    }
    if !rest.is_empty() {
        parts.push(Part::Literal(rest));
    }
    parts
}

/// Render `template` against `fields`.
///
/// Absent (or empty) tokens collapse together with the literal separator that
/// joins them to their neighbour. Leading and trailing literal text next to a
/// present token is kept. If nothing renders, falls back to `fields.filename`.
pub fn render_name_template(template: &str, fields: &NameFields<'_>) -> String {
    let mut out = String::new();
    // Literals seen since the last emitted token, a candidate separator.
    let mut pending = String::new();
    // Whether any token value has been emitted yet.
    let mut emitted_content = false;
    // Whether `pending` follows an absent token (so it is a dangling separator,
    // not genuine leading text).
    let mut pending_after_absent = false;

    for part in parse(template) {
        match part {
            Part::Literal(text) => pending.push_str(text),
            Part::Token(placeholder) => {
                match placeholder
                    .resolve_within_limit(fields)
                    .filter(|value| !value.is_empty())
                {
                    Some(value) => {
                        // Emit the pending literal when it separates prior
                        // content from this token, or when it is genuine leading
                        // text (not a separator orphaned by an absent token).
                        if emitted_content || !pending_after_absent {
                            out.push_str(&pending);
                        }
                        pending.clear();
                        pending_after_absent = false;
                        out.push_str(&value);
                        emitted_content = true;
                    }
                    None => {
                        pending.clear();
                        pending_after_absent = true;
                    }
                }
            }
        }
    }
    // Flush trailing literal text on the same rule a present token uses: keep it
    // when there is prior content to attach to, or when it is genuine leading/
    // standalone text (a template with no resolving tokens). Only a separator
    // left dangling by an absent token with no preceding content is dropped.
    if emitted_content || !pending_after_absent {
        out.push_str(&pending);
    }

    let trimmed = out.trim();
    if trimmed.is_empty() {
        fields.filename.to_owned()
    } else {
        trimmed.to_owned()
    }
}

#[cfg(test)]
#[expect(
    clippy::literal_string_with_formatting_args,
    reason = "template literals intentionally contain {token} placeholders, not format args"
)]
mod tests {
    use super::{NameFields, Token, render_name_template};
    use rstest::rstest;

    fn fields<'a>(
        title: Option<&'a str>,
        device: Option<&'a str>,
        identity: Option<&'a str>,
    ) -> NameFields<'a> {
        NameFields {
            title,
            device,
            identity,
            filename: "ride.gtd",
        }
    }

    #[rstest]
    // Both fields present: separator kept.
    #[case("{title} — {device}", Some("Alpha"), Some("Bravo"), "Alpha — Bravo")]
    // Trailing token absent: its leading separator collapses.
    #[case("{title} — {device}", Some("Alpha"), None, "Alpha")]
    // Leading token absent: its trailing separator collapses.
    #[case("{title} — {device}", None, Some("Bravo"), "Bravo")]
    // Both absent: whole template collapses, falls back to filename.
    #[case("{title} — {device}", None, None, "ride.gtd")]
    // Default template is just the filename.
    #[case("{filename}", Some("Alpha"), Some("Bravo"), "ride.gtd")]
    // Literal-only template renders verbatim.
    #[case("just text", None, None, "just text")]
    // Unknown token kept as literal text.
    #[case("{foo}", Some("Alpha"), None, "{foo}")]
    // Empty template falls back to filename.
    #[case("", Some("Alpha"), Some("Bravo"), "ride.gtd")]
    // Leading affix text kept next to a present token.
    #[case("Track: {title}", Some("Alpha"), None, "Track: Alpha")]
    // Leading affix orphaned by an absent token collapses to the filename.
    #[case("Track: {title}", None, None, "ride.gtd")]
    // Trailing affix text kept next to a present token.
    #[case("{title} ready", Some("Alpha"), None, "Alpha ready")]
    // Substantive trailing prose is kept when earlier content exists, even
    // though an intervening token was absent (consistent with the case above,
    // regardless of whether a later token happens to resolve).
    #[case("{title} — {device} (raw)", Some("Alpha"), None, "Alpha (raw)")]
    fn renders_expected(
        #[case] template: &str,
        #[case] title: Option<&str>,
        #[case] device: Option<&str>,
        #[case] expected: &str,
    ) {
        assert_eq!(
            render_name_template(template, &fields(title, device, None)),
            expected
        );
    }

    #[rstest]
    // A value over the limit is cut to that many characters plus an ellipsis.
    #[case("{title:3}", Some("Alphabet"), "Alp…")]
    // A value within the limit renders unchanged.
    #[case("{title:9}", Some("Alpha"), "Alpha")]
    // An absent token with a limit collapses with its separator as usual.
    #[case("{title:4} — {device}", None, "Bravo")]
    // Each token of a limited pair is cut on its own limit.
    #[case("{title:2} — {device:2}", Some("Alpha"), "Al… — Br…")]
    fn limits_token_length(
        #[case] template: &str,
        #[case] title: Option<&str>,
        #[case] expected: &str,
    ) {
        assert_eq!(
            render_name_template(template, &fields(title, Some("Bravo"), None)),
            expected
        );
    }

    #[rstest]
    // No limit given.
    #[case("{title:}")]
    // Zero cannot cut anything.
    #[case("{title:0}")]
    // Non-numeric limit.
    #[case("{title:abc}")]
    // A signed number: integer parsing would accept a leading sign, the syntax
    // does not.
    #[case("{title:-3}")]
    #[case("{title:+3}")]
    fn malformed_limit_renders_as_literal(#[case] template: &str) {
        assert_eq!(
            render_name_template(template, &fields(Some("Alpha"), None, None)),
            template
        );
    }

    #[test]
    fn middle_token_absent_keeps_one_separator() {
        // A collapsing middle token should not eat both surrounding separators.
        let f = fields(Some("Alpha"), None, Some("Charlie"));
        assert_eq!(
            render_name_template("{title}/{device}/{identity}", &f),
            "Alpha/Charlie"
        );
    }

    #[test]
    fn identity_token_resolves() {
        let f = fields(None, None, Some("auto:ride.gtd"));
        assert_eq!(render_name_template("{identity}", &f), "auto:ride.gtd");
    }

    #[test]
    fn unmatched_brace_is_literal() {
        let f = fields(Some("Alpha"), None, None);
        assert_eq!(render_name_template("{title", &f), "{title");
    }

    #[test]
    fn wire_names_parse_to_variants() {
        use strum::EnumCount;
        // Each wire name a user types maps to a variant. Asserting the table's
        // length against `COUNT` fails the test if a variant is added without a
        // parse case here.
        let cases = [
            ("title", Token::Title),
            ("device", Token::Device),
            ("identity", Token::Identity),
            ("filename", Token::Filename),
        ];
        assert_eq!(cases.len(), Token::COUNT);
        for (name, token) in cases {
            assert_eq!(name.parse::<Token>(), Ok(token));
        }
        "bogus".parse::<Token>().unwrap_err();
    }

    proptest::proptest! {
        /// Rendering never panics for arbitrary user-typed template text,
        /// including stray braces and multi-byte characters around them.
        #[test]
        fn render_never_panics(template in ".*") {
            let _ = render_name_template(&template, &fields(Some("Alpha"), None, Some("auto:x")));
        }

        /// Any limit spec, well-formed or not, renders a multi-byte value
        /// without panicking on a character boundary.
        #[test]
        fn render_limited_token_never_panics(limit in "[^{}]{0,8}") {
            let template = format!("{{title:{limit}}}");
            let _ = render_name_template(&template, &fields(Some("ærøskøbing"), None, None));
        }
    }
}

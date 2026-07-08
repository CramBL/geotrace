//! Position analysis for editor assistance: what is valid at the cursor
//! (autocomplete) and what construct is under it (hover docs).
//!
//! Both tokenize with the lenient [`crate::lexer::tokenize`], which never
//! fails, so they work on the incomplete text an editor holds mid-edit. The
//! pipeline's stage structure makes the position tractable: split on `|`,
//! find the stage the cursor is in, then the expected slot.

use std::collections::HashMap;
use std::ops::Range;
use std::str::FromStr as _;
use std::sync::OnceLock;

use strum::IntoEnumIterator as _;

use crate::ast::{Func, ParamName};
use crate::check::{ChannelInfo, ChannelSchema};
use crate::construct::{Construct, ConstructKind, catalog};
use crate::lexer::{self, Token, TokenClass};
use crate::metric::{Quantity, QueryMetric};
use crate::unit::Unit;

/// Completions offered at a cursor position.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Completions {
    /// Byte range of the partial word to replace when a candidate is
    /// accepted. Empty (`start == end`) when completing at a gap.
    pub range: Range<usize>,
    /// Candidate constructs, best first.
    pub items: Vec<Construct>,
}

/// How a completion request was initiated. The trigger decides how eager the
/// empty-prefix behavior is: while typing, most positions wait for a first
/// character; an explicit request opens on whatever the slot accepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompletionTrigger {
    /// Recomputed as the caret moves or the text changes.
    Automatic,
    /// Explicitly requested (Ctrl+Space in the editor).
    Manual,
}

/// The constructs valid at `cursor` in `src`, fuzzy-filtered and ranked
/// against the partial word under the cursor. The schema types `@channel`
/// references so a unit suggested after `where @accel > 1 ` matches the
/// channel's dimension.
pub fn completions_at(
    src: &str,
    cursor: usize,
    schema: &ChannelSchema,
    trigger: CompletionTrigger,
) -> Completions {
    let cursor = cursor.min(src.len());
    let toks = lexer::tokenize(src);

    // The partial word under the cursor is a name-shaped token the cursor
    // sits within or just after; its span is what accepting replaces.
    let word = toks
        .iter()
        .find(|t| is_wordish(t.kind) && t.span.start < cursor && cursor <= t.span.end);
    let range = word.map_or(cursor..cursor, |t| t.span.start..t.span.end);
    let prefix = src.get(range.start..cursor).unwrap_or("");
    // The whole word the cursor sits in (not just the prefix up to the cursor):
    // used to suppress re-suggesting a word that is already complete.
    let word_text = src.get(range.start..range.end).unwrap_or("");

    // A typed word is only completed when it begins at a real boundary (start
    // of input, or after whitespace, punctuation, an operator, or a digit).
    // This stops the `h` of a finished `km/h` from completing into `heading`.
    if !prefix.is_empty() && !starts_at_boundary(src, range.start) {
        return Completions {
            range,
            items: Vec::new(),
        };
    }

    let slot = slot_before(&toks, range.start);
    // A number that already has a unit right after the cursor needs no unit
    // suggestion (clicking after the `2` in `2 km/h` must not offer units).
    if slot == Slot::Unit && unit_follows(&toks, cursor) {
        return Completions {
            range,
            items: Vec::new(),
        };
    }
    // At a unit position, restrict to the units the value's quantity allows,
    // so `velocity > 30 ` never offers `m` or `g` and `with mask 15 ` offers
    // only `deg`. `None` when the quantity can't be inferred.
    let allowed_units = (slot == Slot::Unit)
        .then(|| allowed_units_at(&toks, range.start, schema))
        .flatten();

    // With nothing typed yet, an automatic request offers candidates only
    // where the offer is small and specific: the units matching an inferred
    // quantity, or the parameter names after `with`. Everywhere else the
    // popup waits for the first character (`points | ` and `table ` must not
    // dump every keyword or metric). A manual request always offers.
    let eager = match slot {
        Slot::Unit => allowed_units.is_some(),
        Slot::Param => true,
        _ => false,
    };
    if prefix.is_empty() && trigger == CompletionTrigger::Automatic && !eager {
        return Completions {
            range,
            items: Vec::new(),
        };
    }

    let mut items: Vec<(i32, Construct)> = catalog()
        .iter()
        .copied()
        .filter(|c| slot.accepts(c))
        .filter(|c| match &allowed_units {
            Some(names) if c.kind == ConstructKind::Unit => names.contains(&c.name),
            _ => true,
        })
        .filter_map(|c| fuzzy_score(prefix, c.name).map(|score| (score, c)))
        .collect();
    // Higher score first; stable, so the catalog order breaks ties.
    items.sort_by_key(|(score, _)| std::cmp::Reverse(*score));

    // The word the cursor sits in is already a complete construct (the caret
    // is inside `velocity`, not building it) - there is nothing to add.
    if !word_text.is_empty()
        && items
            .iter()
            .any(|(_, c)| c.name.eq_ignore_ascii_case(word_text))
    {
        items.clear();
    }

    Completions {
        range,
        items: items.into_iter().map(|(_, c)| c).collect(),
    }
}

/// A channel offered by the editor: its bare name (no `@`) and a one-line
/// summary of its dimension, for the completion popup and the hover tooltip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelSuggestion {
    pub name: String,
    pub summary: String,
}

/// Channel-name completions for a `@channel` reference being typed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelCompletions {
    /// Byte range of the `@partial` an accepted `@name` replaces.
    pub range: Range<usize>,
    /// Matching channels, best first.
    pub items: Vec<ChannelSuggestion>,
}

/// Channel completions at `cursor`, or `None` when the cursor is not typing a
/// `@channel` reference. The typed `@` sigil is the trigger, so a partial
/// `@ac` (or a lone `@`) offers the schema's scalar channels and each vector
/// channel's components (`@accel.x`), ranked against the text after the `@`.
///
/// The sigil is explicit intent, but not unconditional: a `@` inside a
/// comment is prose, and one in a position no channel can occupy (a stage
/// keyword, a `table` column, a `with` parameter) would only complete into a
/// parse error - both stay quiet.
pub fn channel_completions_at(
    src: &str,
    cursor: usize,
    schema: &ChannelSchema,
) -> Option<ChannelCompletions> {
    let (range, prefix) = channel_partial(src, cursor)?;
    if in_comment(src, range.start) {
        return None;
    }
    let toks = lexer::tokenize(src);
    match slot_before(&toks, range.start) {
        // A channel stands where a source or a value name can.
        Slot::Source | Slot::ValueName => {}
        _ => return None,
    }
    let mut scored: Vec<(i32, ChannelSuggestion)> = schema
        .iter_unsorted()
        .flat_map(|(name, info)| channel_offers(name, info))
        .filter_map(|suggestion| {
            fuzzy_score(prefix, &suggestion.name).map(|score| (score, suggestion))
        })
        .collect();
    // Highest score first; the schema iterates in arbitrary (hash) order, so
    // break ties by name for a stable popup.
    scored.sort_by(|(a_score, a), (b_score, b)| {
        b_score.cmp(a_score).then_with(|| a.name.cmp(&b.name))
    });
    Some(ChannelCompletions {
        range,
        items: scored.into_iter().map(|(_, s)| s).collect(),
    })
}

/// The completion entries a channel contributes: a scalar channel offers itself
/// (`@incline`); a vector channel offers the whole vector (`@accel`, for
/// `norm`) and each component (`@accel.x`).
fn channel_offers(name: &str, info: &ChannelInfo) -> Vec<ChannelSuggestion> {
    if info.components.is_empty() {
        return vec![ChannelSuggestion {
            name: name.to_owned(),
            summary: channel_summary(info),
        }];
    }
    let whole = ChannelSuggestion {
        name: name.to_owned(),
        summary: format!("vector ({})", info.components.join(", ")),
    };
    std::iter::once(whole)
        .chain(info.components.iter().map(|component| ChannelSuggestion {
            name: format!("{name}.{component}"),
            summary: channel_summary(info),
        }))
        .collect()
}

/// The channel under `cursor` for a hover tooltip, or `None` when the token is
/// not a channel the schema knows. A component `@accel.x` reads as the channel's
/// dimension; a whole vector `@accel` reads as a vector with its component list.
pub fn channel_at(src: &str, cursor: usize, schema: &ChannelSchema) -> Option<ChannelSuggestion> {
    let cursor = cursor.min(src.len());
    let toks = lexer::tokenize(src);
    let tok = toks
        .iter()
        .find(|t| t.kind == Token::Channel && t.span.start <= cursor && cursor < t.span.end)?;
    let body = tok.text.strip_prefix('@')?;
    let (name, component) = match body.split_once('.') {
        Some((name, component)) => (name, Some(component)),
        None => (body, None),
    };
    let info = schema.get(name)?;
    match component {
        // A named component reads as the channel's dimension, if it exists.
        Some(component) => {
            info.components
                .iter()
                .any(|c| c == component)
                .then(|| ChannelSuggestion {
                    name: format!("{name}.{component}"),
                    summary: channel_summary(info),
                })
        }
        // A scalar channel reads as its dimension; a whole vector names its
        // components, since it has no scalar value on its own.
        None if info.components.is_empty() => Some(ChannelSuggestion {
            name: name.to_owned(),
            summary: channel_summary(info),
        }),
        None => Some(ChannelSuggestion {
            name: name.to_owned(),
            summary: format!("vector ({})", info.components.join(", ")),
        }),
    }
}

/// The `@partial` the cursor is completing: the byte range from the `@` to the
/// end of the partial reference, and the text between the `@` and the cursor.
/// The body may carry a `.component` (`@accel.x`). `None` unless a `@`
/// immediately precedes the reference the cursor sits in.
fn channel_partial(src: &str, cursor: usize) -> Option<(Range<usize>, &str)> {
    let cursor = cursor.min(src.len());
    let before = src.get(..cursor)?;
    let prefix_len = before
        .bytes()
        .rev()
        .take_while(|b| is_body_byte(*b))
        .count();
    // The `@` sits just before the typed body (a name, optionally `.component`).
    let at = cursor.checked_sub(prefix_len)?.checked_sub(1)?;
    if src.as_bytes().get(at) != Some(&b'@') {
        return None;
    }
    // Extend over any body characters after the cursor so accepting replaces the
    // whole `@ref`, not only the part left of the caret.
    let after_len = src
        .get(cursor..)?
        .bytes()
        .take_while(|b| is_body_byte(*b))
        .count();
    Some((at..cursor + after_len, src.get(at + 1..cursor)?))
}

/// Whether `b` is a channel-name character (`[a-z0-9_]`). Mirrors the lexer's
/// `ident` subpattern (see [`crate::lexer`]); keep the two in step if the
/// SDK's channel-name rule changes. All such bytes are single-byte ASCII, so
/// scanning by byte never splits a multi-byte character.
fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_'
}

/// Whether `b` can appear in a channel reference body after the `@`: an ident
/// character or the `.` separating a vector component (`@accel.x`).
fn is_body_byte(b: u8) -> bool {
    is_ident_byte(b) || b == b'.'
}

/// Whether `byte` falls inside a `# …` comment. [`lexer::tokenize`] drops
/// comments, so this goes through the highlighter's classification, which
/// covers them.
fn in_comment(src: &str, byte: usize) -> bool {
    lexer::highlight_classes(src)
        .iter()
        .any(|(span, class)| *class == TokenClass::Comment && span.start <= byte && byte < span.end)
}

/// A one-line description of a channel's dimension for the popup and hover. The
/// `@` prefix and the hover's "channel" label already say it is a channel, so
/// this names only the kind of value.
fn channel_summary(info: &ChannelInfo) -> String {
    match (&info.unit, info.period_deg) {
        (_, Some(_)) => "direction".to_owned(),
        (Some(unit), None) => format!("in {unit}"),
        (None, None) => "unitless".to_owned(),
    }
}

/// The unit names of a quantity, in catalog order, for filtering the unit
/// slot. A direction is written in angle units (`heading > 90 deg`), so it
/// maps to the angle set.
fn units_of_quantity(quantity: Quantity) -> Vec<&'static str> {
    let quantity = match quantity {
        Quantity::Direction => Quantity::Angle,
        other => other,
    };
    Unit::iter()
        .filter(|u| u.quantity() == quantity)
        .map(Unit::text)
        .collect()
}

/// Whether a unit already follows the cursor: the next token starting at or
/// after it is unit-shaped (an identifier, `%`, or `per`). After a number, an
/// identifier can only be its unit, so there is no unit left to suggest.
fn unit_follows(toks: &[lexer::Tok<'_>], cursor: usize) -> bool {
    toks.iter()
        .find(|t| t.span.start >= cursor)
        .is_some_and(|t| matches!(t.kind, Token::Ident | Token::Percent | Token::Per))
}

/// Whether the byte at `start` begins a word at a real boundary: the start of
/// input, or right after whitespace, punctuation, a comparison or arithmetic
/// operator, or a digit (compact typing like `eph>2km` completes at each
/// step). `/` is deliberately not a boundary, so the `h` of a finished `km/h`
/// never completes into `heading`.
fn starts_at_boundary(src: &str, start: usize) -> bool {
    match src.get(..start).and_then(|s| s.chars().next_back()) {
        None => true,
        Some(c) => {
            c.is_whitespace()
                || c.is_ascii_digit()
                || matches!(c, '|' | '(' | ',' | '<' | '>' | '=' | '!' | '+' | '-' | '*')
        }
    }
}

/// The unit names allowed for the value at this position, or `None` when the
/// quantity can't be inferred. Walks left from the number to whatever the
/// value belongs to: a `window` duration, a `with` parameter (`mask` wants an
/// angle, `snr_drop` no unit), or a compared metric or channel (`velocity` a
/// speed, `@accel` its schema unit's quantity).
///
/// The walk covers the whole run back to the start of the condition atom
/// (`where`, a connective, a comma). A dimension-changing operator anywhere in
/// that run (`velocity / eph`, a power) means the found name does not type the
/// value, so the inference honestly gives up rather than suggesting the wrong
/// units. `+`/`-` keep the dimension and are walked through.
fn allowed_units_at(
    toks: &[lexer::Tok<'_>],
    word_start: usize,
    schema: &ChannelSchema,
) -> Option<Vec<&'static str>> {
    let number = toks
        .iter()
        .rposition(|t| t.span.end <= word_start && t.kind == Token::Number)?;
    let mut source: Option<(usize, Quantity)> = None;
    for (idx, tok) in toks.iter().enumerate().take(number).rev() {
        match tok.kind {
            // The start of the value's context: nothing left to scan.
            Token::Where
            | Token::And
            | Token::Or
            | Token::Not
            | Token::With
            | Token::Comma
            | Token::Pipe => break,
            // `window 15 ` - the count-or-duration argument.
            Token::Window => {
                return match source {
                    None => Some(units_of_quantity(Quantity::Duration)),
                    Some(_) => None,
                };
            }
            // The value's dimension is not the found name's; give up.
            Token::Star | Token::Slash | Token::Superscript | Token::CaretPower => return None,
            Token::Ident if source.is_none() => {
                if let Ok(param) = ParamName::from_str(tok.text) {
                    return Some(match param.value_quantity() {
                        Some(quantity) => units_of_quantity(quantity),
                        // A bare number, like snr_drop: no unit belongs here.
                        None => Vec::new(),
                    });
                }
                if let Ok(metric) = QueryMetric::from_str(tok.text) {
                    source = Some((idx, metric.quantity()));
                }
            }
            Token::Channel if source.is_none() => {
                // The channel's quantity comes from its schema unit; a channel
                // with no (or an unknown) unit is a bare number, like the
                // checker types it.
                let quantity = channel_quantity(tok.text, schema)?;
                source = Some((idx, quantity));
            }
            _ => {}
        }
    }
    source.map(|(idx, quantity)| units_of_quantity(wrapped_quantity(toks, idx, quantity)))
}

/// The quantity of a `@channel` reference per the schema, or `None` when the
/// channel is unknown. A channel without a recognisable unit is a bare number
/// (no unit belongs after it), mirroring the checker.
fn channel_quantity(text: &str, schema: &ChannelSchema) -> Option<Quantity> {
    let body = text.strip_prefix('@')?;
    let name = body.split_once('.').map_or(body, |(name, _)| name);
    let info = schema.get(name)?;
    if info.period_deg.is_some() {
        return Some(Quantity::Direction);
    }
    Some(
        info.unit
            .as_deref()
            .and_then(Unit::from_label)
            .map_or(Quantity::Count, Unit::quantity),
    )
}

/// A value's effective quantity, accounting for a directly-wrapping aggregate:
/// `delta(time)` is a duration, `spread(heading)` an angle. The transform is
/// [`Func::result_quantity`], shared with the checker.
fn wrapped_quantity(toks: &[lexer::Tok<'_>], value_idx: usize, quantity: Quantity) -> Quantity {
    let func = value_idx
        .checked_sub(2)
        .filter(|_| {
            toks.get(value_idx - 1)
                .is_some_and(|t| t.kind == Token::LParen)
        })
        .and_then(|i| toks.get(i))
        .and_then(|t| Func::from_str(t.text).ok());
    match func {
        Some(func) => func.result_quantity(quantity),
        None => quantity,
    }
}

/// The construct under the cursor, for hover. `None` over whitespace, an
/// operator, or an unknown word.
///
/// `min` is both the aggregate and the minute unit; a number immediately
/// before it means the unit, otherwise the function - the same disambiguation
/// the parser uses.
pub fn construct_at(src: &str, cursor: usize) -> Option<&'static Construct> {
    let cursor = cursor.min(src.len());
    let toks = lexer::tokenize(src);
    let idx = toks
        .iter()
        .position(|t| t.span.start <= cursor && cursor < t.span.end)?;
    let entries = by_name().get(toks.get(idx)?.text)?;
    if let [only] = entries.as_slice() {
        return Some(only);
    }
    let prev_is_number = idx
        .checked_sub(1)
        .and_then(|i| toks.get(i))
        .is_some_and(|t| t.kind == Token::Number);
    entries
        .iter()
        .find(|c| (c.kind == ConstructKind::Unit) == prev_is_number)
        .or_else(|| entries.first())
}

/// Which family of constructs a position accepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Slot {
    /// The `points` source (start of the query).
    Source,
    /// A stage keyword (just after `|`).
    Stage,
    /// A `with` parameter name.
    Param,
    /// A metric or function (the start of a `where` atom).
    ValueName,
    /// A `table` column - a metric only, never a function.
    Column,
    /// A unit suffix (just after a number).
    Unit,
    /// After a complete `where` atom: `and`, `or`.
    Connective,
    /// Nothing is offered here.
    None,
}

impl Slot {
    fn accepts(self, construct: &Construct) -> bool {
        let kind = construct.kind;
        match self {
            Slot::Source => kind == ConstructKind::Source,
            Slot::Stage => matches!(kind, ConstructKind::Stage | ConstructKind::Mode),
            Slot::Param => kind == ConstructKind::Param,
            // The start of an atom takes a value name, or `not` negating one -
            // but never `and`/`or`, which need a left-hand side.
            Slot::ValueName => {
                matches!(kind, ConstructKind::Metric | ConstructKind::Function)
                    || construct.name == "not"
            }
            Slot::Column => kind == ConstructKind::Metric,
            Slot::Unit => kind == ConstructKind::Unit,
            // `not` starts an atom rather than joining two, so after a
            // complete one only `and`/`or` fit.
            Slot::Connective => kind == ConstructKind::Connective && construct.name != "not",
            Slot::None => false,
        }
    }
}

/// Decide the slot from the tokens strictly before the word being completed.
fn slot_before(toks: &[lexer::Tok<'_>], word_start: usize) -> Slot {
    let before: Vec<Token> = toks
        .iter()
        .filter(|t| t.span.end <= word_start)
        .map(|t| t.kind)
        .collect();

    // Nothing yet, or still inside the source stage: only `points`.
    let Some(last_pipe) = before.iter().rposition(|t| *t == Token::Pipe) else {
        return if before.is_empty() {
            Slot::Source
        } else {
            // After `points` (or a stray token) with no `|` yet.
            Slot::None
        };
    };

    let stage = before.get(last_pipe + 1..).unwrap_or(&[]);
    let Some(keyword) = stage.first() else {
        // Right after `|`: a stage keyword is expected.
        return Slot::Stage;
    };
    let prev = before.last().copied();

    match keyword {
        Token::With => match prev {
            // The first parameter name, or another after a comma.
            Some(Token::With | Token::Comma) => Slot::Param,
            // A unit for the value just typed.
            Some(Token::Number) => Slot::Unit,
            // After a parameter name a number is expected, and after a value's
            // unit a comma is - neither is completable.
            _ => Slot::None,
        },
        Token::Where => match prev {
            Some(Token::Number) => Slot::Unit,
            // Start of an atom: right after `where`, an operator, `(`, or a
            // logical connective.
            Some(
                Token::Where
                | Token::And
                | Token::Or
                | Token::Not
                | Token::LParen
                | Token::Lt
                | Token::Le
                | Token::Gt
                | Token::Ge
                | Token::EqEq
                | Token::Ne
                | Token::Plus
                | Token::Minus
                | Token::Star
                | Token::Slash,
            ) => Slot::ValueName,
            // After a complete atom (metric, `)`, `%`, a unit): a comparison
            // operator or a connective can follow; only the connectives are
            // completable names.
            _ => Slot::Connective,
        },
        Token::Table => match prev {
            // A column name at the start or after a comma.
            Some(Token::Table | Token::Comma) => Slot::Column,
            _ => Slot::None,
        },
        Token::Window => match prev {
            // `window 15 ` - the count may carry a duration unit.
            Some(Token::Number) => Slot::Unit,
            _ => Slot::None,
        },
        // The display modes take nothing after them.
        _ => Slot::None,
    }
}

/// Whether a token is a name-shaped partial word the cursor can complete:
/// identifiers and any keyword (a partially typed keyword lexes as an
/// identifier, but a fully typed one is its keyword token).
fn is_wordish(kind: Token) -> bool {
    !matches!(
        kind,
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
            | Token::Percent
            | Token::Number
    )
}

/// Fuzzy score of `name` against the typed `prefix`, or `None` when it does
/// not match. Exact prefix beats subsequence; shorter names win ties. The
/// language is all-lowercase, so matching is case-insensitive on the input.
fn fuzzy_score(prefix: &str, name: &str) -> Option<i32> {
    if prefix.is_empty() {
        return Some(0);
    }
    let prefix = prefix.to_ascii_lowercase();
    let penalty = i32::try_from(name.len()).unwrap_or(i32::MAX);
    if name.starts_with(&prefix) {
        Some(1000 - penalty)
    } else if is_subsequence(&prefix, name) {
        Some(100 - penalty)
    } else {
        None
    }
}

/// Whether `needle`'s characters appear in `haystack` in order.
fn is_subsequence(needle: &str, haystack: &str) -> bool {
    let mut hay = haystack.chars();
    needle.chars().all(|c| hay.any(|h| h == c))
}

/// Name → construct(s) lookup, built once from the catalog. A name maps to
/// more than one construct only for `min` (aggregate and minute unit).
fn by_name() -> &'static HashMap<&'static str, Vec<Construct>> {
    static MAP: OnceLock<HashMap<&'static str, Vec<Construct>>> = OnceLock::new();
    MAP.get_or_init(|| {
        let mut map: HashMap<&'static str, Vec<Construct>> = HashMap::new();
        for c in catalog() {
            map.entry(c.name).or_default().push(*c);
        }
        map
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The candidate names at `cursor`, typed automatically with an empty
    /// channel schema - the common case for the construct-path tests.
    fn names_at(src: &str, cursor: usize) -> Vec<&'static str> {
        completions_with(
            src,
            cursor,
            &ChannelSchema::new(),
            CompletionTrigger::Automatic,
        )
    }

    fn completions_with(
        src: &str,
        cursor: usize,
        schema: &ChannelSchema,
        trigger: CompletionTrigger,
    ) -> Vec<&'static str> {
        completions_at(src, cursor, schema, trigger)
            .items
            .iter()
            .map(|c| c.name)
            .collect()
    }

    fn names(src: &str) -> Vec<&'static str> {
        names_at(src, src.len())
    }

    #[test]
    fn source_waits_for_a_typed_character() {
        // An empty editor (or the blank line after a query) stays quiet: the
        // popup must not assert `points` before anything is typed - a channel
        // source is an equally valid start.
        assert!(names("").is_empty());
        assert_eq!(names("po"), vec!["points"]);
    }

    #[test]
    fn manual_trigger_offers_at_an_empty_prefix() {
        // Ctrl+Space is explicit intent, so the empty-prefix wait does not
        // apply: the source slot offers `points` on request.
        let manual = completions_with("", 0, &ChannelSchema::new(), CompletionTrigger::Manual);
        assert_eq!(manual, vec!["points"]);
        // A stage position offers the stage keywords and display modes.
        let src = "points | ";
        let stage = completions_with(
            src,
            src.len(),
            &ChannelSchema::new(),
            CompletionTrigger::Manual,
        );
        assert!(stage.contains(&"where"));
        assert!(stage.contains(&"draw"));
    }

    #[test]
    fn stage_keywords_after_a_typed_character() {
        // Nothing right after `|`: stage keywords and display modes are two
        // kinds, so the popup waits for the first character.
        assert!(names("points | ").is_empty());

        let items = names("points | w");
        assert!(items.contains(&"where"));
        assert!(items.contains(&"window"));
        assert!(items.contains(&"with"));
        // A display mode is reachable by its own letters.
        assert!(names("points | d").contains(&"draw"));
        // No metrics offered at a stage-keyword position.
        assert!(!names("points | wi").contains(&"velocity"));
    }

    #[test]
    fn partial_stage_keyword_fuzzy_filters() {
        // "wh": `where` is a prefix (ranks first); `with` matches as a
        // subsequence (w..h); `window` has no 'h' so it is filtered out.
        let items = names("points | wh");
        assert_eq!(items.first(), Some(&"where"));
        assert!(items.contains(&"with"));
        assert!(!items.contains(&"window"));
        assert!(!items.contains(&"table"));
    }

    #[test]
    fn metrics_and_functions_after_a_typed_character() {
        // Nothing right after `where`: metrics and functions are two kinds.
        assert!(names("points | where ").is_empty());
        assert!(names("points | where v").contains(&"velocity"));
        assert!(names("points | where a").contains(&"avg"));
        // After a metric, an operator is expected - no names.
        assert!(names("points | where velocity ").is_empty());
    }

    #[test]
    fn only_matching_units_after_a_number() {
        // `velocity` is a speed, so a unit there is a speed unit - never `m`,
        // `g`, or `deg`.
        let items = names("points | where velocity > 30 ");
        assert!(items.contains(&"km/h"));
        assert!(items.contains(&"m/s"));
        assert!(items.contains(&"kn"));
        assert!(!items.contains(&"m"));
        assert!(!items.contains(&"g"));
        assert!(!items.contains(&"deg"));
        assert!(!items.contains(&"velocity"));
        // A length metric gets length units.
        let lengths = names("points | where eph > 20 ");
        assert!(lengths.contains(&"m"));
        assert!(lengths.contains(&"km"));
        assert!(!lengths.contains(&"km/h"));
    }

    #[test]
    fn duration_units_after_delta_of_time() {
        // delta(time) is a duration, so its unit is a duration unit.
        let items = names("points | window 3 | where delta(time) <= 15 ");
        assert!(items.contains(&"min"));
        assert!(items.contains(&"s"));
        assert!(!items.contains(&"km/h"));
    }

    #[test]
    fn nothing_glued_to_a_finished_unit() {
        // The `h` of `km/h` sits after `/`, not a boundary, so it is not
        // completed into `heading` or `eph`.
        let items = names("points | window 3 | where avg(velocity) > 30 km/h");
        assert!(items.is_empty(), "expected no completions, got {items:?}");
    }

    #[test]
    fn caret_inside_a_complete_word_offers_nothing() {
        // The caret sits inside `velocity` (after the `l`), which is already a
        // complete metric - re-suggesting it is pointless.
        let src = "points | where velocity < 2 km/h | hide";
        let cursor = src.find("velocity").expect("has velocity") + 3;
        assert!(names_at(src, cursor).is_empty());
    }

    #[test]
    fn no_unit_offered_when_one_already_follows() {
        // The caret sits right after the `2`, which already has `km/h` after
        // it - there is no unit to add.
        let src = "points | where velocity < 2 km/h | hide";
        let cursor = src.find("2 km/h").expect("has the literal") + 1;
        assert!(names_at(src, cursor).is_empty());
    }

    #[test]
    fn params_in_with() {
        let items = names("points | with ");
        assert_eq!(
            {
                let mut i = items.clone();
                i.sort_unstable();
                i
            },
            vec!["mask", "slip_window", "snr_drop"]
        );
    }

    #[test]
    fn with_expects_a_value_then_the_right_unit() {
        // After a parameter name a number is expected - nothing to complete.
        assert!(names("points | with mask ").is_empty());
        // The unit after the number is the parameter's quantity: mask is an
        // angle, so only `deg`.
        let mask_units = names("points | with mask 15 ");
        assert!(mask_units.contains(&"deg"));
        assert!(!mask_units.contains(&"m"));
        assert!(!mask_units.contains(&"km/h"));
        // slip_window is a duration.
        let slip_units = names("points | with mask 15 deg, slip_window 5 ");
        assert!(slip_units.contains(&"min"));
        assert!(slip_units.contains(&"s"));
        assert!(!slip_units.contains(&"deg"));
        // snr_drop is a bare number, so no unit is offered.
        assert!(names("points | with mask 15 deg, snr_drop 10 ").is_empty());
        // A comma starts the next parameter name.
        assert!(names("points | with mask 15 deg, ").contains(&"snr_drop"));
    }

    #[test]
    fn columns_in_table() {
        // `table ` must not dump every metric unprompted - the popup waits
        // for the first character, then offers only metrics.
        assert!(names("points | where velocity > 30 km/h | table ").is_empty());
        let items = names("points | where velocity > 30 km/h | table v");
        assert!(items.contains(&"velocity"));
        assert!(
            !items.contains(&"avg"),
            "columns are metrics, not functions"
        );
        let after_comma = names("points | where velocity > 30 km/h | table time, h");
        assert!(after_comma.contains(&"heading"));
    }

    #[test]
    fn nothing_after_display_modes() {
        assert!(names("points | draw ").is_empty());
    }

    #[test]
    fn duration_units_after_a_window_count() {
        // `window 15 s` is valid grammar, so the count position offers the
        // duration units - and only those.
        let items = names("points | window 15 ");
        assert!(items.contains(&"s"));
        assert!(items.contains(&"min"));
        assert!(!items.contains(&"km/h"));
        assert!(!items.contains(&"deg"));
    }

    #[test]
    fn connectives_complete_after_an_atom() {
        // After a complete comparison, a typed character offers the joining
        // connectives - but nothing pops eagerly, and `not` (which starts an
        // atom rather than joining two) is not offered there.
        assert!(names("points | where velocity > 30 km/h ").is_empty());
        assert_eq!(names("points | where velocity > 30 km/h a"), vec!["and"]);
        assert_eq!(names("points | where velocity > 30 km/h o"), vec!["or"]);
        assert!(!names("points | where velocity > 30 km/h n").contains(&"not"));
        // At the start of an atom `not` is offered alongside value names.
        assert!(names("points | where velocity > 30 km/h and no").contains(&"not"));
    }

    #[test]
    fn channels_type_the_unit_slot_through_the_schema() {
        // `@accel` is m/s2 in the schema, so the unit offered after a
        // comparison against it is an acceleration - never a speed.
        let src = "points | where @accel > 1 ";
        let items = completions_with(
            src,
            src.len(),
            &channel_schema(),
            CompletionTrigger::Automatic,
        );
        assert!(items.contains(&"m/s2"));
        assert!(items.contains(&"g"));
        assert!(!items.contains(&"km/h"));
    }

    #[test]
    fn arithmetic_between_metrics_gives_up_instead_of_guessing() {
        // `velocity / eph` is dimensionless; suggesting length units (from
        // `eph`) would be wrong, so nothing pops eagerly...
        assert!(names("points | where velocity / eph > 3 ").is_empty());
        // ...while a typed prefix still completes over the full unit set (the
        // honest fallback when the quantity is unknown).
        let typed = names("points | where velocity / eph > 3 k");
        assert!(typed.contains(&"km/h"));
        assert!(typed.contains(&"km"));
    }

    #[test]
    fn a_leading_literal_offers_no_eager_units() {
        // `where 30 ` (intending `30 km/h < velocity`) has nothing to type the
        // literal yet - offering every unit would be noise.
        assert!(names("points | where 30 ").is_empty());
    }

    #[test]
    fn compact_typing_completes_after_operators_and_digits() {
        // No spaces anywhere: the metric after `(`, the unit after the digit.
        assert!(names("points | where eph>2k").contains(&"km"));
        assert!(names("points | where avg(velocity)>3 k").contains(&"km/h"));
        // A name right after a comparison operator completes too.
        assert!(names("points | where eph>vel").contains(&"velocity"));
    }

    #[test]
    fn replace_range_covers_the_partial_word() {
        // "vel" starts at byte 15 in "points | where vel".
        let completions = completions_at(
            "points | where vel",
            18,
            &ChannelSchema::new(),
            CompletionTrigger::Automatic,
        );
        assert_eq!(completions.range, 15..18);
        assert_eq!(completions.items.first().map(|c| c.name), Some("velocity"));
    }

    #[test]
    fn construct_under_cursor_for_hover() {
        // Cursor inside "spread".
        let c = construct_at("points | window 3 | where spread(heading) < 10 deg", 28).unwrap();
        assert_eq!(c.name, "spread");
        assert_eq!(c.kind, ConstructKind::Function);
        // Over the metric.
        assert_eq!(
            construct_at("points | where velocity > 0 km/h", 18).map(|c| c.name),
            Some("velocity")
        );
        // Over whitespace: nothing.
        assert!(construct_at("points | where velocity", 6).is_none());
    }

    #[test]
    fn fuzzy_prefix_beats_subsequence() {
        // "sl" prefixes slip_* and is a subsequence of slip_window etc.
        assert!(fuzzy_score("sl", "slip_all") > fuzzy_score("sl", "sats_fix"));
        assert_eq!(fuzzy_score("xyz", "velocity"), None);
        // Empty prefix matches everything at a neutral score.
        assert_eq!(fuzzy_score("", "anything"), Some(0));
    }

    /// accel (m/s2), incline (unitless), bearing (a direction), and the vector
    /// gyro - one of each shape the summary and the scalar filter distinguish.
    fn channel_schema() -> ChannelSchema {
        let mut schema = ChannelSchema::new();
        schema.insert(
            "accel",
            ChannelInfo {
                unit: Some("m/s2".to_owned()),
                period_deg: None,
                components: vec![],
            },
        );
        schema.insert(
            "incline",
            ChannelInfo {
                unit: None,
                period_deg: None,
                components: vec![],
            },
        );
        schema.insert(
            "bearing",
            ChannelInfo {
                unit: Some("deg".to_owned()),
                period_deg: Some(360.0),
                components: vec![],
            },
        );
        schema.insert(
            "gyro",
            ChannelInfo {
                unit: Some("deg".to_owned()),
                period_deg: None,
                components: vec!["x".to_owned(), "y".to_owned(), "z".to_owned()],
            },
        );
        schema
    }

    fn channel_names(src: &str, cursor: usize) -> Vec<String> {
        channel_completions_at(src, cursor, &channel_schema())
            .map(|c| c.items.into_iter().map(|s| s.name).collect())
            .unwrap_or_default()
    }

    #[test]
    fn a_lone_at_offers_channels_and_vector_components() {
        // Scalar channels by name, the whole vector `gyro` (for norm) plus each
        // of its components, all sorted by name.
        let src = "points | window 3 | where max(@";
        assert_eq!(
            channel_names(src, src.len()),
            vec![
                "accel", "bearing", "gyro", "gyro.x", "gyro.y", "gyro.z", "incline"
            ]
        );
    }

    #[test]
    fn an_at_at_the_start_offers_channels_as_sources() {
        // The `@` sigil triggers channel completion anywhere, including the
        // source position, so `@acc` at the query start offers accel as a source.
        let names = channel_names("@acc", "@acc".len());
        assert_eq!(names, vec!["accel"]);
    }

    #[test]
    fn a_component_prefix_offers_the_matching_components() {
        // `@gyro.` offers every component of gyro.
        let dot = "points | window 3 | where max(@gyro.";
        assert_eq!(
            channel_names(dot, dot.len()),
            vec!["gyro.x", "gyro.y", "gyro.z"]
        );
        // `@gyro.y` narrows to the one component, replacing the whole `@gyro.y`.
        let one = "points | window 3 | where max(@gyro.y";
        let completions = channel_completions_at(one, one.len(), &channel_schema()).unwrap();
        let at = one.find("@gyro.y").expect("has @gyro.y");
        assert_eq!(completions.range, at..at + "@gyro.y".len());
        assert_eq!(
            completions
                .items
                .iter()
                .map(|s| s.name.as_str())
                .collect::<Vec<_>>(),
            vec!["gyro.y"]
        );
    }

    #[test]
    fn a_channel_prefix_filters_the_offer() {
        // `@in` prefixes incline (ranked first); it is a subsequence of
        // bear-in-g too, but accel has no 'i' then 'n' so it drops out.
        let incline = "points | window 3 | where max(@in";
        let names = channel_names(incline, incline.len());
        assert_eq!(names.first().map(String::as_str), Some("incline"));
        assert!(!names.contains(&"accel".to_owned()));
        // `@b` prefixes only bearing.
        let bearing = "points | window 3 | where max(@b";
        assert_eq!(channel_names(bearing, bearing.len()), vec!["bearing"]);
    }

    #[test]
    fn channel_completion_replaces_the_at_partial() {
        let src = "points | window 3 | where max(@ac)";
        let at = src.find("@ac").expect("has @ac");
        // Cursor just after `@ac`, before the `)`.
        let completions = channel_completions_at(src, at + 3, &channel_schema()).unwrap();
        assert_eq!(completions.range, at..at + 3);
        assert_eq!(
            completions.items.first().map(|s| s.name.as_str()),
            Some("accel")
        );
    }

    #[test]
    fn no_channel_completion_without_an_at() {
        let src = "points | where velocity";
        assert!(channel_completions_at(src, src.len(), &channel_schema()).is_none());
    }

    #[test]
    fn no_channel_completion_inside_a_comment() {
        // A `@` in a comment is prose, not a reference being typed.
        for src in ["# see @", "points | draw # ping @a"] {
            assert!(
                channel_completions_at(src, src.len(), &channel_schema()).is_none(),
                "no channel popup in {src:?}"
            );
        }
    }

    #[test]
    fn no_channel_completion_where_a_channel_cannot_go() {
        // Accepting a channel at a stage keyword, a `table` column, or a
        // `with` parameter would insert an immediate parse error.
        for src in ["points | @", "points | table @", "points | with mask @"] {
            assert!(
                channel_completions_at(src, src.len(), &channel_schema()).is_none(),
                "no channel popup in {src:?}"
            );
        }
        // Value positions still offer on the sigil.
        let value = "points | where @";
        assert!(channel_completions_at(value, value.len(), &channel_schema()).is_some());
    }

    #[test]
    fn channel_hover_describes_the_dimension() {
        let src = "points | window 3 | where max(@accel) > 1 g";
        let inside = src.find("@accel").expect("has @accel") + 2;
        let hover = channel_at(src, inside, &channel_schema()).expect("hovers a channel");
        assert_eq!(hover.name, "accel");
        assert_eq!(hover.summary, "in m/s2");

        // A direction channel reads as one; a position off any channel has no
        // hover.
        let dir = "points | window 3 | where spread(@bearing) < 5 deg";
        let bi = dir.find("@bearing").expect("has @bearing") + 2;
        assert_eq!(
            channel_at(dir, bi, &channel_schema()).map(|s| s.summary),
            Some("direction".to_owned())
        );
        assert!(channel_at(dir, 0, &channel_schema()).is_none());
    }

    #[test]
    fn channel_hover_handles_components_and_whole_vectors() {
        // A component reads as the channel's dimension.
        let comp = "points | window 3 | where max(@gyro.x) > 1 deg";
        let ci = comp.find("@gyro.x").expect("has @gyro.x") + 2;
        let hover = channel_at(comp, ci, &channel_schema()).expect("hovers a component");
        assert_eq!(hover.name, "gyro.x");
        assert_eq!(hover.summary, "in deg");

        // A whole vector names its components; an unknown component has no hover.
        let whole = "points | window 3 | where max(@gyro) > 1 deg";
        let gi = whole.find("@gyro").expect("has @gyro") + 2;
        assert_eq!(
            channel_at(whole, gi, &channel_schema()).map(|s| s.summary),
            Some("vector (x, y, z)".to_owned())
        );
        let bad = "points | window 3 | where max(@gyro.w) > 1 deg";
        let wi = bad.find("@gyro.w").expect("has @gyro.w") + 2;
        assert!(channel_at(bad, wi, &channel_schema()).is_none());
    }

    mod properties {
        use proptest::prelude::*;

        use super::channel_schema;
        use crate::position::CompletionTrigger;
        use crate::{channel_at, channel_completions_at, completions_at, construct_at, lexer};

        proptest! {
            /// Editor assistance runs on arbitrary, incomplete text and an
            /// externally-driven caret, so it must never panic - including a
            /// cursor past the end of the text or inside a multi-byte char.
            #[test]
            fn never_panics(src in ".*", cursor in 0usize..256) {
                let schema = channel_schema();
                let _ = lexer::tokenize(&src);
                let _ = completions_at(&src, cursor, &schema, CompletionTrigger::Automatic);
                let _ = completions_at(&src, cursor, &schema, CompletionTrigger::Manual);
                let _ = construct_at(&src, cursor);
                let _ = channel_completions_at(&src, cursor, &schema);
                let _ = channel_at(&src, cursor, &schema);
            }
        }
    }
}

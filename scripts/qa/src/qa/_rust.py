"""Reading Rust source as code alone, with its comments and strings blanked out."""

import bisect
import re
from typing import NamedTuple

# Where a comment or a string literal opens. A raw string keeps the hashes of
# its opening in group 1, which its closing repeats.
_LITERAL_START = re.compile(r"//|/\*|(?<![\w])b?r(#*)\"|\"|'")

# A char literal, which holds a quote of its own in `'"'` and must not open a
# string. Where this matches nothing, the `'` opens a lifetime.
_CHAR_LITERAL = re.compile(r"'(?:\\.|[^\\'])'")


def _blank(masked: list[str], start: int, end: int) -> None:
    for index in range(start, min(end, len(masked))):
        if masked[index] != "\n":
            masked[index] = " "


def _end_of_block_comment(source: str, start: int) -> int:
    depth = 0
    index = start
    while index < len(source):
        if source.startswith("/*", index):
            depth += 1
            index += 2
        elif source.startswith("*/", index):
            depth -= 1
            index += 2
            if depth == 0:
                return index
        else:
            index += 1
    return len(source)


def _end_of_string(source: str, start: int) -> int:
    index = start + 1
    while index < len(source):
        if source[index] == "\\":
            index += 2
        elif source[index] == '"':
            return index + 1
        else:
            index += 1
    return len(source)


def mask_comments_and_strings(source: str) -> str:
    """`source` with every comment and string literal blanked to spaces.

    Line numbers and offsets stay as they are. A check then reads code alone: a
    `use` line or a `mod` line written inside a block comment or inside a raw
    string holding Rust source declares nothing, and a call written in either is
    no call. An unterminated comment or string is blanked to the end of the file.
    """
    masked = list(source)
    index = 0
    while (found := _LITERAL_START.search(source, index)) is not None:
        start = found.start()
        opening = found.group(0)
        if opening == "//":
            end = source.find("\n", start)
            end = len(source) if end < 0 else end
        elif opening == "/*":
            end = _end_of_block_comment(source, start)
        elif opening == '"':
            end = _end_of_string(source, start)
        elif opening == "'":
            char_literal = _CHAR_LITERAL.match(source, start)
            if char_literal is None:
                index = start + 1
                continue
            end = char_literal.end()
        else:
            closing = source.find('"' + found.group(1), found.end())
            end = len(source) if closing < 0 else closing + 1 + len(found.group(1))
        _blank(masked, start, end)
        index = end
    return "".join(masked)


# `#[cfg(` on an item, and `#![cfg(` on the file that opens with it, up to the
# opening parenthesis of the predicate.
_CFG_ATTRIBUTE = re.compile(r"#(?P<inner>!?)(?P<bracket>\[)cfg\(")

# The features `CODE_STYLE.md` defines as adding test helpers to a crate.
_TEST_ONLY_FEATURES = frozenset({"fixtures", "test-util"})

_CFG_PREDICATE_TOKEN = re.compile(r'"[^"]*"|[\w-]+|[(),=]')


def line_numbers_gated_for_tests(source: str) -> frozenset[int]:
    """The 1-based line numbers of the regions of `source` gated for tests alone.

    A `cfg` attribute gates a region for tests alone when its predicate is
    `test`, a feature in `_TEST_ONLY_FEATURES`, an `all(…)` with such a
    predicate among its operands, or an `any(…)` whose every operand is such a
    predicate. `not(…)` never is: `not(test)` gates production code.

    An inner attribute gates every line of the file it opens. An outer attribute
    gates the one item under it, which ends at the closing brace of its block or
    at the semicolon of a declaration. Production code below a `mod tests`, and
    production code between two gated items, sits outside every region.
    """
    masked = mask_comments_and_strings(source)
    line_starts = [0, *(index + 1 for index, char in enumerate(masked) if char == "\n")]
    numbers: set[int] = set()
    for attribute in _CFG_ATTRIBUTE.finditer(masked):
        predicate_end = _end_of_brackets(masked, attribute.end() - 1)
        tokens = _CFG_PREDICATE_TOKEN.findall(source, attribute.end(), predicate_end - 1)
        if not _read_cfg_predicate(tokens, 0).test_only:
            continue
        if attribute.group("inner"):
            return frozenset(range(1, len(source.splitlines()) + 1))
        end = _end_of_gated_item(masked, _end_of_brackets(masked, attribute.start("bracket")))
        first = bisect.bisect_right(line_starts, attribute.start())
        last = bisect.bisect_right(line_starts, end - 1)
        numbers.update(range(first, last + 1))
    return frozenset(numbers)


class _CfgPredicate(NamedTuple):
    test_only: bool
    next_token: int


def _read_cfg_predicate(tokens: list[str], start: int) -> _CfgPredicate:
    """Whether the `cfg` predicate opening at `tokens[start]` gates for tests alone,
    and the index of the token after it."""
    name = tokens[start] if start < len(tokens) else ""
    index = start + 1
    if tokens[index : index + 1] == ["="]:
        value = tokens[index + 1].strip('"') if index + 1 < len(tokens) else ""
        return _CfgPredicate(name == "feature" and value in _TEST_ONLY_FEATURES, index + 2)
    if tokens[index : index + 1] != ["("]:
        return _CfgPredicate(name == "test", index)
    index += 1
    operands: list[bool] = []
    while index < len(tokens) and tokens[index] != ")":
        if tokens[index] == ",":
            index += 1
            continue
        operand = _read_cfg_predicate(tokens, index)
        operands.append(operand.test_only)
        index = operand.next_token
    index += 1
    if name == "all":
        return _CfgPredicate(any(operands), index)
    if name == "any":
        return _CfgPredicate(bool(operands) and all(operands), index)
    return _CfgPredicate(False, index)


def _end_of_brackets(masked: str, start: int) -> int:
    """The offset just past the bracket that closes the one at `start`."""
    depth = 0
    for index in range(start, len(masked)):
        if masked[index] in "([":
            depth += 1
        elif masked[index] in ")]":
            depth -= 1
            if depth == 0:
                return index + 1
    return len(masked)


def _end_of_gated_item(masked: str, start: int) -> int:
    """The offset just past the item an attribute ending at `start` gates.

    Bracket depth skips a further attribute and a type between the two, so the
    first `;` or `{` found outside every bracket belongs to the item itself.
    """
    depth = 0
    index = start
    while index < len(masked):
        char = masked[index]
        if char in "([":
            depth += 1
        elif char in ")]":
            depth -= 1
        elif depth == 0 and char == ";":
            return index + 1
        elif depth == 0 and char == "{":
            return _end_of_block(masked, index)
        index += 1
    return len(masked)


def _end_of_block(masked: str, start: int) -> int:
    depth = 0
    for index in range(start, len(masked)):
        if masked[index] == "{":
            depth += 1
        elif masked[index] == "}":
            depth -= 1
            if depth == 0:
                return index + 1
    return len(masked)

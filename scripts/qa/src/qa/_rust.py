"""Reading Rust source as code alone, with its comments and strings blanked out."""

import bisect
import re

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


# `#[cfg(test)]` on an item, and `#![cfg(test)]` on the file that opens with it.
_CFG_TEST_ATTRIBUTE = re.compile(r"#\[cfg\(test\)\]")
_INNER_CFG_TEST_ATTRIBUTE = re.compile(r"#!\[cfg\(test\)\]")


def cfg_test_line_numbers(source: str) -> frozenset[int]:
    """The 1-based line numbers of the `#[cfg(test)]` regions of `source`.

    `#![cfg(test)]` gates the file it opens, so every line of it counts. An
    outer `#[cfg(test)]` gates the one item under it, which ends at the closing
    brace of its block or at the semicolon of a declaration. Production code
    below a `mod tests`, and production code between two gated items, sits
    outside every region.
    """
    masked = mask_comments_and_strings(source)
    if _INNER_CFG_TEST_ATTRIBUTE.search(masked) is not None:
        return frozenset(range(1, len(source.splitlines()) + 1))
    line_starts = [0, *(index + 1 for index, char in enumerate(masked) if char == "\n")]
    numbers: set[int] = set()
    for attribute in _CFG_TEST_ATTRIBUTE.finditer(masked):
        end = _end_of_gated_item(masked, attribute.end())
        first = bisect.bisect_right(line_starts, attribute.start())
        last = bisect.bisect_right(line_starts, end - 1)
        numbers.update(range(first, last + 1))
    return frozenset(numbers)


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

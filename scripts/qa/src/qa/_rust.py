"""Reading Rust source as code alone, with its comments and strings blanked out."""

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

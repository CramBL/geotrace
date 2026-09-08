"""Forbid fully importing a function, in Rust source.

CODE_STYLE requires a type to be imported by its short name and a function to
keep at least one module level: `fs::read_to_string(path)` names where the
function comes from, and `read_to_string(path)` does not.

A name alone does not say whether it is a module or a function: `mercator` and
`to_pixel` are both `snake_case`. A rule keyed on module names would have to
list every module of every dependency, and that list is where it produces false
positives. This check reads the call site instead. The use itself is the
evidence, because the file that imports a name is the file that uses it: a name
the file calls as `name(…)` or `name::<T>(…)`, with no module qualifying it, is
a function. None of the three other forms holds such a call. A module appears as
`name::item`, a macro as `name!`, and a lowercase type such as uom's `meter`
only as a generic argument (`Length::new::<meter>`). A re-export raises nothing
either: the file it sits in does not call the name.

Every leaf of a `use` tree is in scope, over as many lines as the statement
spans: an import indented inside a `mod tests`, and a function listed beside
types in one brace group.

Exemption syntax (on any line of the `use` statement):

    use foo::bar::helper; // [qa-allow-check-function-imports, reason = "why"]
"""

import argparse
import re
import sys
from collections.abc import Iterator
from pathlib import Path

from qa._allow import is_exempt
from qa._check import (
    Check,
    Violation,
    added_lines_since,
    merge_base_with_head,
    repo_root,
    rs_files,
    run_check,
    run_check_on_added,
)

CHECK = "check-function-imports"

_USE_PREFIX = re.compile(r"^\s*(?:pub(?:\s*\([^)]*\))?\s+)?use\s+")

# A `snake_case` leaf: a function, a module, or one of the few lowercase types.
_SNAKE_CASE = re.compile(r"[a-z][a-z0-9_]*")

# Path keywords that refer to no item, and the `_` alias of a trait import.
_NOT_AN_ITEM = frozenset({"self", "super", "crate", "_"})

# `fn name(` defines the function, and `#[name(…)]` invokes an attribute macro.
# This check counts neither as a call: a `mod tests` that imports a function from
# `super` sits in the file that defines it.
_NOT_A_CALL = re.compile(r"(?:\bfn\s+|#\[)$")

# What `_NOT_A_CALL` needs to match on: `#[` is two characters and `fn ` three,
# with room for the whitespace a hand-formatted line holds between them.
_NOT_A_CALL_WINDOW = 8


def _has_an_unqualified_call(source: str, name: str) -> bool:
    call = re.compile(rf"(?<![\w.:]){re.escape(name)}\s*(?:\(|::\s*<)")
    return any(
        _NOT_A_CALL.search(source[max(0, found.start() - _NOT_A_CALL_WINDOW) : found.start()])
        is None
        for found in call.finditer(source)
    )


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

    Line numbers and offsets stay as they are. The scan below then reads code
    alone: a `use` line written inside a block comment or inside a raw string
    holding Rust source is no import, and a call written in either is no call.
    An unterminated comment or string is blanked to the end of the file.
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


def use_statements(lines: list[str]) -> Iterator[tuple[int, str]]:
    """Every `use` statement, as its first line's index and its text up to the `;`.

    An offset into the text maps back to a line of the file: the text keeps the
    newlines of a statement written over several lines.
    """
    start: int | None = None
    collected: list[str] = []
    for index, line in enumerate(lines):
        if start is None:
            if _USE_PREFIX.match(line) is None:
                continue
            start = index
        collected.append(line)
        if line.rstrip().endswith(";"):
            yield start, "\n".join(collected)
            start = None
            collected = []


def _split_at_top_level_commas(text: str) -> Iterator[tuple[str, int]]:
    depth = 0
    item_start = 0
    for offset, char in enumerate(text):
        if char == "{":
            depth += 1
        elif char == "}":
            depth -= 1
        elif char == "," and depth == 0:
            yield text[item_start:offset], item_start
            item_start = offset + 1
    yield text[item_start:], item_start


def _leaves(text: str, offset: int) -> Iterator[tuple[str, int]]:
    for item, item_offset in _split_at_top_level_commas(text):
        opening = item.find("{")
        if opening >= 0:
            inner = item[opening + 1 : item.rfind("}")]
            yield from _leaves(inner, offset + item_offset + opening + 1)
            continue
        leaf = item.strip()
        if not leaf:
            continue
        path, _, alias = leaf.partition(" as ")
        name = alias.strip() if alias else path.strip().rpartition("::")[2].strip()
        yield name, offset + item_offset + item.rfind(name)


def imported_names(statement: str) -> Iterator[tuple[str, int]]:
    """Every name a `use` tree binds, with its offset in `statement`.

    A leaf with an alias binds the alias, and a leaf without one binds the last
    segment of its path.
    """
    prefix = _USE_PREFIX.match(statement)
    start = 0 if prefix is None else prefix.end()
    yield from _leaves(statement[start:].rstrip().removesuffix(";"), start)


def _violations_in(path: Path) -> Iterator[Violation]:
    source = path.read_text(errors="replace")
    code = mask_comments_and_strings(source)
    lines = source.splitlines()
    for start, statement in use_statements(code.splitlines()):
        end = start + statement.count("\n")
        if any(is_exempt(line, CHECK) for line in lines[start : end + 1]):
            continue
        for name, offset in imported_names(statement):
            if name in _NOT_AN_ITEM or _SNAKE_CASE.fullmatch(name) is None:
                continue
            if _has_an_unqualified_call(code, name):
                lineno = start + statement[:offset].count("\n")
                yield path, lineno + 1, f"{name} - {lines[lineno].strip()}"


def _collect(root: Path) -> list[Violation]:
    return [violation for path in rs_files(root) for violation in _violations_in(path)]


_NOTE = [
    "this file calls the name with no module before it, and the import hides where",
    "the function comes from.",
    "CODE_STYLE keeps at least the parent module on a function.",
]
_HELP = [
    "import the parent module and qualify the call (`use std::fs;` then",
    "`fs::read_to_string(path)`), or exempt with:",
]

DEFINITION = Check(
    name=CHECK,
    title="a fully imported function found",
    collect=_collect,
    note=_NOTE,
    help=_HELP,
)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo-root", type=Path, default=repo_root())
    parser.add_argument(
        "--added",
        metavar="BASE",
        help="check the lines added since BASE, instead of every tracked file",
    )
    args = parser.parse_args()

    root: Path = args.repo_root.resolve()
    if args.added is None:
        failed = run_check(DEFINITION, root)
    else:
        added = added_lines_since(root, merge_base_with_head(root, args.added))
        failed = run_check_on_added(DEFINITION, root, added)
    if failed:
        sys.exit(1)

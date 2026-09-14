"""Forbid fully importing a function, in Rust source.

CODE_STYLE requires a type to be imported by its short name and a function to
keep at least one module level: `fs::read_to_string(path)` names where the
function comes from, and `read_to_string(path)` does not.

A name alone does not say whether it is a module or a function: `mercator` and
`to_pixel` are both `snake_case`. A rule keyed on module names would have to
list every module of every dependency, and that list is where it produces false
positives. This check reads the uses instead. The use itself is the evidence,
because the file that imports a name is the file that uses it: a name the file
calls as `name(…)` or `name::<T>(…)`, with no module qualifying it, is a
function. None of the three other forms holds such a call. A module appears as
`name::item`, a macro as `name!`, and a lowercase type such as uom's `meter` or
std's `c_char` after `<`, `*const`, `*mut`, `as` or `->`, or in an array type
(`[c_char; 256]`). A re-export raises nothing either: the file it sits in does
not use the name.

A name the file passes as a bare value, as in `.map(name)`, is a function too.
This check skips every bare value of a name that the file uses anywhere as a
module, a macro or a type. A local binding can share the name of an imported
module. In `transform::lod_points(transform)`, the second `transform` is a
parameter with the name of an imported module.

This check reads a name after a single `:` or a `&`, or alone between
brackets, as neither a value nor a type. In `x: c_char`, `&mut c_char` and
`&[c_char]` the name is a type, and in `Foo { f: name }`, `&name` and `[name]`
it is a value. A function that the file only sets as a struct literal field,
borrows or puts in an array passes this check.

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
from enum import Enum, auto
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
from qa._rust import mask_comments_and_strings

CHECK = "check-function-imports"

_USE_PREFIX = re.compile(r"^\s*(?:pub(?:\s*\([^)]*\))?\s+)?use\s+")

# A `snake_case` leaf: a function, a module, or one of the few lowercase types.
_SNAKE_CASE = re.compile(r"[a-z][a-z0-9_]*")

# Path keywords that refer to no item, and the `_` alias of a trait import.
_NOT_AN_ITEM = frozenset({"self", "super", "crate", "_"})

# `fn name`, `mod name` and `macro_rules! name` declare the name, and `#[name]`
# invokes an attribute macro. A `mod tests` that imports a function from `super`
# sits in the file that defines it.
_DECLARATION_OR_ATTRIBUTE_BEFORE = re.compile(r"(?:\b(?:fn|mod)\s+|\bmacro_rules!\s*|#\[)$")

# A type follows `<` in a generic argument, `*const` and `*mut` in a raw pointer,
# `as` in a cast, and `->` in a return type.
_TYPE_BEFORE = re.compile(r"(?:<|\*const\s+|\*mut\s+|\bas\s+|->\s*)$")
_OPENING_BRACKET_BEFORE = re.compile(r"\[\s*$")
_SINGLE_COLON_OR_REFERENCE_BEFORE = re.compile(r"(?:(?<!:):|&(?:'\w+\s+)?(?:mut\s+)?)\s*$")

# Room for the longest match of the patterns above, `macro_rules! ` or a
# reference with a lifetime such as `&'static mut `, and the whitespace a
# hand-formatted line holds.
_BEFORE_WINDOW = 32

_CALL_AFTER = re.compile(r"\s*(?:\(|::\s*<)")
_MODULE_PATH_OR_MACRO_AFTER = re.compile(r"\s*(?:::|!(?!=))")
_ARRAY_LENGTH_AFTER = re.compile(r"\s*;")
_CLOSING_BRACKET_AFTER = re.compile(r"\s*\]")

_NON_WHITESPACE = re.compile(r"\S")


class _OccurrenceKind(Enum):
    BARE_VALUE = auto()
    CALL = auto()
    DECLARATION_OR_ATTRIBUTE = auto()
    MODULE_MACRO_OR_TYPE = auto()
    TYPE_OR_VALUE = auto()


def _occurrence_kind(code: str, found: re.Match[str]) -> _OccurrenceKind:
    before = code[max(0, found.start() - _BEFORE_WINDOW) : found.start()]
    if _DECLARATION_OR_ATTRIBUTE_BEFORE.search(before) is not None:
        return _OccurrenceKind.DECLARATION_OR_ATTRIBUTE
    if _CALL_AFTER.match(code, found.end()) is not None:
        return _OccurrenceKind.CALL
    if (
        _MODULE_PATH_OR_MACRO_AFTER.match(code, found.end()) is not None
        or _TYPE_BEFORE.search(before) is not None
    ):
        return _OccurrenceKind.MODULE_MACRO_OR_TYPE
    if _OPENING_BRACKET_BEFORE.search(before) is not None:
        if _ARRAY_LENGTH_AFTER.match(code, found.end()) is not None:
            return _OccurrenceKind.MODULE_MACRO_OR_TYPE
        if _CLOSING_BRACKET_AFTER.match(code, found.end()) is not None:
            return _OccurrenceKind.TYPE_OR_VALUE
    if _SINGLE_COLON_OR_REFERENCE_BEFORE.search(before) is not None:
        return _OccurrenceKind.TYPE_OR_VALUE
    return _OccurrenceKind.BARE_VALUE


def _is_used_as_a_function(code: str, name: str) -> bool:
    """Whether `code` calls `name`, or uses it as a bare value and as no module, macro or type."""
    unqualified = re.compile(rf"(?<![\w.:]){re.escape(name)}(?!\w)")
    occurrences = {_occurrence_kind(code, found) for found in unqualified.finditer(code)}
    return _OccurrenceKind.CALL in occurrences or (
        _OccurrenceKind.BARE_VALUE in occurrences
        and _OccurrenceKind.MODULE_MACRO_OR_TYPE not in occurrences
    )


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
    statements = list(use_statements(code.splitlines()))
    code_outside_use_statements = _blank_use_statements(code, statements)
    lines = source.splitlines()
    for start, statement in statements:
        end = start + statement.count("\n")
        if any(is_exempt(line, CHECK) for line in lines[start : end + 1]):
            continue
        for name, offset in imported_names(statement):
            if name in _NOT_AN_ITEM or _SNAKE_CASE.fullmatch(name) is None:
                continue
            if _is_used_as_a_function(code_outside_use_statements, name):
                lineno = start + statement[:offset].count("\n")
                yield path, lineno + 1, f"{name} - {lines[lineno].strip()}"


def _blank_use_statements(code: str, statements: list[tuple[int, str]]) -> str:
    """`code` with the text of every `use` statement blanked to spaces.

    Without the blanking, this check reads the leaf of an import
    (`use m::{a, name};`, `use x as name;`) as a bare use of the name it imports.
    """
    lines = code.splitlines(keepends=True)
    for start, statement in statements:
        for index in range(start, start + statement.count("\n") + 1):
            lines[index] = _NON_WHITESPACE.sub(" ", lines[index])
    return "".join(lines)


def _collect(root: Path) -> list[Violation]:
    return [violation for path in rs_files(root) for violation in _violations_in(path)]


_NOTE = [
    "this file calls the name or passes it as a value with no module before it, and",
    "the import hides where the function comes from.",
    "CODE_STYLE keeps at least the parent module on a function.",
]
_HELP = [
    "import the parent module and qualify each use (`use std::fs;` then",
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

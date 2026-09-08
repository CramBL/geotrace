"""Forbid a module written inline with a body, in Rust source.

CODE_STYLE states that a module is a file. A reader searching the file names
finds every module written as a file, and none written inline.

Two forms raise nothing. `mod tests { … }` at file level is the Rust idiom for a
file's own unit tests. A module with a `#[path = "…"]` attribute sets where its
children are read from, which groups several test files into one test binary.

A module nested inside `mod tests` is a violation of its own. Each test there
takes a name that says what it covers, and the tests stay in one list.

A module declared without a body (`mod parser;`) is the form this check leaves
alone.

Exemption syntax (on the `mod` line):

    mod sealed { … } // [qa-allow-check-inline-modules, reason = "why"]
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
from qa._rust import mask_comments_and_strings

CHECK = "check-inline-modules"

# The module a file's own unit tests go in, exempt at file level alone.
_TEST_MODULE = "tests"

# A `mod` declaration, and every other brace, which together give the nesting a
# `mod` sits at. The `mod` alternative consumes the brace that opens its body,
# so the scan below counts that brace once.
_DECLARATION_OR_BRACE = re.compile(
    r"(?<![\w:])(?:pub(?:\s*\([^)]*\))?\s+)?mod\s+(?P<name>\w+)\s*(?P<opens_a_body>\{|;)"
    r"|(?P<opening_brace>\{)|(?P<closing_brace>\})"
)

_PATH_ATTRIBUTE = re.compile(r"#\[\s*path\s*=")


def attributes_before(code: str, start: int) -> str:
    """The attributes written directly above the item at `start`, as one text.

    `code` is masked source, so a bracket inside a string pairs with nothing and
    an attribute holding one ends where it is written.
    """
    item = start
    while item > 0:
        cursor = item
        while cursor > 0 and code[cursor - 1].isspace():
            cursor -= 1
        if cursor == 0 or code[cursor - 1] != "]":
            break
        depth = 0
        while cursor > 0:
            cursor -= 1
            if code[cursor] == "]":
                depth += 1
            elif code[cursor] == "[":
                depth -= 1
                if depth == 0:
                    break
        if depth != 0 or cursor == 0 or code[cursor - 1] != "#":
            break
        item = cursor - 1
    return code[item:start]


def inline_modules(code: str) -> Iterator[tuple[int, str]]:
    """Every module `code` declares with a body, as its offset and its name.

    A `mod tests` at file level is left out, and a module nested inside it is
    not. The nesting comes from the braces the scan holds open, one entry per
    brace, holding the module name where a module opened it.
    """
    open_modules: list[str | None] = []
    for found in _DECLARATION_OR_BRACE.finditer(code):
        if found.group("closing_brace") is not None:
            if open_modules:
                open_modules.pop()
        elif found.group("opening_brace") is not None:
            open_modules.append(None)
        elif found.group("opens_a_body") == "{":
            name = found.group("name")
            if open_modules or name != _TEST_MODULE:
                yield found.start(), name
            open_modules.append(name)


def _violations_in(path: Path) -> Iterator[Violation]:
    source = path.read_text(errors="replace")
    code = mask_comments_and_strings(source)
    lines = source.splitlines()
    for offset, name in inline_modules(code):
        if _PATH_ATTRIBUTE.search(attributes_before(code, offset)) is not None:
            continue
        lineno = code[:offset].count("\n")
        line = lines[lineno] if lineno < len(lines) else ""
        if is_exempt(line, CHECK):
            continue
        yield path, lineno + 1, f"{name} - {line.strip()}"


def _collect(root: Path) -> list[Violation]:
    return [violation for path in rs_files(root) for violation in _violations_in(path)]


_NOTE = [
    "CODE_STYLE states that a module is a file, so that a reader finds any module",
    "by searching file names.",
    "`mod tests` at file level is the exception, and a module inside it is not.",
]
_HELP = [
    "move the module into `<name>.rs` and declare it as `mod <name>;`, or, inside",
    "`mod tests`, give the tests a name that says what they cover, or exempt with:",
]

DEFINITION = Check(
    name=CHECK,
    title="an inline module found",
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

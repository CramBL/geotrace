"""Forbid `short`, `long` and `long long` in the C sources of the SDK.

A value holds the same range on every target: every field, parameter and local
of the C SDK is a fixed-width type from `<stdint.h>`. A `long` is 32 bits on a
32-bit target, where the gold timestamp parser's day count times 86400
overflows for a date past 2038.

This check reads the `.c` and `.h` files. clang-tidy's `google-runtime-int`
covers the same declarations in a C++ translation unit.

Exemption syntax (same line):

    long value = strtol(text, &end, 10); // [qa-allow-check-fixed-width-integers, reason = "why"]
"""

import re
import sys
from pathlib import Path

from qa._allow import is_exempt
from qa._check import Check, Violation, c_family_files, repo_root, run_check

CHECK = "check-fixed-width-integers"

_C_SUFFIXES = frozenset({".c", ".h"})

# In code, `long` and `short` as whole words are the type keywords. C reserves
# both, and no identifier can be named `long` or `short`.
_WIDTH_VARYING_TYPE = re.compile(r"\b(?:long|short)\b")

# Comments and string and character literals in one alternation. The leftmost
# match wins, which keeps a quote inside a comment and a `//` inside a literal
# part of the construct they sit in.
_NOT_CODE = re.compile(r"/\*.*?\*/|//[^\n]*|'(?:\\.|[^'\\])*'|\"(?:\\.|[^\"\\])*\"", re.DOTALL)


def _code_only(text: str) -> str:
    """`text` with every comment and literal replaced by the newlines it spans,
    which leaves the code on the line numbers it was written at."""
    return _NOT_CODE.sub(lambda match: "\n" * match.group().count("\n"), text)


def _collect(root: Path) -> list[Violation]:
    violations: list[Violation] = []
    for path in c_family_files(root):
        if path.suffix not in _C_SUFFIXES:
            continue
        text = path.read_text(errors="replace")
        lines = zip(text.splitlines(), _code_only(text).splitlines(), strict=True)
        for i, (raw, code) in enumerate(lines):
            if _WIDTH_VARYING_TYPE.search(code) and not is_exempt(raw, CHECK):
                violations.append((path, i + 1, raw.strip()))
    return violations


_NOTE = [
    "a sum that fits on one platform can overflow on another:",
    "`short`, `long` and `long long` are 16, 32 or 64 bits depending on the target",
]
_HELP = [
    "declare a fixed-width type from <stdint.h> (int32_t, int64_t, uint32_t),",
    "or exempt with:",
]

DEFINITION = Check(
    name=CHECK,
    title="integer type of a width that varies per platform found",
    collect=_collect,
    note=_NOTE,
    help=_HELP,
)


def main() -> None:
    if run_check(DEFINITION, repo_root()):
        sys.exit(1)

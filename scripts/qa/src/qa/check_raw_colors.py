"""Forbid egui's named chromatic colour constants in production Rust.

`Color32::YELLOW`, `Color32::GREEN`, and friends are pure, maximally saturated
primaries. They read on a dark surface but wash out or vanish on a light one,
which is how a string of light-mode-only legibility bugs slipped through. Every
semantic foreground colour belongs in `gt-ui-theme` as a `ThemedColor` (with a
light and a dark variant, contrast-checked by that crate's tests) rather than a
raw primary grabbed at the call site.

The un-themed `gt-ui-theme` foreground constants that now have theme-aware
accessors (`WARNING_AMBER` → `warning_amber(dark_mode)`, `ERROR_INDICATOR` →
`error_indicator(dark_mode)`) are banned in the same pass: using the raw
constant hard-codes the dark variant onto whichever theme is active, which is
the exact regression this fix removed.

Only the chromatic constants and those accessor-backed foregrounds are banned.
Theme-neutral colours (`WHITE`, `BLACK`, `GRAY`, `TRANSPARENT`), deliberate
tuned `from_rgb(...)` values, and anything inside a `#[cfg(test)]` block (where
primaries are opaque sentinels, not rendered colours) are left alone.

Exemption syntax (same line):
    let c = Color32::RED; // [qa-allow-check-raw-colors, reason = "why"]
"""

import re
import sys
from collections.abc import Iterator
from pathlib import Path

from qa._allow import is_exempt
from qa._check import Check, Violation, repo_root, rs_files, run_check

CHECK = "check-raw-colors"

# The palette crate is where themed colours are defined, so it is exempt.
_PALETTE_CRATE = "crates/gt-ui-theme/"

# Named chromatic Color32 constants. Neutral hues (WHITE/BLACK/GRAY/TRANSPARENT)
# and constructors (from_rgb/from_gray) are intentionally absent.
_CHROMATIC = (
    "RED",
    "GREEN",
    "BLUE",
    "YELLOW",
    "GOLD",
    "BROWN",
    "ORANGE",
    "KHAKI",
    "PURPLE",
    "MAGENTA",
    "CYAN",
    "LIGHT_RED",
    "LIGHT_GREEN",
    "LIGHT_BLUE",
    "LIGHT_YELLOW",
    "DARK_RED",
    "DARK_GREEN",
    "DARK_BLUE",
)
_RAW_COLOR = re.compile(rf"Color32::(?:{'|'.join(_CHROMATIC)})\b")

# gt-ui-theme foreground constants that now have theme-aware accessors; using
# the bare constant elsewhere pins the dark variant onto every theme. The word
# boundary keeps `WARNING_AMBER` from also matching `WARNING_AMBER_LIGHT`.
_UNTHEMED_CONST = re.compile(r"\b(?:WARNING_AMBER|ERROR_INDICATOR)\b")

_CFG_TEST = re.compile(r"#\[cfg\(test\)\]")


def _non_test_lines(lines: list[str]) -> Iterator[tuple[int, str]]:
    """Yield `(index, line)` for lines outside any `#[cfg(test)]` block.

    Tracks brace depth so a `#[cfg(test)]`-attributed module or function is
    skipped in full, wherever it sits in the file, and normal code after it is
    still scanned.
    """
    depth = 0
    pending_test = False
    test_depth: int | None = None
    for i, line in enumerate(lines):
        if test_depth is None and not pending_test and _CFG_TEST.search(line):
            pending_test = True
        opens = line.count("{")
        closes = line.count("}")
        if test_depth is None:
            yield i, line
        if pending_test and opens > 0:
            test_depth = depth
            pending_test = False
        depth += opens - closes
        if test_depth is not None and depth <= test_depth:
            test_depth = None


def _collect(root: Path) -> list[Violation]:
    violations: list[Violation] = []
    for path in rs_files(root):
        if _PALETTE_CRATE in path.as_posix():
            continue
        lines = path.read_text(errors="replace").splitlines()
        for i, raw in _non_test_lines(lines):
            hit = _RAW_COLOR.search(raw) or _UNTHEMED_CONST.search(raw)
            if hit and not is_exempt(raw, CHECK):
                violations.append((path, i + 1, raw.strip()))
    return violations


_NOTE = [
    "chromatic Color32 primaries and the un-themed WARNING_AMBER/ERROR_INDICATOR",
    "constants read on dark surfaces but wash out on light - the recurring light-mode bug",
]
_HELP = [
    "use a gt-ui-theme accessor (warning_amber/error_indicator) or add a ThemedColor,",
    "or exempt with:",
]

DEFINITION = Check(
    name=CHECK,
    title="raw theme-blind colour constants found",
    collect=_collect,
    note=_NOTE,
    help=_HELP,
)


def main() -> None:
    if run_check(DEFINITION, repo_root()):
        sys.exit(1)

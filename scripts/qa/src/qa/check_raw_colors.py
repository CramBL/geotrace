"""Forbid egui's named chromatic colour constants in production Rust.

`Color32::YELLOW`, `Color32::GREEN`, and friends are pure, maximally saturated
primaries. They read on a dark surface but wash out or vanish on a light one,
which is how a string of light-mode-only legibility bugs slipped through. Every
semantic foreground colour belongs in `gt-ui-theme` as a `ThemedColor` (with a
light and a dark variant, contrast-checked by that crate's tests).

The un-themed `gt-ui-theme` foreground constants that now have theme-aware
accessors (`WARNING_AMBER` → `warning_amber(dark_mode)`, `ERROR_INDICATOR` →
`error_indicator(dark_mode)`) are banned in the same pass: using the raw
constant hard-codes the dark variant onto whichever theme is active, which is
the exact regression this fix removed.

Only the chromatic constants and those accessor-backed foregrounds are banned.
Theme-neutral colours (`WHITE`, `BLACK`, `GRAY`, `TRANSPARENT`), deliberate
tuned `from_rgb(...)` values, and anything inside a `#[cfg(test)]` block or a
test-only module file (where primaries are opaque sentinels, not rendered
colours) are left alone.

Exemption syntax (same line):

    let c = Color32::RED; // [qa-allow-check-raw-colors, reason = "why"]
"""

import re
import sys
from pathlib import Path

from qa._allow import is_exempt
from qa._check import Check, Violation, is_test_only_module, repo_root, rs_files, run_check
from qa._rust import cfg_test_line_numbers

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

# gt-ui-theme foreground constants that now have theme-aware accessors. Using
# the bare constant elsewhere pins the dark variant onto every theme. The word
# boundary keeps `WARNING_AMBER` from also matching `WARNING_AMBER_LIGHT`.
_UNTHEMED_CONST = re.compile(r"\b(?:WARNING_AMBER|ERROR_INDICATOR)\b")


def _collect(root: Path) -> list[Violation]:
    violations: list[Violation] = []
    for path in rs_files(root):
        if _PALETTE_CRATE in path.as_posix() or is_test_only_module(path):
            continue
        source = path.read_text(errors="replace")
        test_lines = cfg_test_line_numbers(source)
        for lineno, raw in enumerate(source.splitlines(), 1):
            if lineno in test_lines:
                continue
            hit = _RAW_COLOR.search(raw) or _UNTHEMED_CONST.search(raw)
            if hit and not is_exempt(raw, CHECK):
                violations.append((path, lineno, raw.strip()))
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

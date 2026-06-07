"""Forbid numbered narrative comments in Rust and C/C++ source files.

Narrative comments are comments that start with a number and a dot (e.g. "// 1. Load
the map", "// 2. Filter results"), forming a step-by-step story woven through code.
They are a symptom of code that is too complex to be read without an accompanying
narration, and they create a maintenance burden because the numbering must be kept
in sync with the code they describe.  The fix is to extract the steps into small,
well-named functions that are self-documenting.

Exemption syntax (same line as the comment):
    // 1. Special case // [qa-allow-check-narrative-comments, reason = "why"]
"""

import re
from pathlib import Path

from qa._allow import is_exempt
from qa._check import Violation, c_family_files, repo_root, rs_files, run_check

CHECK = "check-narrative-comments"

# Matches a comment that begins (after optional whitespace and the comment marker)
# with a decimal number immediately followed by a dot and a space/end-of-text.
# Examples that match:   // 1. Do this    /* 2. Then that */    // 12. Finally
# Examples that do not:  // v1.0          // PI = 3.14...       // e.g. 1.5x faster
_RS = re.compile(r"^\s*//[^/!].*?(\d+)\.\s")
_C_LINE = re.compile(r"^\s*//[^/].*?(\d+)\.\s")
_C_BLOCK = re.compile(r"^\s*/\*[^*].*?(\d+)\.\s.*\*/\s*$")

# More precise anchored patterns: the number+dot must appear right after the
# comment marker (possibly after whitespace), not deep inside the comment text.
_RS_ANCHORED = re.compile(r"^\s*//[^/!]\s*\d+\.\s")
_C_LINE_ANCHORED = re.compile(r"^\s*//[^/]\s*\d+\.\s")
_C_BLOCK_ANCHORED = re.compile(r"^\s*/\*[^*]\s*\d+\.\s.*\*/\s*$")

_NOTE = [
    "numbered narrative comments narrate code that is too complex to be self-explanatory;",
    "they couple the comment ordering to the code and break silently when either changes",
]
_HELP = [
    "extract each numbered step into a small, well-named function instead, or exempt with:",
]


def _collect(root: Path) -> list[Violation]:
    violations: list[Violation] = []

    for path in rs_files(root):
        lines = path.read_text(errors="replace").splitlines()
        for i, raw in enumerate(lines):
            if _RS_ANCHORED.match(raw) and not is_exempt(raw, CHECK):
                violations.append((path, i + 1, raw.strip()))

    for path in c_family_files(root):
        lines = path.read_text(errors="replace").splitlines()
        for i, raw in enumerate(lines):
            if (_C_LINE_ANCHORED.match(raw) or _C_BLOCK_ANCHORED.match(raw)) and not is_exempt(
                raw, CHECK
            ):
                violations.append((path, i + 1, raw.strip()))

    return violations


def main() -> None:
    run_check(CHECK, "numbered narrative comments found", _collect(repo_root()), _NOTE, _HELP)

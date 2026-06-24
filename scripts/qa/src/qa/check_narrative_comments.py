"""Forbid numbered narrative comments (numbered lists or labelled phases) in source files.

Exemption syntax (same line as the comment):
    // 1. Special case // [qa-allow-check-narrative-comments, reason = "why"]
    # Phase 1 setup    # [qa-allow-check-narrative-comments, reason = "why"]
"""

import re
import sys
from itertools import chain
from pathlib import Path

from qa._allow import is_exempt
from qa._check import (
    Check,
    Violation,
    c_family_files,
    hash_comment_files,
    repo_root,
    rs_files,
    run_check,
    yaml_files,
)

CHECK = "check-narrative-comments"

# A narrative marker: either a numbered-list item ("1. ", "12. ") or a
# labelled phase ("Phase 1", "phase 12"). The marker must appear right after
# the comment opener (possibly after whitespace), not deep inside the
# comment text.
# Examples that match:    // 1. Do this    /* 2. Then that */    // Phase 3
# Examples that do not:   // v1.0          // PI = 3.14...       // e.g. 1.5x faster
_MARKER = r"(?:\d+\.\s|[Pp]hase\s+\d+\b)"

_RS_ANCHORED = re.compile(rf"^\s*//[^/!]\s*{_MARKER}")
_C_LINE_ANCHORED = re.compile(rf"^\s*//[^/]\s*{_MARKER}")
_C_BLOCK_ANCHORED = re.compile(rf"^\s*/\*[^*]\s*{_MARKER}.*\*/\s*$")
_HASH_ANCHORED = re.compile(rf"^\s*#\s*{_MARKER}")

_NOTE = [
    "numbered/phased narrative comments narrate code that is too complex to be self-explanatory;",
    "they couple the comment ordering to the code and break silently when either changes",
]
_HELP = [
    "extract each step into a small, well-named function instead, or exempt with:",
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

    for path in chain(hash_comment_files(root), yaml_files(root)):
        lines = path.read_text(errors="replace").splitlines()
        for i, raw in enumerate(lines):
            if _HASH_ANCHORED.match(raw) and not is_exempt(raw, CHECK):
                violations.append((path, i + 1, raw.strip()))

    return violations


DEFINITION = Check(
    name=CHECK,
    title="numbered narrative comments found",
    collect=_collect,
    note=_NOTE,
    help=_HELP,
)


def main() -> None:
    if run_check(DEFINITION, repo_root()):
        sys.exit(1)

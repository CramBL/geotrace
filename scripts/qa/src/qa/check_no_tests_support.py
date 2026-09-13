"""Forbid a Rust file under a `tests/support/` directory.

CODE_STYLE puts the helpers a crate's tests share in the crate's `test_util`
module, and those of a published crate in a test-only crate. A module of test
helpers is never named `support`.

Exemption syntax (on the first line of the file):

    //! Fixtures for … // [qa-allow-check-no-tests-support, reason = "why"]
"""

import sys
from pathlib import Path

from qa._allow import is_exempt
from qa._check import Check, Violation, repo_root, rs_files, run_check

CHECK = "check-no-tests-support"

_TESTS_SUPPORT_DIRECTORY = "/tests/support/"


def _collect(root: Path) -> list[Violation]:
    violations: list[Violation] = []
    for path in rs_files(root):
        if _TESTS_SUPPORT_DIRECTORY not in f"/{path.relative_to(root).as_posix()}":
            continue
        first_line = next(iter(path.read_text(errors="replace").splitlines()), "")
        if is_exempt(first_line, CHECK):
            continue
        violations.append((path, 1, first_line.strip()))
    return violations


_NOTE = [
    "CODE_STYLE puts the helpers a crate's tests share in the crate's `test_util` module,",
    "and those of a published crate in a test-only crate.",
]
_HELP = [
    "move the helpers into `src/test_util.rs` behind the crate's `test-util` feature and",
    "call them as `<crate>::test_util::…`, or exempt the file on its first line with:",
]

DEFINITION = Check(
    name=CHECK,
    title="a test helper module found under `tests/support/`",
    collect=_collect,
    note=_NOTE,
    help=_HELP,
)


def main() -> None:
    if run_check(DEFINITION, repo_root()):
        sys.exit(1)

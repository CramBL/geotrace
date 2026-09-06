"""Run all QA checks against a repository in a single pass."""

import argparse
import sys
from pathlib import Path

from qa import (
    check_fixed_width_integers,
    check_floating_comments,
    check_narrative_comments,
    check_no_network,
    check_raw_colors,
)
from qa._check import repo_root, run_check

_CHECKS = [
    check_fixed_width_integers.DEFINITION,
    check_floating_comments.DEFINITION,
    check_narrative_comments.DEFINITION,
    check_no_network.DEFINITION,
    check_raw_colors.DEFINITION,
]


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--repo-root",
        type=Path,
        default=repo_root(),
        help="repository root to scan (default: this checkout's root)",
    )
    args = parser.parse_args()

    root: Path = args.repo_root.resolve()
    failed = False
    for check in _CHECKS:
        if run_check(check, root):
            failed = True

    if failed:
        sys.exit(1)

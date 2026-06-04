"""Run ruff and mypy over the QA project source."""

import subprocess
import sys
from pathlib import Path


def main() -> None:
    src = Path(__file__).parent.parent
    failed = False

    result = subprocess.run(["ruff", "check", str(src)], check=False)
    if result.returncode != 0:
        failed = True

    result = subprocess.run(
        ["mypy", "--strict", str(src)],
        check=False,
    )
    if result.returncode != 0:
        failed = True

    if failed:
        sys.exit(1)

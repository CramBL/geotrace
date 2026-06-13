"""Shared infrastructure for QA checks: file iteration, reporting, and exit logic."""

import subprocess
import sys
from collections.abc import Iterator
from functools import lru_cache
from pathlib import Path

Violation = tuple[Path, int, str]


def repo_root() -> Path:
    """Repository root, resolved from this file's location rather than the
    current working directory.

    `just` runs recipes from imported submodules (such as this `qa` module)
    with the submodule's directory as the working directory, not the repo
    root or the invocation directory — so checks that scanned `Path(".")`
    silently scanned `scripts/qa` instead of the repo, finding nothing.
    """
    return Path(__file__).resolve().parent.parent.parent.parent.parent


@lru_cache(maxsize=1)
def _tracked_files() -> frozenset[Path]:
    """Absolute paths of files git does not ignore: tracked files plus
    untracked files that aren't matched by `.gitignore`.

    Keeps the checks below from scanning build artifacts, virtualenvs, and
    other generated files that happen to match a checked file pattern.
    """
    root = repo_root()
    output = subprocess.run(
        ["git", "ls-files", "--cached", "--others", "--exclude-standard", "-z"],
        cwd=root,
        capture_output=True,
        check=True,
        text=True,
    ).stdout
    return frozenset(root / p for p in output.split("\0") if p)


def _is_excluded(path: Path) -> bool:
    return path not in _tracked_files()


def rs_files(root: Path) -> Iterator[Path]:
    for path in sorted(root.rglob("*.rs")):
        if not _is_excluded(path):
            yield path


def hash_comment_files(root: Path) -> Iterator[Path]:
    """Yield Python, Just, and CMake source files (uses # comments)."""
    seen: set[Path] = set()
    for pattern in ("*.py", "*.just", "justfile", "CMakeLists.txt", "*.cmake"):
        for path in root.rglob(pattern):
            if not _is_excluded(path) and path not in seen:
                seen.add(path)
    yield from sorted(seen)


def c_family_files(root: Path) -> Iterator[Path]:
    """Yield C and C++ source and header files."""
    seen: set[Path] = set()
    for pattern in ("*.c", "*.h", "*.cpp", "*.hpp"):
        for path in root.rglob(pattern):
            if not _is_excluded(path) and path not in seen:
                seen.add(path)
    yield from sorted(seen)


def yaml_files(root: Path) -> Iterator[Path]:
    """Yield YAML files (workflow definitions, configuration)."""
    seen: set[Path] = set()
    for pattern in ("*.yml", "*.yaml"):
        for path in root.rglob(pattern):
            if not _is_excluded(path) and path not in seen:
                seen.add(path)
    yield from sorted(seen)


def _labelled_lines(tag: str, lines: list[str]) -> None:
    for i, text in enumerate(lines):
        prefix = f"  = {tag}: " if i == 0 else "          "
        print(f"{prefix}{text}")


def report(
    check: str,
    title: str,
    violations: list[Violation],
    note: list[str],
    help: list[str],
) -> None:
    print(f"error[{check}]: {title}\n")
    for path, lineno, line in violations:
        print(f"  --> {path}:{lineno}")
        print("   |")
        print(f"   |  {line}")
        print("   |")
    _labelled_lines("note", note)
    _labelled_lines("help", help + [f'// [qa-allow-{check}, reason = "why this is acceptable"]'])


def run_check(
    check: str,
    title: str,
    violations: list[Violation],
    note: list[str],
    help: list[str],
) -> None:
    if violations:
        report(check, title, violations, note, help)
        sys.exit(1)

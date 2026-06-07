"""Shared infrastructure for QA checks: file iteration, reporting, and exit logic."""

import sys
from collections.abc import Iterator
from pathlib import Path

Violation = tuple[Path, int, str]


_EXCLUDED = frozenset({"target", ".venv", "build"})


def repo_root() -> Path:
    """Repository root, resolved from this file's location rather than the
    current working directory.

    `just` runs recipes from imported submodules (such as this `qa` module)
    with the submodule's directory as the working directory, not the repo
    root or the invocation directory — so checks that scanned `Path(".")`
    silently scanned `scripts/qa` instead of the repo, finding nothing.
    """
    return Path(__file__).resolve().parent.parent.parent.parent.parent


def _is_excluded(path: Path) -> bool:
    return any(part in _EXCLUDED for part in path.parts)


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

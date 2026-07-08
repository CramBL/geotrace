"""Tests for `qa.changelog`: promoting a `## [unreleased]` section to a release."""

from datetime import date
from pathlib import Path

import pytest

from qa import changelog

_TODAY = date(2026, 6, 24)

_WITH_NOTES = """\
# Changelog

## [unreleased]

### Added

- A shiny new thing.

### Fixed

- An embarrassing old thing.

## [0.1.0] - 2026-06-21

Initial release
"""

_EMPTY_UNRELEASED = """\
# Changelog

## [unreleased]

## [0.1.0] - 2026-06-21

Initial release
"""


def _write(tmp_path: Path, text: str) -> Path:
    path = tmp_path / "CHANGELOG.md"
    path.write_text(text, encoding="utf-8")
    return path


def _section_lines(text: str) -> list[str]:
    return [line for line in text.splitlines() if line.startswith("## ")]


def test_promote_moves_notes_into_dated_section(tmp_path: Path) -> None:
    path = _write(tmp_path, _WITH_NOTES)

    assert changelog.promote(path, "0.2.0", _TODAY) is True

    out = path.read_text(encoding="utf-8")
    assert _section_lines(out) == [
        "## [unreleased]",
        "## [0.2.0] - 2026-06-24",
        "## [0.1.0] - 2026-06-21",
    ]
    assert "## [0.2.0] - 2026-06-24\n\n### Added\n\n- A shiny new thing." in out
    assert out.index("- A shiny new thing.") < out.index("## [0.1.0]")
    between = out.split("## [unreleased]", 1)[1].split("## [0.2.0]", 1)[0]
    assert between.strip() == ""


def test_promote_is_idempotent(tmp_path: Path) -> None:
    path = _write(tmp_path, _WITH_NOTES)

    assert changelog.promote(path, "0.2.0", _TODAY) is True
    once = path.read_text(encoding="utf-8")
    # A later-dated re-run must not change the file or re-date the section.
    assert changelog.promote(path, "0.2.0", date(2026, 12, 31)) is False
    assert path.read_text(encoding="utf-8") == once


def test_prerelease_promotes_the_core_version(tmp_path: Path) -> None:
    path = _write(tmp_path, _WITH_NOTES)

    assert changelog.promote(path, "0.2.0-rc.1", _TODAY) is True

    out = path.read_text(encoding="utf-8")
    assert "## [0.2.0] - 2026-06-24" in out
    assert "rc" not in out
    # Final release of the same core version is a no-op.
    assert changelog.promote(path, "0.2.0", _TODAY) is False


def test_promote_empty_unreleased_creates_an_empty_section(tmp_path: Path) -> None:
    path = _write(tmp_path, _EMPTY_UNRELEASED)

    assert changelog.promote(path, "0.2.0", _TODAY) is True

    out = path.read_text(encoding="utf-8")
    assert _section_lines(out) == [
        "## [unreleased]",
        "## [0.2.0] - 2026-06-24",
        "## [0.1.0] - 2026-06-21",
    ]
    between = out.split("## [0.2.0] - 2026-06-24", 1)[1].split("## [0.1.0]", 1)[0]
    assert between.strip() == ""


def test_promote_is_case_insensitive_about_unreleased(tmp_path: Path) -> None:
    path = _write(tmp_path, _WITH_NOTES.replace("[unreleased]", "[Unreleased]"))

    assert changelog.promote(path, "0.2.0", _TODAY) is True

    out = path.read_text(encoding="utf-8")
    assert "## [unreleased]" in out
    assert "## [0.2.0] - 2026-06-24" in out


def test_promote_can_write_cargo_dist_headings(tmp_path: Path) -> None:
    path = _write(tmp_path, _WITH_NOTES)

    assert changelog.promote(path, "0.2.0", _TODAY, heading_style="cargo_dist") is True

    out = path.read_text(encoding="utf-8")
    assert _section_lines(out) == [
        "## Unreleased",
        "## 0.2.0 - 2026-06-24",
        "## [0.1.0] - 2026-06-21",
    ]
    assert "## 0.2.0 - 2026-06-24\n\n### Added\n\n- A shiny new thing." in out


def test_promote_without_unreleased_raises(tmp_path: Path) -> None:
    path = _write(tmp_path, "# Changelog\n\n## [0.1.0] - 2026-06-21\n\nInitial release\n")

    with pytest.raises(SystemExit):
        changelog.promote(path, "0.2.0", _TODAY)


def test_promote_ends_with_a_single_trailing_newline(tmp_path: Path) -> None:
    path = _write(tmp_path, _WITH_NOTES)

    changelog.promote(path, "0.2.0", _TODAY)

    out = path.read_text(encoding="utf-8")
    assert out.endswith("\n")
    assert not out.endswith("\n\n")


@pytest.mark.parametrize(
    ("version", "expected"),
    [("0.1.0", True), ("0.2.0", False), ("0.1.0-rc.1", True), ("0.1.0+build.5", True)],
)
def test_section_exists_matches_on_core_version(version: str, expected: bool) -> None:
    assert changelog.section_exists(_WITH_NOTES, version) is expected


def test_section_exists_matches_cargo_dist_headings() -> None:
    text = """\
# Changelog

## Unreleased

## 0.1.0 - 2026-06-21

Initial release
"""
    assert changelog.section_exists(text, "0.1.0")


def test_section_exists_ignores_the_unreleased_header() -> None:
    assert changelog.section_exists("## [unreleased]\n", "0.1.0") is False

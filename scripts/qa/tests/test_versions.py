"""Tests for release version spot checks and bumps."""

from pathlib import Path

from qa import versions

_APP_CRATES = versions._APP_LOCK_CRATES


def _write_app_release_files(tmp_path: Path, manifest_version: str, lock_version: str) -> None:
    tmp_path.joinpath("Cargo.toml").write_text(
        f"""\
[package]
name = "geotrace"
version.workspace = true

[workspace.package]
version = "{manifest_version}"
""",
        encoding="utf-8",
    )
    tmp_path.joinpath("Cargo.lock").write_text(
        "\n".join(
            f"""\
[[package]]
name = "{crate}"
version = "{lock_version}"
"""
            for crate in _APP_CRATES
        ),
        encoding="utf-8",
    )
    tmp_path.joinpath("CHANGELOG.md").write_text(
        """\
# Changelog

## Unreleased

## 0.2.0 - 2026-06-24
""",
        encoding="utf-8",
    )


def test_check_app_fails_when_lockfile_versions_drift(tmp_path: Path) -> None:
    _write_app_release_files(tmp_path, "0.2.0", "0.1.0")

    assert versions._cmd_check_app(tmp_path, "0.2.0") == 1


def test_bump_app_updates_root_lockfile_versions(tmp_path: Path) -> None:
    _write_app_release_files(tmp_path, "0.1.0", "0.1.0")

    versions._apply(tmp_path, versions._APP_SPOTS, "0.2.0", "0.2.0")

    assert versions._cmd_check_app(tmp_path, "0.2.0") == 0
    lock_text = tmp_path.joinpath("Cargo.lock").read_text(encoding="utf-8")
    assert 'version = "0.1.0"' not in lock_text
    assert lock_text.count('version = "0.2.0"') == len(_APP_CRATES)

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


def _write_publish_fixture(tmp_path: Path, publish: list[str], deps: dict[str, list[str]]) -> None:
    """A minimal repo shape for the publish-closure check: a release workflow
    publishing `publish` in order, and one manifest per crate with the path
    dependencies `deps` names. Every crate is registered as a version spot."""
    workflow = tmp_path / versions._RELEASE_WORKFLOW
    workflow.parent.mkdir(parents=True, exist_ok=True)
    workflow.write_text(
        "".join(f"          publish {crate}\n" for crate in publish),
        encoding="utf-8",
    )
    for crate, crate_deps in deps.items():
        manifest = tmp_path / f"sdk/rust/{crate}/Cargo.toml"
        manifest.parent.mkdir(parents=True, exist_ok=True)
        manifest.write_text(
            f'[package]\nname = "{crate}"\nversion = "0.5.0"\n\n[dependencies]\n'
            + "".join(
                f'{dep} = {{ path = "../{dep}", version = "0.5.0" }}\n' for dep in crate_deps
            ),
            encoding="utf-8",
        )


def test_publish_closure_accepts_a_closed_ordered_list(tmp_path: Path) -> None:
    _write_publish_fixture(
        tmp_path,
        publish=["geotrace-sdk-macros", "geotrace-sdk-units", "geotrace-sdk"],
        deps={
            "geotrace-sdk-macros": [],
            "geotrace-sdk-units": [],
            "geotrace-sdk": ["geotrace-sdk-units"],
        },
    )
    assert versions.publish_closure_errors(tmp_path) == []


def test_publish_closure_rejects_an_unpublished_path_dependency(tmp_path: Path) -> None:
    """The geotrace-units release failure: a path dependency of a published
    crate that the workflow never publishes."""
    _write_publish_fixture(
        tmp_path,
        publish=["geotrace-sdk-macros", "geotrace-sdk"],
        deps={
            "geotrace-sdk-macros": [],
            "geotrace-sdk": ["geotrace-sdk-units"],
        },
    )
    errors = versions.publish_closure_errors(tmp_path)
    assert any("geotrace-sdk-units is not published" in e for e in errors), errors


def test_publish_closure_rejects_a_dependency_missing_from_version_spots(tmp_path: Path) -> None:
    """A published path dependency that bump-sdk would not keep in lockstep."""
    _write_publish_fixture(
        tmp_path,
        publish=["geotrace-not-a-spot", "geotrace-sdk"],
        deps={
            "geotrace-not-a-spot": [],
            "geotrace-sdk": ["geotrace-not-a-spot"],
        },
    )
    errors = versions.publish_closure_errors(tmp_path)
    assert any("not a version spot" in e for e in errors), errors


def test_publish_closure_rejects_a_dependency_published_too_late(tmp_path: Path) -> None:
    _write_publish_fixture(
        tmp_path,
        publish=["geotrace-sdk", "geotrace-sdk-units"],
        deps={
            "geotrace-sdk-units": [],
            "geotrace-sdk": ["geotrace-sdk-units"],
        },
    )
    errors = versions.publish_closure_errors(tmp_path)
    assert any("published after" in e for e in errors), errors


def _write_workspace(tmp_path: Path, members: dict[str, bool]) -> None:
    """A minimal workspace: each member maps to whether it inherits the
    workspace version."""
    tmp_path.joinpath("Cargo.toml").write_text(
        "[workspace]\nmembers = [\n"
        + "".join(f'    "crates/{name}",\n' for name in members)
        + "]\n",
        encoding="utf-8",
    )
    for name, inherits in members.items():
        manifest = tmp_path / "crates" / name / "Cargo.toml"
        manifest.parent.mkdir(parents=True, exist_ok=True)
        version = "version.workspace = true" if inherits else 'version = "1.0.0"'
        manifest.write_text(
            f'[package]\nname = "{name}"\n{version}\n',
            encoding="utf-8",
        )


def test_app_lock_crates_accepts_a_matching_list(tmp_path: Path) -> None:
    _write_workspace(tmp_path, {"gt-a": True, "geotrace-sdk-units": False})
    assert versions.app_lock_crate_errors(tmp_path, listed_crates=["gt-a"]) == []


def test_app_lock_crates_rejects_a_listed_member_that_stopped_inheriting(tmp_path: Path) -> None:
    """A listed crate pinned to its own version would get the wrong version."""
    _write_workspace(tmp_path, {"gt-a": True, "gt-detached": False})
    errors = versions.app_lock_crate_errors(tmp_path, listed_crates=["gt-a", "gt-detached"])
    assert any("gt-detached" in e and "no longer inherits" in e for e in errors), errors


def test_app_lock_crates_reports_a_nameless_member_manifest(tmp_path: Path) -> None:
    _write_workspace(tmp_path, {"gt-a": True})
    nameless = tmp_path / "crates" / "gt-nameless" / "Cargo.toml"
    nameless.parent.mkdir(parents=True)
    nameless.write_text('[package]\nversion.workspace = true\n', encoding="utf-8")
    root = tmp_path / "Cargo.toml"
    root.write_text(
        root.read_text(encoding="utf-8").replace(
            '\n]\n', '\n    "crates/gt-nameless",\n]\n'
        ),
        encoding="utf-8",
    )
    errors = versions.app_lock_crate_errors(tmp_path, listed_crates=["gt-a"])
    assert any("no package name found" in e for e in errors), errors


def test_app_lock_crates_rejects_an_unlisted_workspace_member(tmp_path: Path) -> None:
    """The gt-snap incident: a new workspace-versioned crate that never
    joined the bump list, so bump-app left its lock pin stale and the
    release's lockfile guard failed."""
    _write_workspace(tmp_path, {"gt-a": True, "gt-new": True})
    errors = versions.app_lock_crate_errors(tmp_path, listed_crates=["gt-a"])
    assert any("gt-new" in e and "missing from" in e for e in errors), errors


def test_bump_sdk_rewrites_the_uv_lock_package_pin_and_keeps_its_format_version(
    tmp_path: Path,
) -> None:
    lock = tmp_path / versions._PY_UV_LOCK
    lock.parent.mkdir(parents=True)
    lock.write_text(
        """\
version = 1
revision = 3
requires-python = ">=3.12"

[[package]]
name = "geotrace-sdk"
version = "0.5.1"
source = { editable = "." }
""",
        encoding="utf-8",
    )
    spot = next(s for s in versions._SDK_SPOTS if s.path == versions._PY_UV_LOCK)

    versions._apply(tmp_path, [spot], "0.6.0", "0.6.0")

    assert versions._read(tmp_path, spot).value == "0.6.0"
    assert lock.read_text(encoding="utf-8").splitlines()[0] == "version = 1"

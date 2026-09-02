"""Read, verify, and bump GeoTrace's version strings.

The GUI app and the SDK release on independent cadences, so their versions live
in different places:

- GUI: the workspace package version in the root ``Cargo.toml`` and the root
  ``Cargo.lock`` pins for packages that inherit it.
- SDK: kept in lockstep across the Rust crates (``geotrace-sdk``,
  ``geotrace-sdk-macros``, ``geotrace-sdk-units``, ``geotrace-c``), the macro
  and units dependency pins, the Python package (``Cargo.toml`` +
  ``pyproject.toml``), the C and C++ headers
  (``GEOTRACE_C_VERSION`` / ``GEOTRACE_CPP_VERSION`` and their numeric parts),
  the C and C++ CMake ``project(... VERSION)`` declarations, and the SDK-crate
  pins in both committed ``Cargo.lock`` files (the root workspace and the
  isolated Python workspace).

Most spots carry the full version (``0.2.0`` or ``0.2.0-rc.1``); CMake project
versions and the numeric ``*_MAJOR/MINOR/PATCH`` macros only hold the numeric
core (``0.2.0``), since they cannot express a prerelease suffix.

``check`` / ``check-app`` verify every spot agrees. ``bump-sdk`` / ``bump-app``
rewrite them and promote the matching changelog (see ``qa.changelog``). Given
``--expect X.Y.Z``, the checks also require a matching changelog section.
"""

import argparse
import re
import sys
import tomllib
from dataclasses import dataclass, field
from datetime import date
from pathlib import Path

from qa import changelog
from qa._check import repo_root

_SEMVER = re.compile(r"^(\d+)\.(\d+)\.(\d+)(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?$")

# Each pattern captures the kept prefix as group 1 and the version token as
# group 2, consuming nothing after it (so the closing quote / rest of line is
# preserved on rewrite).
_TOML_VERSION = re.compile(r'^(version = ")([^"]*)', re.MULTILINE)
_MACRO_PIN = re.compile(r'(geotrace-sdk-macros = \{[^}]*\bversion = ")([^"]*)')
_UNITS_PIN = re.compile(r'(geotrace-sdk-units = \{[^}]*\bversion = ")([^"]*)')


def _define_str(macro: str) -> re.Pattern[str]:
    return re.compile(rf'(#define {macro}\s+")([^"]*)')


def _define_int(macro: str, part: str) -> re.Pattern[str]:
    return re.compile(rf"(#define {macro}_{part}\s+)(\d+)")


def _cmake_project(name: str) -> re.Pattern[str]:
    return re.compile(rf"(project\({name} VERSION )(\d+\.\d+\.\d+)")


def _lock_version(crate: str) -> re.Pattern[str]:
    # Cargo writes each [[package]] block with `name` immediately followed by
    # `version`, so anchoring on the exact (quote-terminated) crate name reads
    # that block's pin. The trailing quote in the name disambiguates
    # `geotrace-sdk` from `geotrace-sdk-macros`.
    return re.compile(rf'(name = "{re.escape(crate)}"\nversion = ")([^"]*)')


@dataclass(frozen=True)
class Spot:
    """One place a version is recorded (a single capture in one file)."""

    path: str
    pattern: re.Pattern[str]
    core: bool = False  # holds the numeric core only (no prerelease suffix)
    note: str = ""

    def label(self) -> str:
        return f"{self.path}{f' ({self.note})' if self.note else ''}"


_C_HEADER = "sdk/c/geotrace.h"
_CPP_HEADER = "sdk/cpp/include/geotrace/geotrace.hpp"
# `cbindgen` writes _C_HEADER from this config, whose `after_includes` block holds
# the version macros verbatim. A bump that missed it would come back the next
# time anyone ran `just sdk-c-header`.
_C_CBINDGEN = "sdk/rust/geotrace-c/cbindgen.toml"

# The two committed Cargo.lock files that pin the SDK crates. The root lock is
# the main workspace (geotrace-c and its deps). The Python lock is an isolated
# workspace whose pins only refresh when cargo runs in its own directory, so it
# is the one that silently drifts after a bump.
_ROOT_LOCK = "Cargo.lock"
_PY_LOCK = "sdk/python/geotrace-py/Cargo.lock"

_SDK_SPOTS: list[Spot] = [
    Spot("sdk/rust/geotrace-sdk/Cargo.toml", _TOML_VERSION),
    Spot("sdk/rust/geotrace-sdk/Cargo.toml", _MACRO_PIN, note="macro pin"),
    Spot("sdk/rust/geotrace-sdk/Cargo.toml", _UNITS_PIN, note="units pin"),
    Spot("sdk/rust/geotrace-sdk-macros/Cargo.toml", _TOML_VERSION),
    Spot("sdk/rust/geotrace-sdk-units/Cargo.toml", _TOML_VERSION),
    Spot("sdk/rust/geotrace-c/Cargo.toml", _TOML_VERSION),
    Spot("sdk/python/geotrace-py/Cargo.toml", _TOML_VERSION),
    Spot("sdk/python/geotrace-py/pyproject.toml", _TOML_VERSION),
    Spot(_C_HEADER, _define_str("GEOTRACE_C_VERSION"), note="GEOTRACE_C_VERSION"),
    Spot(_C_CBINDGEN, _define_str("GEOTRACE_C_VERSION"), note="GEOTRACE_C_VERSION"),
    Spot(_CPP_HEADER, _define_str("GEOTRACE_CPP_VERSION"), note="GEOTRACE_CPP_VERSION"),
    Spot("sdk/c/CMakeLists.txt", _cmake_project("GeoTraceC"), core=True),
    Spot("sdk/cpp/CMakeLists.txt", _cmake_project("GeoTraceCpp"), core=True),
    # Cargo.lock pins (full version). Bumping them here keeps the locks in
    # lockstep with the manifests. Checking them makes a drifted lock fail
    # loudly instead of being silently re-resolved at build time. The SDK
    # packages are path crates with no checksum and are referenced by name only,
    # so rewriting the block's `version` line leaves each lock valid.
    Spot(_ROOT_LOCK, _lock_version("geotrace-sdk"), note="geotrace-sdk lock"),
    Spot(_ROOT_LOCK, _lock_version("geotrace-sdk-macros"), note="geotrace-sdk-macros lock"),
    Spot(_ROOT_LOCK, _lock_version("geotrace-sdk-units"), note="geotrace-sdk-units lock"),
    Spot(_ROOT_LOCK, _lock_version("geotrace-c"), note="geotrace-c lock"),
    Spot(_PY_LOCK, _lock_version("geotrace-py"), note="geotrace-py lock"),
    Spot(_PY_LOCK, _lock_version("geotrace-sdk"), note="geotrace-sdk lock"),
    Spot(_PY_LOCK, _lock_version("geotrace-sdk-macros"), note="geotrace-sdk-macros lock"),
    Spot(_PY_LOCK, _lock_version("geotrace-sdk-units"), note="geotrace-sdk-units lock"),
]

_APP_LOCK_CRATES: list[str] = [
    "geotrace",
    "gt-analysis",
    "gt-egui-mipmap",
    "gt-fetch",
    "gt-filter",
    "gt-flare",
    "gt-flare-store",
    "gt-fmt",
    "gt-geo-math",
    "gt-hdf5-archive",
    "gt-history",
    "gt-history-backend-pure",
    "gt-history-backend-sys",
    "gt-history-types",
    "gt-icon-tessellate",
    "gt-instance-lock",
    "gt-ionex",
    "gt-ionex-store",
    "gt-jam",
    "gt-jam-store",
    "gt-loaded-files",
    "gt-loader",
    "gt-log-view",
    "gt-logfile",
    "gt-map",
    "gt-pending-writes",
    "gt-plot",
    "gt-query",
    "gt-query-map-harness",
    "gt-query-run",
    "gt-side-panel",
    "gt-sky",
    "gt-store",
    "gt-snap",
    "gt-solar",
    "gt-solar-store",
    "gt-test-utils",
    "gt-track-builder",
    "gt-types",
    "gt-ui-theme",
    "gt-ui-types",
]

_APP_SPOTS: list[Spot] = [Spot("Cargo.toml", _TOML_VERSION)] + [
    Spot(_ROOT_LOCK, _lock_version(crate), note=f"{crate} lock") for crate in _APP_LOCK_CRATES
]


def app_lock_crate_errors(root: Path, listed_crates: list[str] | None = None) -> list[str]:
    """`_APP_LOCK_CRATES` must list exactly the workspace-versioned members.

    Every workspace member inheriting the workspace version gets its
    Cargo.lock pin rewritten by `bump-app`; a member missing from the list
    keeps its old pin, and the very next `cargo update --workspace --locked`
    (the release recipe and CI's lockfile guard) fails. That is how a new
    crate broke the app release without any earlier signal - this check
    derives the expected list from the workspace so the gap fails in CI the
    day the crate lands. The reverse also holds: a listed member that
    stopped inheriting would silently get the wrong version, so both
    directions error. (Entries for removed members already fail loudly
    through their unmatched lock spot.)
    """
    manifest = tomllib.loads((root / "Cargo.toml").read_text(encoding="utf-8"))
    listed = set(_APP_LOCK_CRATES if listed_crates is None else listed_crates) | {"geotrace"}
    errors: list[str] = []
    inheriting: set[str] = {"geotrace"}
    members: set[str] = {"geotrace"}
    for member in manifest.get("workspace", {}).get("members", []):
        package = tomllib.loads(
            (root / member / "Cargo.toml").read_text(encoding="utf-8")
        ).get("package", {})
        name = package.get("name")
        if name is None:
            errors.append(f"{member}/Cargo.toml: no package name found")
            continue
        members.add(name)
        if package.get("version") == {"workspace": True}:
            inheriting.add(name)
    errors += [
        f"{crate} inherits the workspace version but is missing from "
        f"_APP_LOCK_CRATES - bump-app would leave its Cargo.lock pin stale "
        f"and break `cargo update --workspace --locked`"
        for crate in sorted(inheriting - listed)
    ]
    errors += [
        f"{crate} is in _APP_LOCK_CRATES but no longer inherits the "
        f"workspace version - bump-app would rewrite its lock pin wrongly"
        for crate in sorted((listed & members) - inheriting)
    ]
    return errors


_APP_CHANGELOG = "CHANGELOG.md"
_SDK_CHANGELOG = "CHANGELOG_SDK.md"

# The numeric `*_MAJOR/MINOR/PATCH` macro triples (always the core version).
_INT_TRIPLES: list[tuple[str, str]] = [
    (_C_HEADER, "GEOTRACE_C_VERSION"),
    (_C_CBINDGEN, "GEOTRACE_C_VERSION"),
    (_CPP_HEADER, "GEOTRACE_CPP_VERSION"),
]


@dataclass
class Fact:
    """A recorded version found at a labelled spot."""

    label: str
    value: str
    core: bool = field(default=False)


def _read(root: Path, spot: Spot) -> Fact:
    text = (root / spot.path).read_text(encoding="utf-8")
    match = spot.pattern.search(text)
    if match is None:
        raise SystemExit(f"error: {spot.label()}: version spot not found")
    return Fact(spot.label(), match.group(2), spot.core)


def _read_int_triple(root: Path, path: str, macro: str) -> Fact:
    text = (root / path).read_text(encoding="utf-8")
    parts: list[str] = []
    for name in ("MAJOR", "MINOR", "PATCH"):
        match = _define_int(macro, name).search(text)
        if match is None:
            raise SystemExit(f"error: {path}: {macro}_{name} not found")
        parts.append(match.group(2))
    return Fact(f"{path} ({macro}_MAJOR/MINOR/PATCH)", ".".join(parts), core=True)


_RELEASE_WORKFLOW = ".github/workflows/release-sdk.yml"
_PUBLISH_LINE = re.compile(r"^\s*publish (geotrace-[a-z-]+)\s*$", re.MULTILINE)
_PATH_DEP = re.compile(r'^(geotrace-[a-z-]+) = \{[^}]*\bpath = "', re.MULTILINE)


def publish_closure_errors(root: Path) -> list[str]:
    """The crates.io publish list must be dependency-closed and pinned.

    `cargo publish` strips `path` from dependencies and resolves the remaining
    version requirement against the index - a path dependency of a published
    crate that is not itself published (in order, before its dependent) fails
    the upload. This is what happened when `geotrace-units` joined the SDK
    without joining the release workflow; this check fails the same gap in CI
    instead of at release time. Each such dependency must also be a version
    spot, so `bump-sdk` keeps it in lockstep.
    """
    workflow = (root / _RELEASE_WORKFLOW).read_text(encoding="utf-8")
    published = _PUBLISH_LINE.findall(workflow)
    if not published:
        return [f"{_RELEASE_WORKFLOW}: no `publish <crate>` lines found"]
    spot_paths = {spot.path for spot in _SDK_SPOTS}
    errors: list[str] = []
    for index, crate in enumerate(published):
        manifest = f"sdk/rust/{crate}/Cargo.toml"
        text = (root / manifest).read_text(encoding="utf-8")
        for dep in _PATH_DEP.findall(text):
            if dep not in published:
                errors.append(
                    f"{manifest}: path dependency {dep} is not published by "
                    f"{_RELEASE_WORKFLOW} - `cargo publish -p {crate}` will fail "
                    f"to resolve it from crates.io"
                )
            elif published.index(dep) > index:
                errors.append(
                    f"{_RELEASE_WORKFLOW}: {dep} is published after {crate}, "
                    f"which depends on it - reorder so the dependency uploads first"
                )
            if f"sdk/rust/{dep}/Cargo.toml" not in spot_paths:
                errors.append(
                    f"sdk/rust/{dep}/Cargo.toml: published crate is not a version "
                    f"spot - add it to _SDK_SPOTS so bump-sdk keeps it in lockstep"
                )
    return errors


def read_sdk_facts(root: Path) -> list[Fact]:
    facts = [_read(root, spot) for spot in _SDK_SPOTS]
    facts += [_read_int_triple(root, path, macro) for path, macro in _INT_TRIPLES]
    return facts


def _core(version: str) -> str:
    return version.split("-", 1)[0].split("+", 1)[0]


def _cmd_check(root: Path, expect: str | None) -> int:
    facts = read_sdk_facts(root)
    full_values = sorted({f.value for f in facts if not f.core})
    errors: list[str] = []

    if len(full_values) != 1:
        errors.append("full-version spots disagree")
    else:
        version = full_values[0]
        core_expected = _core(version)
        if sorted({f.value for f in facts if f.core} - {core_expected}):
            errors.append(f"numeric-core spots disagree (expected {core_expected})")
        if expect is not None and version != expect:
            errors.append(f"SDK version is {version}, but the release tag is {expect}")

    if expect is not None:
        errors += _changelog_errors(root, _SDK_CHANGELOG, expect, "bump-sdk")
    errors += publish_closure_errors(root)

    if errors:
        print("error: SDK version is inconsistent:")
        for fact in facts:
            print(f"  {fact.value:<16} {fact.label}")
        for err in errors:
            print(f"  -> {err}")
        return 1

    print(f"SDK version OK: {full_values[0]}")
    return 0


def _changelog_errors(root: Path, rel: str, expect: str, bump: str) -> list[str]:
    text = (root / rel).read_text(encoding="utf-8")
    if changelog.section_exists(text, expect):
        return []
    return [
        f"{rel} has no release section for {_core(expect)} - "
        f"promote [unreleased] with `just qa::{bump} {expect}` before releasing"
    ]


def _cmd_check_app(root: Path, expect: str | None) -> int:
    facts = [_read(root, spot) for spot in _APP_SPOTS]
    full_values = sorted({f.value for f in facts})
    errors: list[str] = app_lock_crate_errors(root)
    if len(full_values) != 1:
        errors.append("app version spots disagree")
    elif expect is not None and full_values[0] != expect:
        errors.append(f"app version is {full_values[0]}, but the release tag is {expect}")
    if expect is not None:
        errors += _changelog_errors(root, _APP_CHANGELOG, expect, "bump-app")

    if errors:
        print("error: app version is inconsistent:")
        for fact in facts:
            print(f"  {fact.value:<16} {fact.label}")
        for err in errors:
            print(f"  -> {err}")
        return 1

    print(f"app version OK: {full_values[0]}")
    return 0


def _apply(root: Path, spots: list[Spot], version: str, core: str) -> None:
    for spot in spots:
        path = root / spot.path
        text = path.read_text(encoding="utf-8")
        value = core if spot.core else version
        new, count = spot.pattern.subn(rf"\g<1>{value}", text)
        if count != 1:
            raise SystemExit(f"error: {spot.label()}: matched {count} spots, expected 1")
        path.write_text(new, encoding="utf-8")
        print(f"  {spot.label()} -> {value}")


def _apply_int_triples(root: Path, version: str) -> None:
    major, minor, patch = _core(version).split(".")
    for rel, macro in _INT_TRIPLES:
        path = root / rel
        text = path.read_text(encoding="utf-8")
        for name, num in (("MAJOR", major), ("MINOR", minor), ("PATCH", patch)):
            text, count = _define_int(macro, name).subn(rf"\g<1>{num}", text)
            if count != 1:
                raise SystemExit(f"error: {rel}: matched {count} {macro}_{name}, expected 1")
        path.write_text(text, encoding="utf-8")
        print(f"  {rel} ({macro}_MAJOR/MINOR/PATCH) -> {major}.{minor}.{patch}")


def _validate(version: str) -> None:
    if not _SEMVER.match(version):
        raise SystemExit(f"error: '{version}' is not a major.minor.patch version")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo-root", type=Path, default=repo_root())
    sub = parser.add_subparsers(dest="cmd", required=True)
    check = sub.add_parser("check", help="verify the SDK version is consistent")
    check.add_argument("--expect", help="also require the SDK version (and changelog) to match")
    check_app = sub.add_parser("check-app", help="verify the GUI app version is consistent")
    check_app.add_argument("--expect", help="also require the app version (and changelog) to match")
    bump_sdk = sub.add_parser("bump-sdk", help="set every SDK version and promote its changelog")
    bump_sdk.add_argument("version")
    bump_app = sub.add_parser("bump-app", help="set the GUI app version and promote its changelog")
    bump_app.add_argument("version")
    args = parser.parse_args()

    root: Path = args.repo_root.resolve()
    today = date.today()
    if args.cmd == "check":
        sys.exit(_cmd_check(root, args.expect))
    if args.cmd == "check-app":
        sys.exit(_cmd_check_app(root, args.expect))
    if args.cmd == "bump-sdk":
        _validate(args.version)
        _apply(root, _SDK_SPOTS, args.version, _core(args.version))
        _apply_int_triples(root, args.version)
        changelog.promote(root / _SDK_CHANGELOG, args.version, today)
        sys.exit(_cmd_check(root, args.version))
    if args.cmd == "bump-app":
        _validate(args.version)
        _apply(root, _APP_SPOTS, args.version, _core(args.version))
        changelog.promote(root / _APP_CHANGELOG, args.version, today, heading_style="cargo_dist")
        sys.exit(_cmd_check_app(root, args.version))

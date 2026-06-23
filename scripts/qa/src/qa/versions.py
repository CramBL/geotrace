"""Read, verify, and bump GeoTrace's version strings.

The GUI app and the SDK release on independent cadences, so their versions live
in different places:

- GUI: the workspace package version in the root ``Cargo.toml``.
- SDK: kept in lockstep across the Rust crates (``geotrace-sdk``,
  ``geotrace-sdk-macros``, ``geotrace-c``) and the macro dependency pin, the
  Python package (``Cargo.toml`` + ``pyproject.toml``), the C and C++ headers
  (``GEOTRACE_C_VERSION`` / ``GEOTRACE_CPP_VERSION`` and their numeric parts),
  the C and C++ CMake ``project(... VERSION)`` declarations, and the SDK-crate
  pins in both committed ``Cargo.lock`` files (the root workspace and the
  isolated Python workspace).

Most spots carry the full version (``0.2.0`` or ``0.2.0-rc.1``); CMake project
versions and the numeric ``*_MAJOR/MINOR/PATCH`` macros only hold the numeric
core (``0.2.0``), since they cannot express a prerelease suffix.

``check`` verifies every spot agrees (the guard run before publishing, and in
CI); ``bump-sdk`` / ``bump-app`` rewrite them atomically.
"""

import argparse
import re
import sys
from dataclasses import dataclass, field
from pathlib import Path

from qa._check import repo_root

_SEMVER = re.compile(r"^(\d+)\.(\d+)\.(\d+)(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?$")

# Each pattern captures the kept prefix as group 1 and the version token as
# group 2, consuming nothing after it (so the closing quote / rest of line is
# preserved on rewrite).
_TOML_VERSION = re.compile(r'^(version = ")([^"]*)', re.MULTILINE)
_MACRO_PIN = re.compile(r'(geotrace-sdk-macros = \{[^}]*\bversion = ")([^"]*)')


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

# The two committed Cargo.lock files that pin the SDK crates. The root lock is
# the main workspace (geotrace-c and its deps); the Python lock is an isolated
# workspace whose pins only refresh when cargo runs in its own directory, so it
# is the one that silently drifts after a bump.
_ROOT_LOCK = "Cargo.lock"
_PY_LOCK = "sdk/python/geotrace-py/Cargo.lock"

_SDK_SPOTS: list[Spot] = [
    Spot("sdk/rust/geotrace-sdk/Cargo.toml", _TOML_VERSION),
    Spot("sdk/rust/geotrace-sdk/Cargo.toml", _MACRO_PIN, note="macro pin"),
    Spot("sdk/rust/geotrace-sdk-macros/Cargo.toml", _TOML_VERSION),
    Spot("sdk/rust/geotrace-c/Cargo.toml", _TOML_VERSION),
    Spot("sdk/python/geotrace-py/Cargo.toml", _TOML_VERSION),
    Spot("sdk/python/geotrace-py/pyproject.toml", _TOML_VERSION),
    Spot(_C_HEADER, _define_str("GEOTRACE_C_VERSION"), note="GEOTRACE_C_VERSION"),
    Spot(_CPP_HEADER, _define_str("GEOTRACE_CPP_VERSION"), note="GEOTRACE_CPP_VERSION"),
    Spot("sdk/c/CMakeLists.txt", _cmake_project("GeoTraceC"), core=True),
    Spot("sdk/cpp/CMakeLists.txt", _cmake_project("GeoTraceCpp"), core=True),
    # Cargo.lock pins (full version). Bumping them here keeps the locks in
    # lockstep with the manifests; checking them makes a drifted lock fail
    # loudly instead of being silently re-resolved at build time. The SDK
    # packages are path crates with no checksum and are referenced by name only,
    # so rewriting the block's `version` line leaves each lock valid.
    Spot(_ROOT_LOCK, _lock_version("geotrace-sdk"), note="geotrace-sdk lock"),
    Spot(_ROOT_LOCK, _lock_version("geotrace-sdk-macros"), note="geotrace-sdk-macros lock"),
    Spot(_ROOT_LOCK, _lock_version("geotrace-c"), note="geotrace-c lock"),
    Spot(_PY_LOCK, _lock_version("geotrace-py"), note="geotrace-py lock"),
    Spot(_PY_LOCK, _lock_version("geotrace-sdk"), note="geotrace-sdk lock"),
    Spot(_PY_LOCK, _lock_version("geotrace-sdk-macros"), note="geotrace-sdk-macros lock"),
]

_APP_SPOTS: list[Spot] = [Spot("Cargo.toml", _TOML_VERSION)]

# The numeric `*_MAJOR/MINOR/PATCH` macro triples (always the core version).
_INT_TRIPLES: list[tuple[str, str]] = [
    (_C_HEADER, "GEOTRACE_C_VERSION"),
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

    if errors:
        print("error: SDK version is inconsistent:")
        for fact in facts:
            print(f"  {fact.value:<16} {fact.label}")
        for err in errors:
            print(f"  -> {err}")
        return 1

    print(f"SDK version OK: {full_values[0]}")
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
    check.add_argument("--expect", help="also require the SDK version to equal this")
    bump_sdk = sub.add_parser("bump-sdk", help="set every SDK version")
    bump_sdk.add_argument("version")
    bump_app = sub.add_parser("bump-app", help="set the GUI app version")
    bump_app.add_argument("version")
    args = parser.parse_args()

    root: Path = args.repo_root.resolve()
    if args.cmd == "check":
        sys.exit(_cmd_check(root, args.expect))
    if args.cmd == "bump-sdk":
        _validate(args.version)
        _apply(root, _SDK_SPOTS, args.version, _core(args.version))
        _apply_int_triples(root, args.version)
        sys.exit(_cmd_check(root, args.version))
    if args.cmd == "bump-app":
        _validate(args.version)
        _apply(root, _APP_SPOTS, args.version, _core(args.version))

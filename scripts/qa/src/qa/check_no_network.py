"""Forbid network access from the workspace's Rust tests and examples.

Every test runs against committed fixtures and a canned transport, so a run
never depends on a live service being reachable or on what it returns today.
The services themselves are pinned by the capture tools listed below and by
the fixture-freshness workflow that re-runs them on trunk.

The allowlist is keyed by file and by construct: a file that legitimately
sends requests is listed with the constructs it may use, and a file that only
mentions a URL as inert data (an expected value, a host constant a canned
transport never dials) is listed for `url-literal` alone.

Exemption syntax (same line), for a one-off outside the allowlist:

    let url = "https://example.invalid"; // [qa-allow-check-no-network, reason = "why"]
"""

import re
import sys
from pathlib import Path
from typing import NamedTuple

from qa._allow import is_exempt
from qa._check import Check, Violation, repo_root, rs_files, run_check

CHECK = "check-no-network"


class NetworkConstruct(NamedTuple):
    """One way a test could reach the network, and how to spot it."""

    name: str
    pattern: re.Pattern[str]


_CONSTRUCTS = (
    NetworkConstruct("http-transport", re.compile(r"HttpTransport::new")),
    NetworkConstruct("network-transport-source", re.compile(r"TransportSource::Network")),
    NetworkConstruct("reqwest", re.compile(r"\breqwest\b")),
    NetworkConstruct("url-literal", re.compile(r"https?://")),
)

_EVERY_CONSTRUCT = frozenset(construct.name for construct in _CONSTRUCTS)
_URL_LITERAL_ONLY = frozenset({"url-literal"})

_ALLOWED: dict[str, frozenset[str]] = {
    # The seven capture tools. Requesting the live service is their whole job:
    # `just ionex-fixtures` and its siblings run them by hand, and the
    # fixture-freshness workflow runs them on trunk.
    "crates/gt-flare/examples/fetch_flare_fixtures.rs": _EVERY_CONSTRUCT,
    "crates/gt-ionex/examples/fetch_ionex_fixtures.rs": _EVERY_CONSTRUCT,
    "crates/gt-ionex/examples/fetch_node_series_fixture.rs": _EVERY_CONSTRUCT,
    "crates/gt-jam/examples/fetch_jam_fixtures.rs": _EVERY_CONSTRUCT,
    "crates/gt-map/examples/fetch_map_tile_fixtures.rs": _EVERY_CONSTRUCT,
    "crates/gt-solar/examples/fetch_solar_fixtures.rs": _EVERY_CONSTRUCT,
    "crates/gt-snap/examples/fetch_snap_fixtures.rs": _EVERY_CONSTRUCT,
    # The CDDIS verification tool, run by hand through `just cddis-verify`:
    # the archive it addresses serves files to callers holding a per-user
    # Earthdata token, which CI has none of.
    "crates/gt-ionex/examples/verify_cddis_mirror.rs": _EVERY_CONSTRUCT,
    # The live map-matching API smoke test. Every test in it is `#[ignore]`d
    # and runs only under `just snap-live-test`.
    "crates/gt-snap/tests/live_api.rs": _EVERY_CONSTRUCT,
    # Host constants the archive tests build expected URLs from. The requests
    # go to a canned transport that responds from a committed fixture.
    "crates/gt-flare-store/tests/archive.rs": _URL_LITERAL_ONLY,
    "crates/gt-hdf5-archive/tests/columns.rs": _URL_LITERAL_ONLY,
    "crates/gt-hdf5-archive/tests/file_space_migration.rs": _URL_LITERAL_ONLY,
    "crates/gt-hdf5-archive/tests/prune.rs": _URL_LITERAL_ONLY,
    "crates/gt-ionex-store/tests/archive.rs": _URL_LITERAL_ONLY,
    "crates/gt-jam-store/tests/archive.rs": _URL_LITERAL_ONLY,
    "crates/gt-jam-store/tests/captured_day.rs": _URL_LITERAL_ONLY,
    "crates/gt-jam-store/tests/file_space_migration.rs": _URL_LITERAL_ONLY,
    "crates/gt-solar-store/tests/archive.rs": _URL_LITERAL_ONLY,
    # A URL parsed into its host part, asserted against the expected result.
    "crates/gt-snap/tests/wire_format.rs": _URL_LITERAL_ONLY,
}


def _collect(root: Path) -> list[Violation]:
    violations: list[Violation] = []
    for path in rs_files(root):
        parts = path.relative_to(root).parts
        if "tests" not in parts and "examples" not in parts:
            continue
        allowed = _ALLOWED.get(path.relative_to(root).as_posix(), frozenset())
        for lineno, raw in enumerate(path.read_text(errors="replace").splitlines(), 1):
            if is_exempt(raw, CHECK):
                continue
            if any(
                construct.name not in allowed and construct.pattern.search(raw)
                for construct in _CONSTRUCTS
            ):
                violations.append((path, lineno, raw.strip()))
    return violations


_NOTE = [
    "tests and examples run offline: they drive canned transports over committed fixtures,",
    "so a run never depends on a service being reachable or on what it answers today",
]
_HELP = [
    "replay a committed fixture through a canned transport, or, for a capture tool,",
    "add the file to check_no_network's allowlist. A single line exempts with:",
]

DEFINITION = Check(
    name=CHECK,
    title="network access found in a test or an example",
    collect=_collect,
    note=_NOTE,
    help=_HELP,
)


def main() -> None:
    if run_check(DEFINITION, repo_root()):
        sys.exit(1)

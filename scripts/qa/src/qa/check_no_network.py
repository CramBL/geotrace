"""Forbid network access from the workspace's Rust tests and examples.

Every test runs against committed fixtures and a canned transport, so a run
never depends on a live service being reachable or on what it returns today.
The services themselves are pinned by the capture tools listed below and by
the fixture-freshness workflow that re-runs them on trunk.

A file under `tests/` or `examples/`, and a file a parent module declares
`#[cfg(test)] mod …;`, is test code throughout and is read whole. Any other
file is read inside its `#[cfg(test)]` items alone, which is where the unit
tests of `src/` sit. The production code around them opens transports and
states the hosts it requests, and the check leaves it alone.

The allowlist is keyed by file and by construct: a file that legitimately
sends requests is listed with the constructs it may use, and a file that only
mentions a URL as inert data (an expected value, a host constant a canned
transport never dials) is listed for `url-literal` alone.

Exemption syntax (same line), for a one-off outside the allowlist:

    let url = "https://example.invalid"; // [qa-allow-check-no-network, reason = "why"]
"""

import re
import sys
from collections.abc import Iterator
from pathlib import Path
from typing import NamedTuple

from qa._allow import is_exempt
from qa._check import Check, Violation, is_test_only_module, repo_root, rs_files, run_check
from qa._rust import cfg_test_line_numbers

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

_ENVIRONMENTAL_DATA_DIR = "crates/external_environmental_data"

_ALLOWED: dict[str, frozenset[str]] = {
    # The seven capture tools. Requesting the live service is their whole job:
    # `just ionex-captures` and its siblings run them by hand, and the
    # fixture-freshness workflow runs them on trunk.
    f"{_ENVIRONMENTAL_DATA_DIR}/gt-flare/examples/fetch_flare_captures.rs": _EVERY_CONSTRUCT,
    f"{_ENVIRONMENTAL_DATA_DIR}/gt-ionex/examples/fetch_ionex_captures.rs": _EVERY_CONSTRUCT,
    f"{_ENVIRONMENTAL_DATA_DIR}/gt-ionex/examples/fetch_node_series_capture.rs": _EVERY_CONSTRUCT,
    f"{_ENVIRONMENTAL_DATA_DIR}/gt-jam/examples/fetch_jam_captures.rs": _EVERY_CONSTRUCT,
    "crates/gt-map/examples/fetch_map_tile_captures.rs": _EVERY_CONSTRUCT,
    f"{_ENVIRONMENTAL_DATA_DIR}/gt-solar/examples/fetch_solar_captures.rs": _EVERY_CONSTRUCT,
    "crates/gt-snap/examples/fetch_snap_captures.rs": _EVERY_CONSTRUCT,
    # The CDDIS verification tool, run by hand through `just cddis-verify`:
    # the archive it addresses serves files to callers holding a per-user
    # Earthdata token, which CI has none of.
    f"{_ENVIRONMENTAL_DATA_DIR}/gt-ionex/examples/verify_cddis_mirror.rs": _EVERY_CONSTRUCT,
    # The live map-matching API smoke test. Every test in it is `#[ignore]`d
    # and runs only under `just snap-live-test`.
    "crates/gt-snap/tests/live_api.rs": _EVERY_CONSTRUCT,
    # Host constants the archive tests build expected URLs from. The requests
    # go to a canned transport that responds from a committed fixture.
    f"{_ENVIRONMENTAL_DATA_DIR}/gt-flare-store/tests/archive.rs": _URL_LITERAL_ONLY,
    "crates/gt-hdf5-archive/tests/columns.rs": _URL_LITERAL_ONLY,
    "crates/gt-hdf5-archive/tests/file_space_migration.rs": _URL_LITERAL_ONLY,
    "crates/gt-hdf5-archive/tests/prune.rs": _URL_LITERAL_ONLY,
    f"{_ENVIRONMENTAL_DATA_DIR}/gt-ionex-store/tests/archive.rs": _URL_LITERAL_ONLY,
    f"{_ENVIRONMENTAL_DATA_DIR}/gt-jam-store/tests/archive.rs": _URL_LITERAL_ONLY,
    f"{_ENVIRONMENTAL_DATA_DIR}/gt-jam-store/tests/captured_day.rs": _URL_LITERAL_ONLY,
    f"{_ENVIRONMENTAL_DATA_DIR}/gt-jam-store/tests/file_space_migration.rs": _URL_LITERAL_ONLY,
    f"{_ENVIRONMENTAL_DATA_DIR}/gt-solar-store/tests/archive.rs": _URL_LITERAL_ONLY,
    # A URL parsed into its host part, asserted against the expected result.
    "crates/gt-snap/tests/wire_format.rs": _URL_LITERAL_ONLY,
    # The URL builders of the environment feeds, each case asserting the
    # address one fetch would request. The fetches these tests run go to a
    # canned transport.
    f"{_ENVIRONMENTAL_DATA_DIR}/gt-flare/src/lib.rs": _URL_LITERAL_ONLY,
    f"{_ENVIRONMENTAL_DATA_DIR}/gt-flare/src/transport.rs": _URL_LITERAL_ONLY,
    f"{_ENVIRONMENTAL_DATA_DIR}/gt-ionex/src/cddis.rs": _URL_LITERAL_ONLY,
    f"{_ENVIRONMENTAL_DATA_DIR}/gt-ionex/src/lib.rs": _URL_LITERAL_ONLY,
    f"{_ENVIRONMENTAL_DATA_DIR}/gt-ionex/src/mirrors.rs": _URL_LITERAL_ONLY,
    f"{_ENVIRONMENTAL_DATA_DIR}/gt-ionex/src/transport.rs": _URL_LITERAL_ONLY,
    f"{_ENVIRONMENTAL_DATA_DIR}/gt-jam/src/lib.rs": _URL_LITERAL_ONLY,
    f"{_ENVIRONMENTAL_DATA_DIR}/gt-jam/src/transport.rs": _URL_LITERAL_ONLY,
    f"{_ENVIRONMENTAL_DATA_DIR}/gt-solar/src/lib.rs": _URL_LITERAL_ONLY,
    f"{_ENVIRONMENTAL_DATA_DIR}/gt-solar/src/transport.rs": _URL_LITERAL_ONLY,
    # The request builder and the satellite tile URL, asserted against the
    # string a request would address.
    "crates/gt-fetch/src/lib.rs": _URL_LITERAL_ONLY,
    "crates/gt-map/src/mapbox_tiles.rs": _URL_LITERAL_ONLY,
    # The XML namespace of an inline SVG fixture. Its line sits inside a raw
    # string, where a `//` exemption comment would become fixture text.
    "crates/gt-icon-tessellate/src/tessellate.rs": _URL_LITERAL_ONLY,
    # The citation links of a reference document the test itself builds.
    "crates/gt-ui-types/src/reference.rs": _URL_LITERAL_ONLY,
    "src/app/reference_window/tests.rs": _URL_LITERAL_ONLY,
    # Host strings the app's tests set: on a fetch scheduler, in the TEC mirror
    # editor, or in the settings text they round-trip. A scheduler among them
    # fetches over a canned transport.
    "src/app/flares.rs": _URL_LITERAL_ONLY,
    "src/app/jamming.rs": _URL_LITERAL_ONLY,
    "src/app/settings_ui/persist.rs": _URL_LITERAL_ONLY,
    "src/app/snap.rs": _URL_LITERAL_ONLY,
    "src/app/solar.rs": _URL_LITERAL_ONLY,
    "src/app/tec.rs": _URL_LITERAL_ONLY,
    "src/app/tec_mirrors_ui.rs": _URL_LITERAL_ONLY,
    "src/app/ui_tests.rs": _URL_LITERAL_ONLY,
    "src/settings.rs": _URL_LITERAL_ONLY,
    # The token test's own tests, which pass `TransportSource::Network` to the
    # settings row and never click the button. `MapboxTokenTest::start` opens
    # the transport on a click, and every request in the file goes to a
    # scripted transport the test builds.
    "src/app/mapbox_token_test.rs": frozenset({"network-transport-source"}),
}


def _lines_compiled_for_tests(root: Path, path: Path, source: str) -> Iterator[tuple[int, str]]:
    """Each line of `source` compiled for a test run, with its 1-based number."""
    parts = path.relative_to(root).parts
    whole_file = "tests" in parts or "examples" in parts or is_test_only_module(path)
    numbers = None if whole_file else cfg_test_line_numbers(source)
    for lineno, raw in enumerate(source.splitlines(), 1):
        if numbers is None or lineno in numbers:
            yield lineno, raw


def _collect(root: Path) -> list[Violation]:
    violations: list[Violation] = []
    for path in rs_files(root):
        allowed = _ALLOWED.get(path.relative_to(root).as_posix(), frozenset())
        source = path.read_text(errors="replace")
        for lineno, raw in _lines_compiled_for_tests(root, path, source):
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

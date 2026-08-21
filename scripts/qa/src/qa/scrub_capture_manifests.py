"""Normalize the nondeterministic fields of the fixture capture manifests.

Two correct captures of the same data differ in `captured_at` and nowhere
else: it holds the wall clock at capture time. A change in any other field is
a change in what the service served and stays in the diff, because all of them
are derived from the response (HTTP status, content type, parsed sample
counts, the service's own license and source strings, the URL, the map epochs
and grid geometry). The captured data files are never touched: they must be
byte-identical.

The fixture-freshness workflow scrubs the committed manifests into its
comparison baseline, re-captures, scrubs again, and diffs the two.
"""

import argparse
import json
import sys
from pathlib import Path
from typing import Any

from qa._check import repo_root

SCRUBBED_VALUE = "<scrubbed>"

_NONDETERMINISTIC_FIELDS = frozenset({"captured_at"})

# The manifests of the sources the fixture-freshness workflow re-captures.
# Three manifests are absent: gt-snap's map-matching answers change with every
# OpenStreetMap edit, so it is checked on demand through `just snap-live-test`,
# gt-flare's endpoint needs a per-user api.nasa.gov key that CI has none of, so
# it is re-captured by hand through `just flare-fixtures`, and the CDDIS
# manifest under gt-ionex/tests/fixtures/cddis/ needs a per-user Earthdata
# token, so `just cddis-verify --capture` writes it by hand. Naming one of
# those on the command line scrubs it all the same.
_MANIFESTS = (
    "crates/gt-ionex/tests/fixtures/capture.json",
    "crates/gt-jam/tests/fixtures/capture.json",
    "crates/gt-solar/tests/fixtures/capture.json",
)


def _scrub(node: Any) -> int:
    """Replace every nondeterministic field below `node`, returning how many."""
    if isinstance(node, dict):
        scrubbed = 0
        for key, value in node.items():
            if key in _NONDETERMINISTIC_FIELDS:
                node[key] = SCRUBBED_VALUE
                scrubbed += 1
            else:
                scrubbed += _scrub(value)
        return scrubbed
    if isinstance(node, list):
        return sum(_scrub(item) for item in node)
    return 0


def scrub_manifest(path: Path) -> int:
    """Scrub `path` in place, returning how many fields were replaced.

    Raises if the manifest holds none: the capture tool's schema changed, and
    a scrub that silently does nothing would report a fresh capture's
    timestamps as fixture drift.
    """
    manifest = json.loads(path.read_text(encoding="utf-8"))
    scrubbed = _scrub(manifest)
    if scrubbed == 0:
        raise ValueError(
            f"{path}: no {'/'.join(sorted(_NONDETERMINISTIC_FIELDS))} field found - "
            "the capture manifest schema changed, update _NONDETERMINISTIC_FIELDS"
        )
    path.write_text(f"{json.dumps(manifest, indent=2, ensure_ascii=False)}\n", encoding="utf-8")
    return scrubbed


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "manifests",
        nargs="*",
        type=Path,
        help="capture manifests to scrub (default: every fixture-freshness manifest)",
    )
    args = parser.parse_args()

    root = repo_root()
    paths: list[Path] = args.manifests or [root / manifest for manifest in _MANIFESTS]
    failed = False
    for path in paths:
        try:
            print(f"{path}: scrubbed {scrub_manifest(path)} field(s)")
        except (OSError, ValueError) as error:
            print(f"error: {error}", file=sys.stderr)
            failed = True

    if failed:
        sys.exit(1)

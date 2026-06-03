#!/usr/bin/env python3
"""Skipping event variants with ``event_kind.skip``.

An attribute set to ``event_kind.skip`` returns the skip sentinel instead of a
path string.
Passing the sentinel to ``EventMarker()`` converts it to ``None``, and
``NavFileBuilder.add()`` silently ignores such markers — no error, no entry in
the output file.
"""

from __future__ import annotations

import tempfile
from datetime import UTC, datetime, timedelta
from pathlib import Path

from geotrace_sdk import EventMarker, NavFileBuilder, NavFix, event_kind
START = datetime(2024, 6, 1, 8, 0, 0, tzinfo=UTC)


@event_kind
class DiagEvent:
    boot = None
    internal_health_check = event_kind.skip  # never written to the file
    shutdown = None


builder = NavFileBuilder()

for secs, lat, lon in [
    (0, 51.5074, -0.1278),
    (60, 51.5088, -0.1248),
    (120, 51.5103, -0.1217),
]:
    builder.add(NavFix(lat=lat, lon=lon, gps_time=START + timedelta(seconds=secs)))

builder.add(EventMarker(DiagEvent.boot, START + timedelta(seconds=2)))
builder.add(EventMarker(DiagEvent.internal_health_check, START + timedelta(seconds=30)))

builder.add(EventMarker(DiagEvent.shutdown, START + timedelta(seconds=110)))

nav_file = builder.finish()

out = Path(tempfile.gettempdir()) / "geotrace_event_markers_skip.nvd"
nav_file.write_to_file(out)

loaded = nav_file.__class__.open(out)
print(
    f"Event markers: {len(loaded.event_markers)}  (internal_health_check not recorded)"
)
for em in loaded.event_markers:
    print(f"  {em.variant_path}")

assert len(loaded.event_markers) == 2, "skip variant must not appear in output"

out.unlink()

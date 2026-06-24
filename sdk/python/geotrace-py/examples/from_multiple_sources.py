#!/usr/bin/env python3
"""Aggregate data from multiple sources into a single .gtd GeoTrace data file.

Scenario: your GPS unit logs fixes to one source, and a separate system (a test
harness, an annotation tool, a sensor log) records named events with their own
timestamps. Both are added independently to the builder. finish() sorts
everything by time and interpolates each annotation's map position from the two
surrounding GPS fixes.
"""

from __future__ import annotations

import tempfile
from datetime import UTC, datetime, timedelta
from pathlib import Path

from geotrace_sdk import Annotation, MarkerIcon, NavFileBuilder, NavFix

START = datetime(2024, 6, 1, 8, 0, 0, tzinfo=UTC)

# Source 1: GPS track (lat, lon, heading), one fix every 10 s.
GPS_FIXES = [
    (51.5074, -0.1278, 90.0),
    (51.5075, -0.1276, 91.0),
    (51.5076, -0.1274, 89.5),
    (51.5077, -0.1272, 88.0),
    (51.5078, -0.1270, 90.0),
    (51.5079, -0.1268, 90.5),
]

# Source 2: annotations from a separate log. Their positions are not supplied,
# finish() interpolates them from the GPS fixes by timestamp.
ANNOTATIONS = [
    (5, "Pothole", MarkerIcon.WARNING),
    (15, "Speed camera", MarkerIcon.CIRCLE),
    (25, "Junction", MarkerIcon.PIN),
]

builder = (
    NavFileBuilder()
    .with_title("Merged GPS + annotations")
    .with_device("Aggregator v1.0")
)

for i, (lat, lon, heading) in enumerate(GPS_FIXES):
    builder.add(
        NavFix(
            lat=lat,
            lon=lon,
            gps_time=START + timedelta(seconds=i * 10),
            heading=heading,
        )
    )

for offset, label, icon in ANNOTATIONS:
    builder.add(
        Annotation(time=START + timedelta(seconds=offset), label=label, icon=icon)
    )

nav_file = builder.finish()

out = Path(tempfile.gettempdir()) / "from_multiple_sources.gtd"
nav_file.write_to_file(out)

fix_count = len(nav_file.points)
marker_count = len(nav_file.markers)
print(f"Merged {fix_count} GPS fixes + {marker_count} annotations -> {out}")
print("Annotations were interpolated onto the track by timestamp.")

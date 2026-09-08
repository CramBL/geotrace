#!/usr/bin/env python3
"""Write and read back a .gtd file containing event markers.

Scenario: a device logs structured events alongside its GPS track.
Each event has a slash-separated variant path that places it in a
hierarchy - e.g. ``"connectivity/agps/request"`` - so GeoTrace can
group and filter them by prefix in the Events panel.

Event markers are added via ``builder.add()``.
Per-variant colors and icons are optional. Unlisted variants get a
deterministic fallback color derived from their path.
"""

from __future__ import annotations

import tempfile
from datetime import UTC, datetime, timedelta
from pathlib import Path

from geotrace_sdk import (
    EventMarker,
    EventMarkerStyle,
    MarkerIcon,
    NavFile,
    NavFileBuilder,
    NavFix,
)

START = datetime(2024, 6, 1, 8, 0, 0, tzinfo=UTC)

# A short London track, one fix every 30 s.
TRACK = [
    # (`seconds`, `lat`, `lon`)
    (0, 51.5074, -0.1278),
    (30, 51.5080, -0.1265),
    (60, 51.5088, -0.1248),
    (90, 51.5095, -0.1233),
    (120, 51.5103, -0.1217),
    (150, 51.5110, -0.1200),
]

# Flat and nested variant paths.
EVENTS = [
    # (`variant_path`, `seconds`, `annotation`)
    ("power/boot", 2, "cold start"),
    ("connectivity/agps/request", 5, "EPO fetch started"),
    ("connectivity/agps/success", 18, "EPO applied, TTFF reduced"),
    ("sensor/gps/lock_acquired", 20, None),
    ("power/sleep", 145, None),
]

builder = (
    NavFileBuilder().with_title("Event marker tour").with_device("Example GPS v1.0")
)

for secs, lat, lon in TRACK:
    builder.add(
        NavFix(lat=lat, lon=lon, gps_time=START + timedelta(seconds=secs), heading=90.0)
    )

for path, secs, note in EVENTS:
    builder.add(EventMarker(path, START + timedelta(seconds=secs), annotation=note))

builder.add_event_marker_style(
    EventMarkerStyle("power/boot", icon=MarkerIcon.LIGHTNING, color="#44BB44")
)
builder.add_event_marker_style(
    EventMarkerStyle("power/sleep", icon=MarkerIcon.PIN, color="#4488FF")
)

nav_file = builder.finish()

out = Path(tempfile.gettempdir()) / "geotrace_event_markers.gtd"
nav_file.write_to_file(out)

try:
    loaded = NavFile.open(out)
    print(f"Nav points: {len(loaded.points)}")
    print(f"Event markers: {len(loaded.event_markers)}")
    print(f"Event marker styles: {len(loaded.event_marker_styles)}")
    for event_marker in loaded.event_markers:
        line = (
            f"  {event_marker.variant_path}  "
            f"{event_marker.lat:.5f}, {event_marker.lon:.5f}"
        )
        if event_marker.annotation:
            line += f" - {event_marker.annotation}"
        print(line)
finally:
    out.unlink()

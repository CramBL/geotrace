#!/usr/bin/env python3
"""Type-safe event markers using the @event_kind decorator - flat (single-level) case.

The decorator converts each class attribute to a snake_case path string.
All attributes in this example are leaves, with no nesting.
"""

from __future__ import annotations

import tempfile
from datetime import UTC, datetime, timedelta
from pathlib import Path

from geotrace_sdk import EventMarker, NavFileBuilder, NavFix, event_kind

START = datetime(2024, 6, 1, 8, 0, 0, tzinfo=UTC)


@event_kind
class PowerEvent:
    Boot = None
    Sleep = None
    BatteryLow = None


builder = NavFileBuilder()

for secs, lat, lon in [
    (0, 51.5074, -0.1278),
    (60, 51.5088, -0.1248),
    (120, 51.5103, -0.1217),
]:
    builder.add(NavFix(lat=lat, lon=lon, gps_time=START + timedelta(seconds=secs)))

builder.add(
    EventMarker(PowerEvent.Boot, START + timedelta(seconds=2), annotation="cold start")
)
builder.add(
    EventMarker(PowerEvent.BatteryLow, START + timedelta(seconds=90), annotation="14%")
)
builder.add(EventMarker(PowerEvent.Sleep, START + timedelta(seconds=115)))

nav_file = builder.finish()

out = Path(tempfile.gettempdir()) / "geotrace_event_markers_flat.gtd"
nav_file.write_to_file(out)

loaded = nav_file.__class__.open(out)
print(f"Event markers: {len(loaded.event_markers)}")
for em in loaded.event_markers:
    print(f"  {em.variant_path}")

out.unlink()

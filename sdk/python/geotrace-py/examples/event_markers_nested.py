#!/usr/bin/env python3
"""Type-safe event markers using the @event_kind decorator - nested (3-level) case.

Inner classes become intermediate path segments.
The full path is built by concatenating the snake_case attribute names from
outermost to innermost:

    Event.Connectivity.Agps.request  →  "connectivity/agps/request"
"""

from __future__ import annotations

import tempfile
from datetime import UTC, datetime, timedelta
from pathlib import Path

from geotrace_sdk import EventMarker, NavFileBuilder, NavFix, event_kind

START = datetime(2024, 6, 1, 8, 0, 0, tzinfo=UTC)


@event_kind
class Event:
    class Power:
        boot = None
        sleep = None
        battery_low = None

    class Connectivity:
        class Agps:
            request = None
            success = None
            timeout = None

    class Sensor:
        class Gps:
            lock_acquired = None
            lock_lost = None


builder = NavFileBuilder()

for secs, lat, lon in [
    (0, 51.5074, -0.1278),
    (30, 51.5080, -0.1265),
    (60, 51.5088, -0.1248),
    (90, 51.5095, -0.1233),
    (120, 51.5103, -0.1217),
]:
    builder.add(NavFix(lat=lat, lon=lon, gps_time=START + timedelta(seconds=secs)))

builder.add(
    EventMarker(Event.Power.boot, START + timedelta(seconds=2), annotation="cold start")
)
builder.add(EventMarker(Event.Connectivity.Agps.request, START + timedelta(seconds=5)))
builder.add(EventMarker(Event.Connectivity.Agps.success, START + timedelta(seconds=18)))
builder.add(EventMarker(Event.Sensor.Gps.lock_acquired, START + timedelta(seconds=20)))
builder.add(
    EventMarker(
        Event.Power.battery_low, START + timedelta(seconds=100), annotation="14%"
    )
)
builder.add(EventMarker(Event.Power.sleep, START + timedelta(seconds=115)))

nav_file = builder.finish()

out = Path(tempfile.gettempdir()) / "geotrace_event_markers_nested.gtd"
nav_file.write_to_file(out)

loaded = nav_file.__class__.open(out)
print(f"Event markers: {len(loaded.event_markers)}")
for em in loaded.event_markers:
    note = f"  - {em.annotation}" if em.annotation else ""
    print(f"  {em.variant_path}{note}")

out.unlink()

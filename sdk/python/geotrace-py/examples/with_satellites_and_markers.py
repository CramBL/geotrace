#!/usr/bin/env python3
"""Write a .gtd file with satellite reports and custom map annotations.

Scenario: a field test drive where the engineer annotates points of interest
(a pothole, a speed-camera location, a junction) while the GPS and satellite
data are being logged concurrently.

The builder places each annotation on the track by interpolating its position
from the two surrounding GPS fixes based on timestamp.
"""

from __future__ import annotations

import tempfile
from datetime import UTC, datetime, timedelta
from pathlib import Path

from geotrace_sdk import (
    Annotation,
    Constellation,
    MarkerIcon,
    NavFileBuilder,
    NavFix,
    Satellite,
    SatelliteReport,
)

START = datetime(2024, 7, 22, 8, 30, 0, tzinfo=UTC)

# A stretch of the A3 southbound from Elephant & Castle toward Kennington.
FIXES = [
    # (`seconds`, `lat`, `lon`, `heading`, `speed_mps`, `eph_m`)
    (0, 51.4953, -0.1005, 175.0, 0.0, 5.1),
    (5, 51.4947, -0.1007, 174.0, 8.3, 4.4),
    (10, 51.4940, -0.1009, 176.0, 11.1, 3.9),
    (15, 51.4933, -0.1011, 177.0, 12.5, 3.6),
    (20, 51.4926, -0.1013, 175.0, 13.2, 3.4),
    (25, 51.4919, -0.1015, 174.0, 13.9, 3.2),
    (30, 51.4912, -0.1017, 176.0, 13.6, 3.3),
    (35, 51.4905, -0.1019, 178.0, 12.8, 3.5),
    (40, 51.4897, -0.1021, 177.0, 11.4, 3.8),
    (45, 51.4890, -0.1023, 175.0, 0.0, 4.2),
]

SATS = [
    # (`constellation`, `prn`, `in_fix`, `elev`, `az`, `snr`)
    (Constellation.GPS, 1, True, 68.0, 120.0, 43.0),
    (Constellation.GPS, 11, True, 52.0, 215.0, 40.0),
    (Constellation.GPS, 17, True, 38.0, 50.0, 36.0),
    (Constellation.GPS, 28, False, 14.0, 330.0, 25.0),
    (Constellation.GALILEO, 4, True, 60.0, 180.0, 41.0),
    (Constellation.GALILEO, 9, True, 29.0, 95.0, 34.0),
    (Constellation.GALILEO, 21, False, 10.0, 5.0, 19.0),
    (Constellation.GLONASS, 3, False, 22.0, 255.0, 29.0),
]

ANNOTATIONS = [
    # (seconds, label,                    icon)
    (7, "Start of roadworks", MarkerIcon.WARNING),
    (18, "Pothole - surface damage", MarkerIcon.ERROR),
    (27, "Speed camera", MarkerIcon.LIGHTNING),
    (38, "Kennington junction", MarkerIcon.PIN),
]

builder = (
    NavFileBuilder()
    .with_title("A3 Southbound - field test")
    .with_device("u-blox ZED-F9P")
    .with_notes("Engineer annotations recorded via tablet app")
)

for secs, lat, lon, heading, speed, eph in FIXES:
    t = START + timedelta(seconds=secs)
    builder.add(
        NavFix(
            lat=lat,
            lon=lon,
            gps_time=t,
            heading=heading,
            speed_mps=speed,
            eph_m=eph,
        )
    )
    builder.add(
        SatelliteReport(
            [
                Satellite(c, prn, in_fix=in_fix, elevation=elev, azimuth=az, snr=snr)
                for c, prn, in_fix, elev, az, snr in SATS
            ],
            gps_time=t,
        )
    )

for secs, label, icon in ANNOTATIONS:
    builder.add(
        Annotation(
            START + timedelta(seconds=secs),
            label=label,
            icon=icon,
        )
    )

nav_file = builder.finish()

out = Path(tempfile.gettempdir()) / "with_satellites_and_markers.gtd"
nav_file.write_to_file(out)

print(f"Written to {out}")
print(f"  {len(nav_file.points)} nav points, {len(nav_file.markers)} markers")
print()
print("Markers:")
for m in nav_file.markers:
    print(f"  {m.label!r:35s}  @ {m.lat:.5f}, {m.lon:.5f}  [{m.icon}]")

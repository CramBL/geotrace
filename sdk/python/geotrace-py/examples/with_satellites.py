#!/usr/bin/env python3
"""Write a .gtd file that includes satellite visibility reports.

Each nav fix is paired with a satellite report captured at the same moment.
The builder associates each report to the nearest fix within the default
500 ms window, so timestamps here match exactly.
"""

from __future__ import annotations

import tempfile
from datetime import UTC, datetime, timedelta
from pathlib import Path

from geotrace_sdk import (
    Constellation,
    NavFileBuilder,
    NavFix,
    Satellite,
    SatelliteReport,
)

START = datetime(2024, 3, 10, 14, 0, 0, tzinfo=UTC)

# Short urban loop - roughly Southwark, London.
FIXES = [
    # (`seconds`, `lat`, `lon`, `heading`, `speed_mps`, `eph_m`)
    (0, 51.5030, -0.0978, 5.0, 0.0, 4.2),
    (10, 51.5038, -0.0975, 8.0, 3.1, 3.8),
    (20, 51.5045, -0.0971, 12.0, 4.4, 3.5),
    (30, 51.5053, -0.0966, 10.0, 4.6, 3.1),
    (40, 51.5060, -0.0961, 7.0, 4.4, 2.9),
    (50, 51.5067, -0.0957, 5.0, 3.8, 3.0),
]

# A realistic mixed GPS + Galileo sky: eight satellites, five in the fix.
SAT_TEMPLATE = [
    # (`constellation`, `prn`, `in_fix`, `elev`, `az`, `snr`)
    (Constellation.GPS, 3, True, 72.0, 145.0, 44.0),
    (Constellation.GPS, 8, True, 58.0, 230.0, 41.0),
    (Constellation.GPS, 14, True, 41.0, 60.0, 37.0),
    (Constellation.GPS, 22, False, 18.0, 310.0, 28.0),
    (Constellation.GALILEO, 7, True, 65.0, 195.0, 42.0),
    (Constellation.GALILEO, 12, True, 33.0, 90.0, 35.0),
    (Constellation.GALILEO, 19, False, 12.0, 15.0, 22.0),
    (Constellation.GLONASS, 5, False, 25.0, 270.0, 31.0),
]

builder = NavFileBuilder()

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

    # Vary SNR slightly per fix to simulate changing signal conditions.
    snr_offset = secs / 100.0
    sats = [
        Satellite(
            c,
            prn,
            in_fix=in_fix,
            elevation=elev,
            azimuth=az,
            snr=round(snr - snr_offset, 1),
        )
        for c, prn, in_fix, elev, az, snr in SAT_TEMPLATE
    ]
    builder.add(SatelliteReport(sats, gps_time=t))

nav_file = builder.finish()

out = Path(tempfile.gettempdir()) / "with_satellites.gtd"
nav_file.write_to_file(out)

in_fix_count = sum(
    1
    for pt in nav_file.points
    if pt.satellites and any(s.in_fix for s in pt.satellites.tracked)
)
print(
    f"Written {len(nav_file.points)} fixes "
    f"({in_fix_count} with satellite fix data) to {out}"
)

#!/usr/bin/env python3
"""Write a .gtd file that includes satellite visibility reports.

Each nav fix is paired with a satellite report captured at the same moment.
The builder associates each report to the nearest fix within the default
500 ms window, so timestamps here match exactly.

The example writes the file, reads it back, and prints the per-fix satellite
counts - the data GeoTrace shows in its sky view.
"""

from __future__ import annotations

import tempfile
from datetime import UTC, datetime, timedelta
from pathlib import Path

from geotrace_sdk import (
    Constellation,
    NavFile,
    NavFileBuilder,
    NavFix,
    Satellite,
    SatelliteReport,
)

START = datetime(2024, 6, 1, 8, 0, 0, tzinfo=UTC)

# A short urban loop through Southwark, London, one fix every 10 s.
TRACK = [
    # (`seconds`, `lat`, `lon`, `heading`, `speed_mps`, `eph_m`)
    (0, 51.5030, -0.0978, 5.0, 0.0, 4.2),
    (10, 51.5038, -0.0975, 8.0, 3.1, 3.8),
    (20, 51.5045, -0.0971, 12.0, 4.4, 3.5),
    (30, 51.5053, -0.0966, 10.0, 4.6, 3.1),
    (40, 51.5060, -0.0961, 7.0, 4.4, 2.9),
    (50, 51.5067, -0.0957, 5.0, 3.8, 3.0),
]

# A mixed GPS, Galileo and GLONASS sky: eight satellites, five in the fix.
# GLONASS 5 has an SNR and no elevation or azimuth. A receiver reports that for
# a satellite whose position it has not computed.
SKY = [
    # (`constellation`, `prn`, `in_fix`, `elevation`, `azimuth`, `snr`)
    (Constellation.GPS, 3, True, 72.0, 145.0, 44.0),
    (Constellation.GPS, 8, True, 58.0, 230.0, 41.0),
    (Constellation.GPS, 14, True, 41.0, 60.0, 37.0),
    (Constellation.GPS, 22, False, 18.0, 310.0, 28.0),
    (Constellation.GALILEO, 7, True, 65.0, 195.0, 42.0),
    (Constellation.GALILEO, 12, True, 33.0, 90.0, 35.0),
    (Constellation.GALILEO, 19, False, 12.0, 15.0, 22.0),
    (Constellation.GLONASS, 5, False, None, None, 31.0),
]

builder = (
    NavFileBuilder()
    .with_title("Satellite quality tour")
    .with_device("Example GNSS v1.0")
)

for i, (secs, lat, lon, heading, speed, eph) in enumerate(TRACK):
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

    # SNR climbs slightly along the track as the receiver settles.
    snr_gain = 0.5 * i
    builder.add(
        SatelliteReport(
            [
                Satellite(
                    constellation,
                    prn,
                    in_fix=in_fix,
                    elevation=elevation,
                    azimuth=azimuth,
                    snr=snr + snr_gain,
                )
                for constellation, prn, in_fix, elevation, azimuth, snr in SKY
            ],
            gps_time=t,
        )
    )

out = Path(tempfile.gettempdir()) / "geotrace_with_satellites.gtd"
builder.finish().write_to_file(out)

try:
    loaded = NavFile.open(out)
    print(f"Nav points: {len(loaded.points)}")
    for i, point in enumerate(loaded.points):
        tracked = point.satellites.tracked if point.satellites else []
        in_fix_count = sum(1 for satellite in tracked if satellite.in_fix)
        print(f"  [{i}] {len(tracked)} tracked, {in_fix_count} in fix")
finally:
    out.unlink()

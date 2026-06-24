#!/usr/bin/env python3
"""Convert GPS data from CSV text into a .gtd GeoTrace data file.

Scenario: your GPS logger exports fixes as CSV rows. Parse each row, feed the
fields to the builder, then finish() to produce a validated file ready for
GeoTrace to open. In a real workflow you would read the CSV from a file
instead of the inline CSV_DATA constant.

Timestamps here are whole Unix epoch seconds to keep the parser tiny.
"""

from __future__ import annotations

import csv
import io
import tempfile
from datetime import UTC, datetime
from pathlib import Path

from geotrace_sdk import NavFileBuilder, NavFix

CSV_DATA = """\
timestamp_s,lat,lon,heading_deg,speed_mps
1705309200,51.5074,-0.1278,90.0,12.5
1705309201,51.5075,-0.1276,91.0,12.6
1705309202,51.5076,-0.1274,89.5,12.4
1705309203,51.5077,-0.1272,88.0,12.3
1705309204,51.5078,-0.1270,90.0,12.5
1705309205,51.5079,-0.1268,90.5,12.6
"""

builder = (
    NavFileBuilder().with_title("Imported from CSV").with_device("CSV importer v1.0")
)

reader = csv.reader(io.StringIO(CSV_DATA))
next(reader)  # skip the header row

rows = 0
for row in reader:
    if not row:
        continue
    ts, lat, lon, heading, speed = row
    builder.add(
        NavFix(
            lat=float(lat),
            lon=float(lon),
            gps_time=datetime.fromtimestamp(int(ts), tz=UTC),
            heading=float(heading),
            speed_mps=float(speed),
        )
    )
    rows += 1

nav_file = builder.finish()

out = Path(tempfile.gettempdir()) / "from_csv.gtd"
nav_file.write_to_file(out)
print(f"Parsed {rows} CSV rows into {len(nav_file.points)} nav points -> {out}")

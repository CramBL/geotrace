#!/usr/bin/env python3
"""Read and print a summary of a .gtd file.

Usage:
    python read_file.py <path-to-file.gtd>

If no path is supplied the script first runs write_basic.py to produce a
temp file, then reads that.
"""

from __future__ import annotations

import subprocess
import sys
import tempfile
from pathlib import Path

from geotrace_sdk import NavFile


def summarise(path: Path) -> None:
    nav_file = NavFile.open(path)

    print(f"File : {path}")
    print(f"Points  : {len(nav_file.points)}")
    print(f"Markers : {len(nav_file.markers)}")

    if nav_file.points:
        print()
        print("Nav points:")
        for i, pt in enumerate(nav_file.points):
            ts = str(pt.gps_time) if pt.gps_time else "(no gps time)"
            heading = f"{pt.heading:.1f}°" if pt.heading is not None else "—"
            speed = f"{pt.speed_mps:.1f} m/s" if pt.speed_mps is not None else "—"
            sats = (
                f"{len(pt.satellites.tracked)} sats"
                if pt.satellites is not None
                else "no sat data"
            )
            print(
                f"  [{i}] {pt.lat:.5f}, {pt.lon:.5f}  "
                f"heading={heading}  speed={speed}  {ts}  {sats}"
            )

    if nav_file.markers:
        print()
        print("Markers:")
        for m in nav_file.markers:
            label = m.annotation.label or "(no label)"
            print(f"  {label}  @ {m.lat:.5f}, {m.lon:.5f}")


if __name__ == "__main__":
    if len(sys.argv) >= 2:
        summarise(Path(sys.argv[1]))
    else:
        demo_path = Path(tempfile.gettempdir()) / "write_basic.gtd"
        if not demo_path.exists():
            script = Path(__file__).parent / "write_basic.py"
            subprocess.run([sys.executable, str(script)], check=True)
        summarise(demo_path)

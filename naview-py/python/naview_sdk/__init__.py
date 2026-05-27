"""Python bindings for naview-sdk — read and write .nvd navigation data files.

Quick start — writing a track::

    from datetime import datetime, timezone
    from naview_sdk import NavFileBuilder, NavFix

    builder = NavFileBuilder()
    builder.add_nav_fix(NavFix(
        lat=51.5074,
        lon=-0.1278,
        gps_time=datetime(2024, 1, 15, 9, 0, 0, tzinfo=timezone.utc),
        heading=90.0,
    ))
    nav_file = builder.finish()
    nav_file.write_to_file("track.nvd")

Quick start — reading a file::

    from naview_sdk import NavFile

    nav_file = NavFile.open("track.nvd")
    for point in nav_file.points:
        print(point.lat, point.lon, point.gps_time)
"""

from naview_sdk._naview_sdk import (
    Annotation,
    Constellation,
    EventMarker,
    EventMarkerPoint,
    EventMarkerStyle,
    Marker,
    MarkerIcon,
    Meta,
    NavFile,
    NavFileBuilder,
    NavFix,
    NavPoint,
    Satellite,
    SatelliteReport,
)

__all__ = [
    "Annotation",
    "Constellation",
    "EventMarker",
    "EventMarkerPoint",
    "EventMarkerStyle",
    "Marker",
    "MarkerIcon",
    "Meta",
    "NavFile",
    "NavFileBuilder",
    "NavFix",
    "NavPoint",
    "Satellite",
    "SatelliteReport",
]

"""Python bindings for geotrace-sdk - read and write .gtd navigation data files.

Quick start - writing a track::

    from datetime import datetime, timezone
    from geotrace_sdk import NavFileBuilder, NavFix

    builder = NavFileBuilder()
    builder.add(NavFix(
        lat=51.5074,
        lon=-0.1278,
        gps_time=datetime(2024, 1, 15, 9, 0, 0, tzinfo=timezone.utc),
        heading=90.0,
    ))
    nav_file = builder.finish()
    nav_file.write_to_file("track.gtd")

Quick start - reading a file::

    from geotrace_sdk import NavFile

    nav_file = NavFile.open("track.gtd")
    for point in nav_file.points:
        print(point.lat, point.lon, point.gps_time)
"""

from importlib.metadata import PackageNotFoundError
from importlib.metadata import version as _dist_version

from geotrace_sdk._geotrace_sdk import (
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
from geotrace_sdk.event_kind import event_kind

try:
    __version__ = _dist_version("geotrace-sdk")
except PackageNotFoundError:  # running from a source tree, not an installed dist
    __version__ = "0.0.0+unknown"

__all__ = [
    "__version__",
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
    "event_kind",
]

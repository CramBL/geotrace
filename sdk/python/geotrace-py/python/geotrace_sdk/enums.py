"""The enumerations the SDK exchanges with the extension module."""

from __future__ import annotations

from enum import Enum

__all__ = ["Constellation", "MarkerIcon", "TravelMode"]


class Constellation(Enum):
    """GNSS constellation identifier."""

    GPS = 0
    GLONASS = 1
    GALILEO = 2
    BEIDOU = 3
    NAVIC = 4
    QZSS = 5


class MarkerIcon(Enum):
    """Visual icon for a map annotation marker."""

    PIN = 0
    CROSS = 1
    CIRCLE = 2
    LIGHTNING = 3
    WARNING = 4
    ERROR = 5
    CHECK = 6
    SATELLITE = 7
    SATELLITE_LOST = 8
    GEAR = 9
    REFRESH = 10
    DOWNLOAD = 11
    UPLOAD = 12
    WRENCH = 13


class TravelMode(Enum):
    """Platform a recording was made on, declared by the recorder."""

    CAR = 0
    MOTORCYCLE = 1
    BICYCLE = 2
    PEDESTRIAN = 3
    BOAT = 4
    RAIL = 5
    AIRCRAFT = 6

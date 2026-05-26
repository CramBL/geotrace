"""Type stubs for the naview-sdk Python package."""

from __future__ import annotations

from datetime import datetime
from enum import IntEnum
from os import PathLike
from typing import Union, final

StrPath = Union[str, bytes, PathLike[str]]



@final
class Constellation(IntEnum):
    """GNSS constellation identifier."""

    GPS = 0
    GLONASS = 1
    GALILEO = 2
    BEIDOU = 3



@final
class MarkerIcon(IntEnum):
    """Visual icon for a map annotation marker."""

    PIN = 0
    CROSS = 1
    CIRCLE = 2
    LIGHTNING = 3
    WARNING = 4
    ERROR = 5
    CHECK = 6



@final
class Satellite:
    """One tracked satellite with optional signal metrics.

    Args:
        constellation: GNSS constellation.
        prn: Satellite PRN number.
        in_fix: Whether this satellite is contributing to the current fix.
        elevation: Elevation above horizon in degrees, or ``None``.
        azimuth: Azimuth from true north in degrees, or ``None``.
        snr: Signal-to-noise ratio in dB-Hz, or ``None``.
    """

    def __init__(
        self,
        constellation: Constellation,
        prn: int,
        *,
        in_fix: bool = False,
        elevation: float | None = None,
        azimuth: float | None = None,
        snr: float | None = None,
    ) -> None: ...

    @property
    def constellation(self) -> Constellation: ...

    @property
    def prn(self) -> int: ...

    @property
    def in_fix(self) -> bool: ...

    @property
    def elevation(self) -> float | None:
        """Elevation above horizon in degrees."""
        ...

    @property
    def azimuth(self) -> float | None:
        """Azimuth from true north in degrees."""
        ...

    @property
    def snr(self) -> float | None:
        """Signal-to-noise ratio in dB-Hz."""
        ...

    def __eq__(self, other: object) -> bool: ...



@final
class SatelliteReport:
    """A set of satellites tracked at a point in time.

    Supply at least one of ``gps_time`` or ``sys_time``.
    Both must be timezone-aware :class:`datetime.datetime` objects.

    Args:
        tracked: All satellites currently tracked.
        gps_time: GPS-domain timestamp; present when the receiver had an active fix.
        sys_time: System-clock timestamp at capture time.
    """

    def __init__(
        self,
        tracked: list[Satellite],
        *,
        gps_time: datetime | None = None,
        sys_time: datetime | None = None,
    ) -> None: ...

    @property
    def tracked(self) -> list[Satellite]: ...

    @property
    def gps_time(self) -> datetime | None:
        """GPS-domain timestamp (timezone-aware UTC), or ``None``."""
        ...

    @property
    def sys_time(self) -> datetime | None:
        """System-clock timestamp (timezone-aware UTC), or ``None``."""
        ...

    def __eq__(self, other: object) -> bool: ...



@final
class NavFix:
    """A single GPS/GNSS fix: position, optional heading, and optional speed.

    Provide at least one of ``gps_time`` or ``sys_time``.
    All :class:`datetime.datetime` arguments must be timezone-aware.

    Args:
        lat: Latitude in degrees.
        lon: Longitude in degrees.
        gps_time: GPS-receiver timestamp; ``None`` when the receiver had no lock.
        sys_time: System-clock timestamp recorded alongside this fix.
        heading: Compass heading in degrees [0, 360); ``None`` = unknown.
        speed_mps: Speed in m/s, or ``None``.
        eph_m: Estimated horizontal accuracy radius in metres, or ``None``.
    """

    def __init__(
        self,
        lat: float,
        lon: float,
        *,
        gps_time: datetime | None = None,
        sys_time: datetime | None = None,
        heading: float | None = None,
        speed_mps: float | None = None,
        eph_m: float | None = None,
    ) -> None: ...

    @property
    def lat(self) -> float:
        """Latitude in degrees."""
        ...

    @property
    def lon(self) -> float:
        """Longitude in degrees."""
        ...

    @property
    def gps_time(self) -> datetime | None:
        """GPS-domain timestamp (timezone-aware UTC), or ``None``."""
        ...

    @property
    def sys_time(self) -> datetime | None:
        """System-clock timestamp (timezone-aware UTC), or ``None``."""
        ...

    @property
    def heading(self) -> float | None:
        """Compass heading in degrees [0, 360), or ``None``."""
        ...

    @property
    def speed_mps(self) -> float | None:
        """Speed in m/s, or ``None``."""
        ...

    @property
    def eph_m(self) -> float | None:
        """Estimated horizontal accuracy radius in metres, or ``None``."""
        ...

    def __eq__(self, other: object) -> bool: ...



@final
class Annotation:
    """A user-defined map annotation with an optional label and icon.

    Args:
        time: Timezone-aware timestamp.
        label: Display label; ``None`` renders as unlabelled.
        icon: Visual icon; ``None`` defaults to :attr:`MarkerIcon.PIN`.
    """

    def __init__(
        self,
        time: datetime,
        *,
        label: str | None = None,
        icon: MarkerIcon | None = None,
    ) -> None: ...

    @property
    def time(self) -> datetime:
        """Timestamp (timezone-aware UTC)."""
        ...

    @property
    def label(self) -> str | None: ...

    @property
    def icon(self) -> MarkerIcon | None: ...

    def __eq__(self, other: object) -> bool: ...



@final
class Meta:
    """Optional file-level metadata for a ``.nvd`` file.

    Args:
        title: File title, or ``None``.
        device: Sensor or device that produced the data, or ``None``.
        notes: Free-text notes, or ``None``.
    """

    def __init__(
        self,
        *,
        title: str | None = None,
        device: str | None = None,
        notes: str | None = None,
    ) -> None: ...

    @property
    def title(self) -> str | None: ...

    @property
    def device(self) -> str | None: ...

    @property
    def notes(self) -> str | None: ...

    def __eq__(self, other: object) -> bool: ...



@final
class NavPoint:
    """A nav fix combined with its associated satellite report, as read from a file."""

    @property
    def lat(self) -> float:
        """Latitude in degrees."""
        ...

    @property
    def lon(self) -> float:
        """Longitude in degrees."""
        ...

    @property
    def gps_time(self) -> datetime | None:
        """GPS-domain timestamp (timezone-aware UTC), or ``None``."""
        ...

    @property
    def sys_time(self) -> datetime | None:
        """System-clock timestamp (timezone-aware UTC), or ``None``."""
        ...

    @property
    def heading(self) -> float | None:
        """Compass heading in degrees [0, 360), or ``None``."""
        ...

    @property
    def speed_mps(self) -> float | None:
        """Speed in m/s, or ``None``."""
        ...

    @property
    def eph_m(self) -> float | None:
        """Estimated horizontal accuracy radius in metres, or ``None``."""
        ...

    @property
    def satellites(self) -> SatelliteReport | None:
        """Associated satellite report, or ``None`` if none was recorded."""
        ...



@final
class Marker:
    """A map annotation with its interpolated position on the nav track."""

    @property
    def lat(self) -> float:
        """Interpolated latitude in degrees."""
        ...

    @property
    def lon(self) -> float:
        """Interpolated longitude in degrees."""
        ...

    @property
    def annotation(self) -> Annotation: ...

    @property
    def label(self) -> str | None:
        """Display label from the annotation, or ``None``."""
        ...

    @property
    def icon(self) -> MarkerIcon | None:
        """Visual icon from the annotation, or ``None``."""
        ...

    @property
    def time(self) -> datetime:
        """Annotation timestamp (timezone-aware UTC)."""
        ...



@final
class NavFile:
    """A parsed ``.nvd`` navigation data file.

    Construct via :meth:`NavFileBuilder.finish` to write, or
    :meth:`NavFile.open` to read.
    """

    @staticmethod
    def open(path: StrPath) -> NavFile:
        """Open and parse a ``.nvd`` file at *path*."""
        ...

    def write_to_file(self, path: StrPath) -> None:
        """Write this file to *path*. Appends ``.nvd`` if *path* has no extension."""
        ...

    def to_bytes(self) -> bytes:
        """Serialise the file to a ``bytes`` object."""
        ...

    @property
    def meta(self) -> Meta:
        """File-level metadata."""
        ...

    @property
    def points(self) -> list[NavPoint]:
        """All nav points in chronological order."""
        ...

    @property
    def markers(self) -> list[Marker]:
        """All map markers with their interpolated positions."""
        ...



@final
class NavFileBuilder:
    """Assembles nav fixes, satellite reports, and annotations into a :class:`NavFile`.

    All mutating methods return ``self`` to allow chaining::

        nav_file = (
            NavFileBuilder()
            .set_meta(Meta(title="My Track"))
            .add(NavFix(lat=51.5, lon=-0.1, gps_time=...))
            .finish()
        )
        nav_file.write_to_file("track.nvd")

    Calling :meth:`finish` consumes the builder; further method calls raise
    :class:`RuntimeError`.
    """

    def __init__(self) -> None: ...

    def set_meta(self, meta: Meta) -> NavFileBuilder:
        """Attach file-level metadata. Returns ``self``."""
        ...

    def add(self, item: NavFix | SatelliteReport | Annotation) -> NavFileBuilder:
        """Add a nav fix, satellite report, or annotation. Returns ``self``."""
        ...

    def finish(self) -> NavFile:
        """Process all data and return a :class:`NavFile`.

        Raises:
            RuntimeError: If called more than once.
            ValueError: If the data is invalid (e.g., no nav fixes provided).
        """
        ...

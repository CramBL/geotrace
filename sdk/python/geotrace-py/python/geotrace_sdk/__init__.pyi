"""Type stubs for the geotrace-sdk Python package."""

from __future__ import annotations

from datetime import datetime
from enum import Enum
from os import PathLike
from typing import Any, final

StrPath = str | bytes | PathLike[str]

__version__: str

@final
class Constellation(Enum):
    """GNSS constellation identifier."""

    GPS = 0
    GLONASS = 1
    GALILEO = 2
    BEIDOU = 3
    NAVIC = 4
    QZSS = 5

@final
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
        gps_time: GPS-domain timestamp, present when the receiver had an active fix.
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
class ChannelUnit:
    """A recognized channel unit or an explicit display-only custom unit."""

    @staticmethod
    def recognized(label: str) -> ChannelUnit:
        """Parse a unit GeoTrace understands and can scale in queries."""
        ...

    @staticmethod
    def custom(label: str) -> ChannelUnit:
        """Construct a display-only unit treated as dimensionless in queries."""
        ...

    @property
    def label(self) -> str: ...
    @property
    def is_custom(self) -> bool: ...
    def __str__(self) -> str: ...
    def __repr__(self) -> str: ...

@final
class Channel:
    """A named scalar or vector sensor channel sampled at its own rate.

    Pass ``components`` for a vector channel (one column per component) or omit
    it for a scalar channel. ``values`` is row-major: ``len(times)`` rows of one
    column (scalar) or ``len(components)`` columns (vector). ``times`` must be
    timezone-aware :class:`datetime.datetime` objects.

    Args:
        name: Channel identifier (a lowercase identifier), referenced as ``@name``.
        times: Sample timestamps, one per row of ``values``.
        values: Row-major sample values.
        unit: Recognized unit string, :class:`ChannelUnit`, or ``None``.
        period_deg: Wrap period in degrees for an angular channel, or ``None``.
        description: Human description, or ``None``.
        components: Vector component labels, or ``None`` for a scalar channel.

    Raises:
        ValueError: If the name or a component label is malformed, or ``values``
            is not ``len(times) * max(len(components), 1)`` long.
    """

    def __init__(
        self,
        name: str,
        times: list[datetime],
        values: list[float],
        *,
        unit: str | ChannelUnit | None = None,
        period_deg: float | None = None,
        description: str | None = None,
        components: list[str] | None = None,
    ) -> None: ...
    @property
    def name(self) -> str: ...
    @property
    def unit(self) -> ChannelUnit | None: ...
    @property
    def period_deg(self) -> float | None: ...
    @property
    def description(self) -> str | None: ...
    @property
    def components(self) -> list[str]: ...
    @property
    def is_vector(self) -> bool: ...
    @property
    def times(self) -> list[datetime]: ...
    @property
    def values(self) -> list[float]: ...
    def __eq__(self, other: object) -> bool: ...

@final
class NavFix:
    """A single GPS/GNSS fix: position, optional heading, and optional speed.

    Provide at least one of ``gps_time`` or ``sys_time``.
    All :class:`datetime.datetime` arguments must be timezone-aware.

    Args:
        lat: Latitude in degrees.
        lon: Longitude in degrees.
        gps_time: GPS-receiver timestamp, or ``None`` when the receiver had no lock.
        sys_time: System-clock timestamp recorded alongside this fix.
        heading: Compass heading in degrees [0, 360), or ``None`` if unknown.
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
        label: Display label, or ``None`` to render as unlabelled.
        icon: Visual icon, or ``None`` to default to :attr:`MarkerIcon.PIN`.
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
    """Optional file-level metadata for a ``.gtd`` file.

    Args:
        title: File title, or ``None``.
        device: Sensor or device that produced the data, or ``None``.
        notes: Free-text notes, or ``None``.
        identity: Opaque producer identity string, or ``None``.
    """

    def __init__(
        self,
        *,
        title: str | None = None,
        device: str | None = None,
        notes: str | None = None,
        identity: str | None = None,
    ) -> None: ...
    @property
    def title(self) -> str | None: ...
    @property
    def device(self) -> str | None: ...
    @property
    def notes(self) -> str | None: ...
    @property
    def identity(self) -> str | None: ...
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
class EventMarker:
    """An event marker to add to the nav track.

    Args:
        variant_path: Slash-separated path, e.g. ``"power/boot"``, or ``None`` /
            ``event_kind.skip`` to silently skip this marker.
        sys_time: Timezone-aware timestamp for this event.
        annotation: Optional free-text note shown on hover.
    """

    def __init__(
        self,
        variant_path: str | _SkipSentinel | None,
        sys_time: datetime,
        *,
        annotation: str | None = None,
    ) -> None: ...
    @property
    def variant_path(self) -> str | None: ...
    @property
    def sys_time(self) -> datetime: ...
    @property
    def annotation(self) -> str | None: ...

@final
class EventMarkerStyle:
    """Per-variant icon and color style stored in the file.

    Args:
        variant_path: Must exactly match a ``variant_path`` used in an event marker.
        icon: Icon shape, or ``None`` for the application default (Pin).
        color: Fill color as ``#RRGGBB``, or ``None`` for the deterministic hash color.
    """

    def __init__(
        self,
        variant_path: str,
        *,
        icon: MarkerIcon | None = None,
        color: str | None = None,
    ) -> None: ...
    @property
    def variant_path(self) -> str: ...
    @property
    def icon(self) -> MarkerIcon | None:
        """Icon shape, or ``None`` for the default."""
        ...

    @property
    def color(self) -> str | None:
        """Fill color as ``#RRGGBB``, or ``None`` for the hash-derived color."""
        ...

@final
class EventMarkerPoint:
    """A resolved event marker as read from a :class:`NavFile`.

    Includes an interpolated position.
    """

    @property
    def variant_path(self) -> str: ...
    @property
    def sys_time(self) -> datetime: ...
    @property
    def lat(self) -> float:
        """Interpolated latitude in degrees."""
        ...

    @property
    def lon(self) -> float:
        """Interpolated longitude in degrees."""
        ...

    @property
    def annotation(self) -> str | None: ...

@final
class NavFile:
    """A parsed ``.gtd`` navigation data file.

    Construct via :meth:`NavFileBuilder.finish` to write, or
    :meth:`NavFile.open` to read.
    """

    @staticmethod
    def open(path: StrPath) -> NavFile:
        """Open and parse a ``.gtd`` file at *path*."""
        ...

    @staticmethod
    def from_bytes(data: bytes) -> NavFile:
        """Parse a ``.gtd`` file from raw bytes."""
        ...

    def write_to_file(self, path: StrPath) -> None:
        """Write this file to *path*. Appends ``.gtd`` if *path* has no extension."""
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

    @property
    def event_markers(self) -> list[EventMarkerPoint]:
        """All event markers with their interpolated positions."""
        ...

    @property
    def event_marker_styles(self) -> list[EventMarkerStyle]:
        """Per-variant style overrides stored in the file."""
        ...

    @property
    def channels(self) -> list[Channel]:
        """All ad-hoc sensor channels, sorted by name."""
        ...

@final
class NavFileBuilder:
    """Assembles nav fixes, satellite reports, and annotations into a :class:`NavFile`.

    All mutating methods return ``self`` to allow chaining::

        nav_file = (
            NavFileBuilder()
            .with_meta(Meta(title="My Track"))
            .add(NavFix(lat=51.5, lon=-0.1, gps_time=...))
            .finish()
        )
        nav_file.write_to_file("track.gtd")

    Calling :meth:`finish` consumes the builder. Further method calls raise
    :class:`RuntimeError`.
    """

    def __init__(self) -> None: ...
    def with_meta(self, meta: Meta) -> NavFileBuilder:
        """Attach file-level metadata.

        Must be called before ``add()``. Returns ``self``.
        """
        ...

    def with_title(self, title: str) -> NavFileBuilder:
        """Set the file title. Must be called before ``add()``. Returns ``self``."""
        ...

    def with_device(self, device: str) -> NavFileBuilder:
        """Set the device or sensor name.

        Must be called before ``add()``. Returns ``self``.
        """
        ...

    def with_notes(self, notes: str) -> NavFileBuilder:
        """Set free-text notes. Must be called before ``add()``. Returns ``self``."""
        ...

    def add(
        self, item: NavFix | SatelliteReport | Annotation | EventMarker | Channel
    ) -> NavFileBuilder:
        """Add a nav fix, satellite report, annotation, event marker, or channel.

        Returns ``self``.

        Passing an :class:`EventMarker` whose ``variant_path`` is ``None`` or
        ``event_kind.skip`` is a silent no-op.
        """
        ...

    def add_event_marker_style(self, style: EventMarkerStyle) -> NavFileBuilder:
        """Add a per-variant style override. Returns ``self``."""
        ...

    def finish(self) -> NavFile:
        """Process all data and return a :class:`NavFile`.

        Raises:
            RuntimeError: If called more than once.
            ValueError: If the data is invalid (e.g., no nav fixes provided).
        """
        ...

class _EventKindNamespace:
    """Resolved event-kind namespace. Attributes are strings or nested namespaces."""

    def __getattr__(self, name: str) -> Any: ...
    def all_paths(self) -> list[str]:
        """Return all non-skip leaf path strings, sorted."""
        ...

class _SkipSentinel:
    """Sentinel that marks an event-kind attribute as skipped."""

    ...

@final
class _EventKindDecorator:
    """Class decorator that converts each attribute to its snake_case event path string.

    Attributes in the class body become path strings, and inner classes become
    nested namespaces.
    An attribute set to ``event_kind.skip`` returns the skip sentinel. Passing it to
    :class:`EventMarker` or :meth:`NavFileBuilder.add` is a silent no-op.

    Example::

        @event_kind
        class Event:
            boot = None
            battery_low = None

            class Connectivity:
                class Agps:
                    request = None

        assert Event.boot == "boot"
        assert Event.Connectivity.Agps.request == "connectivity/agps/request"
    """

    def __call__(self, cls: type) -> _EventKindNamespace: ...
    @property
    def skip(self) -> _SkipSentinel: ...

event_kind: _EventKindDecorator

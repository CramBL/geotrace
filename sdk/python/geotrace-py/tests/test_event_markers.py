"""Tests for EventMarker, EventMarkerStyle, and the @event_kind decorator."""

from __future__ import annotations

from datetime import UTC, datetime
from pathlib import Path

import pytest
from geotrace_sdk import (
    EventMarker,
    EventMarkerPoint,
    EventMarkerStyle,
    MarkerIcon,
    NavFile,
    NavFileBuilder,
    NavFix,
    event_kind,
)

T0 = datetime(2024, 6, 1, 9, 0, 0, tzinfo=UTC)
T1 = datetime(2024, 6, 1, 9, 1, 0, tzinfo=UTC)
T2 = datetime(2024, 6, 1, 9, 2, 0, tzinfo=UTC)


def _builder_with_fixes() -> NavFileBuilder:
    b = NavFileBuilder()
    b.add(NavFix(lat=51.5, lon=-0.1, gps_time=T0))
    b.add(NavFix(lat=51.51, lon=-0.11, gps_time=T2))
    return b


# EventMarker construction


def test_event_marker_string_path() -> None:
    em = EventMarker("power/boot", T0)
    assert em.variant_path == "power/boot"
    assert em.annotation is None


def test_event_marker_with_annotation() -> None:
    em = EventMarker("power/boot", T0, annotation="cold start")
    assert em.annotation == "cold start"


def test_event_marker_none_path() -> None:
    em = EventMarker(None, T0)
    assert em.variant_path is None


def test_event_marker_skip_sentinel_converts_to_none() -> None:
    em = EventMarker(event_kind.skip, T0)
    assert em.variant_path is None


def test_event_marker_invalid_path_raises_at_construction() -> None:
    with pytest.raises(ValueError):
        EventMarker("/bad/path", T1)


def test_event_marker_is_true_class() -> None:
    em = EventMarker("power/boot", T0)
    assert isinstance(em, EventMarker)


# add() dispatch for EventMarker


def test_add_dispatch_event_marker() -> None:
    b = _builder_with_fixes()
    b.add(EventMarker("power/boot", T1))
    f = b.finish()
    assert len(f.event_markers) == 1
    assert f.event_markers[0].variant_path == "power/boot"


def test_add_dispatch_skip_sentinel_is_noop() -> None:
    b = _builder_with_fixes()
    b.add(EventMarker(event_kind.skip, T1))
    f = b.finish()
    assert len(f.event_markers) == 0


def test_add_dispatch_none_path_is_noop() -> None:
    b = _builder_with_fixes()
    b.add(EventMarker(None, T1))
    f = b.finish()
    assert len(f.event_markers) == 0


def test_add_dispatch_invalid_path_raises() -> None:
    with pytest.raises(ValueError):
        EventMarker("//bad", T1)


# EventMarkerPoint read-back


def test_event_marker_point_has_interpolated_position() -> None:
    b = NavFileBuilder()
    b.add(NavFix(lat=10.0, lon=20.0, gps_time=T0))
    b.add(NavFix(lat=12.0, lon=24.0, gps_time=T2))
    b.add(EventMarker("test/mid", T1))
    f = b.finish()

    em = f.event_markers[0]
    assert isinstance(em, EventMarkerPoint)
    assert abs(em.lat - 11.0) < 1e-6
    assert abs(em.lon - 22.0) < 1e-6


def test_event_marker_point_annotation_preserved() -> None:
    b = _builder_with_fixes()
    b.add(EventMarker("power/boot", T1, annotation="cold start"))
    f = b.finish()
    assert f.event_markers[0].annotation == "cold start"


# EventMarkerStyle


def test_event_marker_style_auto_defaults() -> None:
    b = _builder_with_fixes()
    b.add(EventMarker("power/boot", T1))
    b.add_event_marker_style(EventMarkerStyle("power/boot"))
    f = b.finish()
    styles = f.event_marker_styles
    assert len(styles) == 1
    assert styles[0].icon is None
    assert styles[0].color is None


def test_event_marker_style_explicit_icon_and_color() -> None:
    b = _builder_with_fixes()
    b.add(EventMarker("power/boot", T1))
    b.add_event_marker_style(
        EventMarkerStyle("power/boot", icon=MarkerIcon.LIGHTNING, color="#44BB44")
    )
    f = b.finish()
    s = f.event_marker_styles[0]
    assert s.icon == MarkerIcon.LIGHTNING
    assert s.color == "#44BB44"


def test_event_marker_style_round_trips(tmp_path: Path) -> None:
    b = _builder_with_fixes()
    b.add(EventMarker("sensor/gps", T1))
    b.add_event_marker_style(
        EventMarkerStyle("sensor/gps", icon=MarkerIcon.CHECK, color="#FF0000")
    )
    f = b.finish()
    path = tmp_path / "style_roundtrip.gtd"
    f.write_to_file(path)
    from geotrace_sdk import NavFile

    loaded = NavFile.open(path)
    s = loaded.event_marker_styles[0]
    assert s.icon == MarkerIcon.CHECK
    assert s.color == "#FF0000"


UNRECOGNIZED_STYLE_FIXTURE = (
    Path(__file__).resolve().parents[3]
    / "c"
    / "tests"
    / "fixtures"
    / "unrecognized_style_values.gtd"
)


def test_style_icon_outside_the_known_set_reads_as_none_with_a_warning() -> None:
    loaded = NavFile.open(UNRECOGNIZED_STYLE_FIXTURE)
    with pytest.warns(UserWarning, match="hovercraft"):
        style = loaded.event_marker_styles[0]
    assert style.icon is None


def test_style_icon_name_holds_a_name_outside_the_known_set() -> None:
    loaded = NavFile.open(UNRECOGNIZED_STYLE_FIXTURE)
    with pytest.warns(UserWarning, match="hovercraft"):
        style = loaded.event_marker_styles[0]
    assert style.icon_name == "hovercraft"


@pytest.mark.parametrize(
    ("icon", "expected"), [(MarkerIcon.SATELLITE_LOST, "satellite_lost"), (None, None)]
)
def test_style_icon_name_is_the_wire_name_of_its_icon(
    icon: MarkerIcon | None, expected: str | None
) -> None:
    assert EventMarkerStyle("power/boot", icon=icon).icon_name == expected


def test_style_color_outside_the_known_form_survives_the_read() -> None:
    loaded = NavFile.open(UNRECOGNIZED_STYLE_FIXTURE)
    with pytest.warns(UserWarning):
        style = loaded.event_marker_styles[0]
    assert style.color == "FFAA00"


def test_style_color_outside_the_known_form_is_rejected_when_written_back() -> None:
    loaded = NavFile.open(UNRECOGNIZED_STYLE_FIXTURE)
    with pytest.warns(UserWarning):
        style = loaded.event_marker_styles[0]

    b = _builder_with_fixes()
    with pytest.raises(ValueError, match="#RRGGBB"):
        b.add_event_marker_style(style)


def test_style_variant_path_past_the_field_capacity_raises_on_write() -> None:
    b = _builder_with_fixes()
    b.add_event_marker_style(EventMarkerStyle("a" * 256))
    with pytest.raises(ValueError, match="event_marker_styles/variant_path"):
        b.finish().to_bytes()


# @event_kind decorator


def test_event_kind_flat_unit_attributes() -> None:
    @event_kind
    class Event:
        boot = None
        battery_low = None
        gps_lock_acquired = None

    assert Event.boot == "boot"
    assert Event.battery_low == "battery_low"
    assert Event.gps_lock_acquired == "gps_lock_acquired"


def test_event_kind_snake_case_conversion() -> None:
    @event_kind
    class Event:
        GPSLock = None
        BatteryLow = None

    assert Event.GPSLock == "gps_lock"
    assert Event.BatteryLow == "battery_low"


def test_event_kind_nested_three_levels() -> None:
    @event_kind
    class Event:
        class Connectivity:
            class Agps:
                request = None
                success = None

    assert Event.Connectivity.Agps.request == "connectivity/agps/request"
    assert Event.Connectivity.Agps.success == "connectivity/agps/success"


def test_event_kind_mixed_flat_and_nested() -> None:
    @event_kind
    class Event:
        boot = None

        class Sensor:
            gps_lock = None

    assert Event.boot == "boot"
    assert Event.Sensor.gps_lock == "sensor/gps_lock"


def test_event_kind_skip_returns_sentinel() -> None:
    @event_kind
    class Event:
        active = None
        internal = event_kind.skip

    assert Event.active == "active"
    assert Event.internal is event_kind.skip


def test_event_kind_skip_used_as_event_marker_is_noop() -> None:
    @event_kind
    class Event:
        active = None
        internal = event_kind.skip

    b = _builder_with_fixes()
    b.add(EventMarker(Event.internal, T1))
    f = b.finish()
    assert len(f.event_markers) == 0


def test_event_kind_non_skip_adds_marker() -> None:
    @event_kind
    class Event:
        boot = None

    b = _builder_with_fixes()
    b.add(EventMarker(Event.boot, T1))
    f = b.finish()
    assert len(f.event_markers) == 1
    assert f.event_markers[0].variant_path == "boot"


def test_event_kind_nested_used_in_builder() -> None:
    @event_kind
    class Event:
        class Connectivity:
            class Agps:
                request = None

    b = _builder_with_fixes()
    b.add(EventMarker(Event.Connectivity.Agps.request, T1))
    f = b.finish()
    assert f.event_markers[0].variant_path == "connectivity/agps/request"


# all_paths()


def test_all_paths_flat() -> None:
    @event_kind
    class Event:
        boot = None
        shutdown = None
        battery_low = None

    assert Event.all_paths() == ["battery_low", "boot", "shutdown"]  # type: ignore


def test_all_paths_nested() -> None:
    @event_kind
    class Event:
        boot = None

        class Connectivity:
            class Agps:
                request = None
                success = None

    assert Event.all_paths() == [  # type: ignore
        "boot",
        "connectivity/agps/request",
        "connectivity/agps/success",
    ]


def test_all_paths_excludes_skip() -> None:
    @event_kind
    class Event:
        boot = None
        internal = event_kind.skip
        shutdown = None

    assert Event.all_paths() == ["boot", "shutdown"]  # type: ignore

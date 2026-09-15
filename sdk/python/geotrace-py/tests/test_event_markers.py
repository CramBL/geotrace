"""Tests for EventMarker, EventMarkerStyle, and the @event_kind decorator."""

from __future__ import annotations

import re
import tomllib
from datetime import UTC, datetime
from pathlib import Path
from typing import Any

import pytest
from geotrace_sdk import (
    Annotation,
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


def test_event_marker_non_string_path_raises_a_type_error() -> None:
    with pytest.raises(
        TypeError, match="variant_path must be a str, None, or event_kind.skip"
    ):
        EventMarker(12345, T0)  # type: ignore[arg-type]


@pytest.mark.parametrize(
    ("variant_path", "annotation", "message"),
    [
        pytest.param("", None, "path is empty", id="empty path"),
        pytest.param(
            "über_lang", None, "outside ASCII alphanumeric", id="non-ASCII path"
        ),
        pytest.param(
            "a" * 256,
            None,
            "256 bytes, past the 255 bytes the field holds",
            id="path past the capacity",
        ),
        pytest.param(
            "power/boot",
            "a" * 512,
            "512 bytes, past the 511 bytes the field holds",
            id="annotation past the capacity",
        ),
    ],
)
def test_event_marker_raises_at_construction_for_a_value_the_rust_builder_rejects(
    variant_path: str, annotation: str | None, message: str
) -> None:
    with pytest.raises(ValueError, match=message):
        EventMarker(variant_path, T1, annotation=annotation)


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


@pytest.mark.parametrize(("value", "expected"), [("", None), ("   ", "   ")])
def test_a_label_and_an_annotation_read_back_as_built(
    value: str, expected: str | None
) -> None:
    annotation = Annotation(T1, label=value)
    assert annotation.label == expected
    b = _builder_with_fixes()
    b.add(annotation)
    b.add(EventMarker("power/boot", T1, annotation=value))
    built = b.finish()

    for nav_file in (built, NavFile.from_bytes(built.to_bytes())):
        assert nav_file.markers[0].label == expected
        assert nav_file.event_markers[0].annotation == expected


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


def test_a_style_read_from_a_file_is_written_back_verbatim() -> None:
    loaded = NavFile.open(UNRECOGNIZED_STYLE_FIXTURE)
    with pytest.warns(UserWarning):
        style = loaded.event_marker_styles[0]

    b = _builder_with_fixes()
    b.add_event_marker_style(style)
    written_back = NavFile.from_bytes(b.finish().to_bytes())

    with pytest.warns(UserWarning):
        written_back_style = written_back.event_marker_styles[0]
    assert written_back_style.icon_name == "hovercraft"
    assert written_back_style.color == "FFAA00"


def test_a_later_style_for_a_variant_path_replaces_the_earlier_one() -> None:
    b = _builder_with_fixes()
    b.add_event_marker_style(
        EventMarkerStyle("power/boot", icon=MarkerIcon.WARNING, color="#FF9900")
    )
    b.add_event_marker_style(
        EventMarkerStyle("power/boot", icon=MarkerIcon.CHECK, color="#00FF00")
    )
    written = NavFile.from_bytes(b.finish().to_bytes())

    styles = written.event_marker_styles
    assert len(styles) == 1
    assert styles[0].icon == MarkerIcon.CHECK
    assert styles[0].color == "#00FF00"


def test_an_empty_style_color_is_the_hash_color() -> None:
    b = _builder_with_fixes()
    b.add(EventMarker("power/boot", T1))
    b.add_event_marker_style(EventMarkerStyle("power/boot", color=""))
    assert b.finish().event_marker_styles[0].color is None


@pytest.mark.parametrize("color", ["red", "FF9900", "   "])
def test_event_marker_style_raises_at_construction_for_a_color_outside_the_rrggbb_form(
    color: str,
) -> None:
    with pytest.raises(ValueError, match="expected the #RRGGBB form"):
        EventMarkerStyle("power/boot", color=color)


@pytest.mark.parametrize(
    ("variant_path", "message"),
    [
        pytest.param("", "path is empty", id="empty"),
        pytest.param("über_lang", "outside ASCII alphanumeric", id="non-ASCII"),
        pytest.param("power/\0boot", "outside ASCII alphanumeric", id="nul byte"),
        pytest.param(
            "a" * 256,
            "256 bytes, past the 255 bytes the field holds",
            id="past the capacity",
        ),
    ],
)
def test_event_marker_style_raises_at_construction_for_a_malformed_variant_path(
    variant_path: str, message: str
) -> None:
    with pytest.raises(ValueError, match=message):
        EventMarkerStyle(variant_path)


# @event_kind decorator

SHARED_SEGMENT_TABLE = tomllib.loads(
    (
        Path(__file__).resolve().parents[4]
        / "tests"
        / "fixtures"
        / "event_kind_variant_path_segments.toml"
    ).read_text(encoding="utf-8")
)


def test_event_kind_flat_unit_attributes() -> None:
    @event_kind
    class Event:
        boot = None
        battery_low = None
        gps_lock_acquired = None

    assert Event.boot == "boot"
    assert Event.battery_low == "battery_low"
    assert Event.gps_lock_acquired == "gps_lock_acquired"


def test_event_kind_derives_the_segment_of_each_shared_derived_name() -> None:
    rows = SHARED_SEGMENT_TABLE["derived"]
    namespace = event_kind(
        type("Event", (), dict.fromkeys(row["name"] for row in rows))
    )
    derived = {row["name"]: getattr(namespace, row["name"]) for row in rows}
    assert derived == {row["name"]: row["segment"] for row in rows}


@pytest.mark.parametrize(
    "row", SHARED_SEGMENT_TABLE["rejected"], ids=lambda row: row["compile_fail"]
)
def test_event_kind_raises_for_each_shared_rejected_row(
    row: dict[str, Any],
) -> None:
    with pytest.raises(ValueError, match=re.escape(row["error"])) as raised:
        event_kind(type("Event", (), dict.fromkeys(row["names"])))
    assert all(name in str(raised.value) for name in row["names"])


def test_event_kind_rename_sets_the_segment_of_attributes_and_inner_classes() -> None:
    @event_kind
    class Event:
        Größe = event_kind.rename("groesse")

        @event_kind.rename("radio-scan")
        class Scan:
            GPSLock = None

    assert Event.all_paths() == ["groesse", "radio-scan/gps_lock"]  # type: ignore


def test_event_kind_raises_for_a_rename_to_the_segment_of_another_attribute() -> None:
    with pytest.raises(ValueError, match="both have the variant path segment 'boot'"):

        @event_kind
        class Event:
            Boot = None
            ColdStart = event_kind.rename("boot")


@pytest.mark.parametrize(
    ("segment", "error"),
    [("", "is empty"), ("power/boot", "'power/boot' contains '/'")],
)
def test_event_kind_raises_for_a_rename_to_an_invalid_segment(
    segment: str, error: str
) -> None:
    with pytest.raises(ValueError, match=f"Boot: variant path segment {error}"):

        @event_kind
        class Event:
            Boot = event_kind.rename(segment)


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

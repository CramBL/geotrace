"""Enum tests: the published enums and the boundary that converts them."""

from __future__ import annotations

from collections.abc import Callable
from datetime import UTC, datetime

import pytest
from geotrace_sdk import (
    Annotation,
    Constellation,
    MarkerIcon,
    Meta,
    Satellite,
    TravelMode,
    constellation_from_name,
    marker_icon_from_name,
)

T0 = datetime(2024, 6, 1, 9, 0, 0, tzinfo=UTC)


@pytest.mark.parametrize("member", list(Constellation))
def test_every_constellation_crosses_the_boundary(member: Constellation) -> None:
    assert Satellite(member, 12).constellation is member


@pytest.mark.parametrize("member", list(MarkerIcon))
def test_every_marker_icon_crosses_the_boundary(member: MarkerIcon) -> None:
    assert Annotation(T0, icon=member).icon is member


@pytest.mark.parametrize("member", list(Constellation))
def test_every_constellation_parses_from_its_wire_name(member: Constellation) -> None:
    assert constellation_from_name(member.name.lower()) is member


@pytest.mark.parametrize("member", list(MarkerIcon))
def test_every_marker_icon_parses_from_its_wire_name(member: MarkerIcon) -> None:
    assert marker_icon_from_name(member.name.lower()) is member


@pytest.mark.parametrize(
    ("parse", "name"),
    [(constellation_from_name, "pulsar"), (marker_icon_from_name, "compass")],
)
def test_a_wire_name_outside_the_set_raises(
    parse: Callable[[str], object], name: str
) -> None:
    with pytest.raises(ValueError, match=name):
        parse(name)


@pytest.mark.parametrize("member", list(TravelMode))
def test_every_travel_mode_crosses_the_boundary(member: TravelMode) -> None:
    assert Meta(travel_mode=member).travel_mode is member


def test_a_member_has_its_name_and_value() -> None:
    assert Constellation.GALILEO.name == "GALILEO"
    assert Constellation.GALILEO.value == 2
    assert MarkerIcon.SATELLITE_LOST.name == "SATELLITE_LOST"
    assert MarkerIcon.SATELLITE_LOST.value == 8


def test_a_class_iterates_and_counts_its_members() -> None:
    assert len(Constellation) == 6
    assert len(MarkerIcon) == 14
    assert len(TravelMode) == 7
    assert list(Constellation)[0] is Constellation.GPS


def test_a_member_is_a_set_element_and_a_dict_key() -> None:
    assert Constellation.QZSS in {Constellation.GPS, Constellation.QZSS}
    assert {TravelMode.BICYCLE: "two wheels"}[TravelMode.BICYCLE] == "two wheels"


def test_a_value_outside_the_class_raises() -> None:
    with pytest.raises(ValueError, match="99"):
        Constellation(99)


@pytest.mark.parametrize("outside_the_class", [0, MarkerIcon.PIN])
def test_a_satellite_takes_a_constellation_member_alone(
    outside_the_class: object,
) -> None:
    with pytest.raises(TypeError, match="Constellation"):
        Satellite(outside_the_class, 12)  # type: ignore[arg-type]

"""``event_kind`` - class decorator for type-safe event-kind derivation."""

from __future__ import annotations

from dataclasses import dataclass

__all__ = ["event_kind"]

_SET_A_SEGMENT = 'set another with event_kind.rename("<segment>")'
_VARIANT_PATH_CAPACITY_BYTES = 255


class _Skip:
    """Sentinel value returned by skipped event-kind attributes."""

    __slots__ = ()

    def __repr__(self) -> str:
        return "event_kind.skip"


_SKIP = _Skip()


@dataclass(frozen=True, slots=True)
class _Rename:
    """The variant path segment set with ``event_kind.rename`` for an attribute, or
    for the inner class it decorates."""

    segment: str
    namespace_class: type | None = None

    def __call__(self, namespace_class: type) -> _Rename:
        return _Rename(self.segment, namespace_class)


class _Namespace:
    """Resolved event-kind namespace.

    Attributes are path strings or nested namespaces.
    """

    def __repr__(self) -> str:
        attrs = {k: v for k, v in self.__dict__.items() if not k.startswith("_")}
        return f"<event_kind namespace {attrs}>"

    def all_paths(self) -> list[str]:
        """Return all non-skip leaf path strings, sorted."""
        result: list[str] = []
        for val in self.__dict__.values():
            if isinstance(val, _Namespace):
                result.extend(val.all_paths())
            elif isinstance(val, str):
                result.append(val)
        return sorted(result)


def _to_snake_case(name: str) -> str:
    """Starts a word at a capital after a lower-case letter or a digit, and at the
    last capital of a run followed by a lower-case letter: ``HTTPError`` gives
    ``http_error``, ``GPS3Lock`` gives ``gps3_lock``. It copies every character
    outside ASCII unchanged.

    ``to_snake_case`` in ``geotrace-sdk-macros`` has the same rule. The tests of
    both SDKs read the names in
    ``tests/fixtures/event_kind_variant_path_segments.toml``.
    """
    snake_case: list[str] = []
    for index, character in enumerate(name):
        if not (character.isascii() and character.isupper()):
            snake_case.append(character)
            continue
        previous = name[index - 1 : index]
        following = name[index + 1 : index + 2]
        if previous.isascii() and (
            previous.islower()
            or previous.isdigit()
            or (previous.isupper() and following.isascii() and following.islower())
        ):
            snake_case.append("_")
        snake_case.append(character.lower())
    return "".join(snake_case)


def _segment_error(segment: str) -> str | None:
    if not segment:
        return "variant path segment is empty"
    length = len(segment.encode())
    if length > _VARIANT_PATH_CAPACITY_BYTES:
        return (
            f"variant path segment is {length} bytes, past the "
            f"{_VARIANT_PATH_CAPACITY_BYTES} bytes a variant path holds"
        )
    invalid_character = next(
        (c for c in segment if not (c.isascii() and (c.isalnum() or c in "-_"))),
        None,
    )
    if invalid_character is not None:
        return (
            f"variant path segment {segment!r} contains {invalid_character!r}, "
            "outside ASCII letters, digits, '-' and '_'"
        )
    return None


def _process(cls: type, prefix: str) -> _Namespace:
    namespace = _Namespace()
    name_by_segment: dict[str, str] = {}
    for name, value in vars(cls).items():
        if name.startswith("_"):
            continue
        if value is _SKIP:
            setattr(namespace, name, _SKIP)
            continue
        if isinstance(value, _Rename):
            segment, hint, value = value.segment, "", value.namespace_class
        else:
            segment, hint = _to_snake_case(name), f": {_SET_A_SEGMENT}"
        error = _segment_error(segment)
        if error is not None:
            raise ValueError(f"{cls.__qualname__}.{name}: {error}{hint}")
        earlier = name_by_segment.setdefault(segment, name)
        if earlier != name:
            raise ValueError(
                f"{cls.__qualname__}.{earlier} and {cls.__qualname__}.{name} both "
                f"have the variant path segment {segment!r}: {_SET_A_SEGMENT}"
            )
        path = f"{prefix}/{segment}" if prefix else segment
        setattr(
            namespace,
            name,
            _process(value, path) if isinstance(value, type) else path,
        )
    return namespace


def event_kind(cls: type) -> _Namespace:
    """Class decorator that converts each attribute to its ``snake_case`` event
    path string.

    Attributes in the class body become path strings, and inner classes become
    nested namespaces. A capital starts a word after a lower-case letter or a
    digit, and so does the last capital of a run before a lower-case letter:
    ``GPS3Lock`` gives ``gps3_lock`` and ``HTTPError`` gives ``http_error``, the
    segments the Rust ``#[derive(EventKind)]`` derives. An attribute set to
    ``event_kind.skip`` returns the skip sentinel value, which
    ``NavFileBuilder.add()`` silently ignores. ``event_kind.rename("<segment>")``
    sets the segment of an attribute, or of the inner class it decorates.

    Raises:
        ValueError: If an attribute's segment has a character outside ASCII
            letters, digits, ``-`` and ``_``, or is past 255 bytes, or if two
            attributes of one class have the same segment. ``EventMarker`` raises
            for a nested path past 255 bytes.

    Example::

        @event_kind
        class Event:
            boot = None
            battery_low = None
            Größe = event_kind.rename("groesse")

            class Connectivity:
                class Agps:
                    request = None
                    success = None

        assert Event.boot == "boot"
        assert Event.battery_low == "battery_low"
        assert Event.Größe == "groesse"
        assert Event.Connectivity.Agps.request == "connectivity/agps/request"
    """
    return _process(cls, "")


event_kind.skip = _SKIP  # type: ignore[attr-defined]
event_kind.rename = _Rename  # type: ignore[attr-defined]

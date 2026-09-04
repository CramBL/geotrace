"""``event_kind`` - class decorator for type-safe event-kind derivation."""

from __future__ import annotations

__all__ = ["event_kind"]


class _Skip:
    """Sentinel value returned by skipped event-kind attributes."""

    __slots__ = ()

    def __repr__(self) -> str:
        return "event_kind.skip"


_SKIP = _Skip()


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
    chars = list(name)
    result: list[str] = []
    for i, c in enumerate(chars):
        if c.isupper():
            if i > 0:
                prev = chars[i - 1]
                next_c = chars[i + 1] if i + 1 < len(chars) else None
                next_lower = next_c is not None and (
                    next_c.islower() or next_c.isdigit()
                )
                if prev.islower() or prev.isdigit() or (prev.isupper() and next_lower):
                    result.append("_")
            result.append(c.lower())
        else:
            result.append(c)
    return "".join(result)


def _process(cls: type, prefix: str) -> _Namespace:
    ns = _Namespace()
    for name, val in vars(cls).items():
        if name.startswith("_"):
            continue
        seg = _to_snake_case(name)
        path = f"{prefix}/{seg}" if prefix else seg
        if val is _SKIP:
            setattr(ns, name, _SKIP)
        elif isinstance(val, type):
            setattr(ns, name, _process(val, path))
        else:
            setattr(ns, name, path)
    return ns


def event_kind(cls: type) -> _Namespace:
    """Class decorator that converts each attribute to its ``snake_case`` event
    path string.

    Attributes in the class body become path strings, and inner classes become
    nested namespaces. An attribute set to ``event_kind.skip`` returns the skip
    sentinel value, which ``NavFileBuilder.add()`` silently ignores.

    Example::

        @event_kind
        class Event:
            boot = None
            battery_low = None

            class Connectivity:
                class Agps:
                    request = None
                    success = None

        assert Event.boot == "boot"
        assert Event.battery_low == "battery_low"
        assert Event.Connectivity.Agps.request == "connectivity/agps/request"
    """
    return _process(cls, "")


event_kind.skip = _SKIP  # type: ignore[attr-defined]

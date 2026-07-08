"""Promote a changelog's unreleased section to a released version.

Turns the unreleased heading into a dated version heading and leaves a fresh
empty unreleased heading on top. The promoted notes become the release body
for the app metadata workflow or the SDK release workflow.

Prerelease suffixes are dropped, so ``0.3.0-rc.1`` promotes ``## [0.3.0]``.
"""

import re
from datetime import date
from pathlib import Path
from typing import Literal

HeadingStyle = Literal["keep_a_changelog", "cargo_dist"]

# A `## [<version>]` or `## <version>` section header.
_SECTION = re.compile(r"^##(?!#)\s+(?:\[([^\]]+)\]|([^ ]+))")


def _core(version: str) -> str:
    return version.split("-", 1)[0].split("+", 1)[0]


def _section_name(line: str) -> str | None:
    match = _SECTION.match(line)
    if not match:
        return None
    name = match.group(1) or match.group(2)
    return name.strip()


def section_exists(text: str, version: str) -> bool:
    """Whether `text` already has a release section for `version`."""
    core = _core(version)
    return any(name == core for name in map(_section_name, text.splitlines()) if name)


def _unreleased_header(style: HeadingStyle) -> str:
    if style == "keep_a_changelog":
        return "## [unreleased]"
    return "## Unreleased"


def _release_header(core: str, today: date, style: HeadingStyle) -> str:
    if style == "keep_a_changelog":
        return f"## [{core}] - {today.isoformat()}"
    return f"## {core} - {today.isoformat()}"


def promote(
    path: Path,
    version: str,
    today: date,
    *,
    heading_style: HeadingStyle = "keep_a_changelog",
) -> bool:
    """Promote `path`'s unreleased section to `version`, dated `today`.

    Returns whether the file changed. No-op if the version's section already
    exists. Raises if there is no unreleased section.
    """
    core = _core(version)
    text = path.read_text(encoding="utf-8")
    if section_exists(text, core):
        return False

    lines = text.splitlines()
    unreleased = next(
        (i for i, line in enumerate(lines) if (_section_name(line) or "").lower() == "unreleased"),
        None,
    )
    if unreleased is None:
        raise SystemExit(f"error: {path}: no unreleased section to promote")

    # Body: everything between the unreleased header and the next section.
    end = next(
        (j for j in range(unreleased + 1, len(lines)) if _section_name(lines[j]) is not None),
        len(lines),
    )
    body = lines[unreleased + 1 : end]
    while body and not body[0].strip():
        body.pop(0)
    while body and not body[-1].strip():
        body.pop()
    if not body:
        print(f"  warning: {path.name}: [unreleased] was empty, {core} will have no notes")

    promoted = [_unreleased_header(heading_style), "", _release_header(core, today, heading_style)]
    if body:
        promoted += ["", *body]
    tail = lines[end:]
    separator = [""] if tail else []
    rebuilt = lines[:unreleased] + promoted + separator + tail
    path.write_text("\n".join(rebuilt) + "\n", encoding="utf-8")
    print(f"  {path.name}: [unreleased] -> {core} - {today.isoformat()}")
    return True

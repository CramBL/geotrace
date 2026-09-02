"""Shared allow-comment parsing for QA checks.

Exemption syntax: // [qa-allow-<check>, reason = "explanation"]
Multiple checks may share one comment:
    // [qa-allow-check-floating-comments, qa-allow-check-narrative-comments, reason = "why"]
"""

import re

_ALLOW = re.compile(r"\[([^\]]+)\]")
_REASON = re.compile(r",\s*reason\s*=\s*\"([^\"]+)\"")


def is_exempt(line: str, check: str) -> bool:
    m = _ALLOW.search(line)
    if not m:
        return False
    block = m.group(1)
    if f"qa-allow-{check}" not in block:
        return False
    return bool(_REASON.search(f",{block}"))

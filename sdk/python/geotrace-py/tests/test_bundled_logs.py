"""Bundled raw-text log tests: construction, validation, and round-trip."""

from __future__ import annotations

from datetime import UTC, datetime
from pathlib import Path

import pytest
from geotrace_sdk import BundledLog, NavFile, NavFileBuilder, NavFix

T0 = datetime(2024, 6, 1, 9, 0, 0, tzinfo=UTC)

SYSLOG_TEXT = "Jun  1 09:00:00 rover gnss[42]: fix acquired\n"
ISO_SPACE_TEXT = "2024-06-01 09:00:00 gnss: tracking 12 satellites\n"
RFC3339_TEXT = "2024-06-01T09:00:00Z service: boot complete\n"


def test_roundtrip_multiple_bundled_logs_sorted_by_name(tmp_path: Path) -> None:
    log_path = tmp_path / "modem.log"
    log_path.write_text(RFC3339_TEXT, encoding="utf-8", newline="\n")

    nav_file = (
        NavFileBuilder()
        .add(NavFix(51.5, -0.1, gps_time=T0))
        .add(BundledLog("syslog.log", SYSLOG_TEXT))
        .add_bundled_log(
            BundledLog.from_utf8_bytes("gnss.log", ISO_SPACE_TEXT.encode())
        )
        .add_bundled_log_from_file(log_path)
        .finish()
    )

    gtd_path = tmp_path / "recording.gtd"
    nav_file.write_to_file(gtd_path)
    reopened = NavFile.open(gtd_path)

    expected = [
        BundledLog("gnss.log", ISO_SPACE_TEXT),
        BundledLog("modem.log", RFC3339_TEXT),
        BundledLog("syslog.log", SYSLOG_TEXT),
    ]
    assert list(reopened.bundled_logs) == expected
    assert [log.name for log in reopened.bundled_logs] == [
        "gnss.log",
        "modem.log",
        "syslog.log",
    ]
    assert reopened.bundled_logs[0].text == ISO_SPACE_TEXT
    assert repr(reopened.bundled_logs[0]) == (
        f'BundledLog(name="gnss.log", len={len(ISO_SPACE_TEXT.encode())})'
    )

    from_bytes = NavFile.from_bytes(nav_file.to_bytes())
    assert list(from_bytes.bundled_logs) == expected


def test_from_file_accepts_path_and_str(tmp_path: Path) -> None:
    log_path = tmp_path / "journal.log"
    log_path.write_text(SYSLOG_TEXT, encoding="utf-8", newline="\n")

    from_path = BundledLog.from_file(log_path)
    from_str = BundledLog.from_file(str(log_path))

    assert from_path == BundledLog("journal.log", SYSLOG_TEXT)
    assert from_str == from_path


def test_from_file_missing_path_raises_os_error(tmp_path: Path) -> None:
    with pytest.raises(OSError):
        BundledLog.from_file(tmp_path / "missing.log")


@pytest.mark.parametrize("name", ["", ".", "..", "dir/file.log", "bad\x00name"])
def test_invalid_log_name_raises_value_error(name: str) -> None:
    with pytest.raises(ValueError):
        BundledLog(name, ISO_SPACE_TEXT)


@pytest.mark.parametrize("text", ["", "   \n\t\n", "plain line without a timestamp\n"])
def test_empty_or_unrecognised_text_raises_value_error(text: str) -> None:
    with pytest.raises(ValueError):
        BundledLog("gnss.log", text)


def test_invalid_utf8_bytes_raises_value_error() -> None:
    with pytest.raises(ValueError):
        BundledLog.from_utf8_bytes("gnss.log", b"2024-06-01 09:00:00 \xff\xfe\n")


def test_duplicate_bundled_log_names_rejected_on_finish() -> None:
    builder = (
        NavFileBuilder()
        .add(NavFix(51.5, -0.1, gps_time=T0))
        .add(BundledLog("gnss.log", ISO_SPACE_TEXT))
        .add_bundled_log(BundledLog("gnss.log", RFC3339_TEXT))
    )
    with pytest.raises(ValueError, match="gnss.log"):
        builder.finish()


def test_add_bundled_log_from_file_on_consumed_builder_raises_runtime_error(
    tmp_path: Path,
) -> None:
    builder = NavFileBuilder()
    builder.finish()
    with pytest.raises(RuntimeError):
        builder.add_bundled_log_from_file(tmp_path / "missing.log")


def test_failed_add_bundled_log_from_file_leaves_builder_configurable(
    tmp_path: Path,
) -> None:
    builder = NavFileBuilder()
    with pytest.raises(OSError):
        builder.add_bundled_log_from_file(tmp_path / "missing.log")

    nav_file = builder.with_title("Recovered").finish()
    assert nav_file.meta.title == "Recovered"

"""SNR tests: each classification of a reading agrees with the Rust SDK."""

from __future__ import annotations

import pytest
from geotrace_sdk import Constellation, Satellite, snr_is_no_data_sentinel


# The cases of `the_band_is_half_a_db_wide_either_side` in the Rust SDK's `snr.rs`.
@pytest.mark.parametrize(
    ("snr", "no_data"),
    [(99.0, True), (99.4, True), (98.5, False), (99.5, False), (40.0, False)],
)
def test_a_reading_classifies_as_in_the_rust_sdk(snr: float, no_data: bool) -> None:
    assert snr_is_no_data_sentinel(snr) is no_data


@pytest.mark.parametrize(
    ("snr", "no_data"), [(None, False), (99.4, True), (40.0, False)]
)
def test_a_satellite_classifies_its_snr_reading_and_returns_false_without_one(
    snr: float | None, no_data: bool
) -> None:
    assert Satellite(Constellation.GPS, 1, snr=snr).snr_is_no_data_sentinel is no_data

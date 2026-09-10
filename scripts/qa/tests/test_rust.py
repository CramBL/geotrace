"""Tests for `qa._rust`: the `#[cfg(test)]` regions of a Rust file."""

import pytest

from qa import _rust

_A_BLOCK_ITEM = """\
fn production() {}

#[cfg(test)]
mod tests {
    fn t() {}
}

fn also_production() {}
"""

_A_DECLARATION = """\
#[cfg(test)]
use std::fs;

fn production() {
    let handle = fs::File::open("a");
}
"""

_AN_ATTRIBUTE_BETWEEN = """\
#[cfg(test)]
#[path = "other.rs"]
mod tests;

fn production() {}
"""

_A_BRACKETED_TYPE = """\
#[cfg(test)]
const N: [u8; 4] = [0; 4];

fn production() {}
"""

_TWO_GATED_ITEMS = """\
#[cfg(test)]
fn helper() {}

fn production() {}

#[cfg(test)]
mod tests {}
"""

_A_BRACE_IN_A_STRING = """\
#[cfg(test)]
mod tests {
    const CLOSING: &str = "}";
}

fn production() {}
"""

_AN_INNER_ATTRIBUTE = """\
#![cfg(test)]

fn helper() {}
"""


@pytest.mark.parametrize(
    ("source", "expected"),
    [
        (_A_BLOCK_ITEM, {3, 4, 5, 6}),
        (_A_DECLARATION, {1, 2}),
        (_AN_ATTRIBUTE_BETWEEN, {1, 2, 3}),
        (_A_BRACKETED_TYPE, {1, 2}),
        (_TWO_GATED_ITEMS, {1, 2, 6, 7}),
        (_A_BRACE_IN_A_STRING, {1, 2, 3, 4}),
        (_AN_INNER_ATTRIBUTE, {1, 2, 3}),
        ("fn production() {}\n", set()),
    ],
)
def test_cfg_test_line_numbers_covers_the_gated_item(source: str, expected: set[int]) -> None:
    assert _rust.cfg_test_line_numbers(source) == expected

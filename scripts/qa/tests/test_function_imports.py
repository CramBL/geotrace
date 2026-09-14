"""Tests for `qa.check_function_imports`: the ban on fully importing a function."""

import random

import pytest
from conftest import GitRepository

from qa import _rust, check_function_imports


def _violations(repository: GitRepository, source: str) -> list[tuple[int, str]]:
    path = repository.root / "src" / "a.rs"
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(source)
    return [
        (lineno, text) for _, lineno, text in check_function_imports._collect(repository.root)
    ]


def test_imported_names_reads_a_nested_brace_group() -> None:
    statement = "use gt_snap::{merge::{self, ChunkOutcome}, wire::Costing, fixtures_dir};"

    assert [name for name, _ in check_function_imports.imported_names(statement)] == [
        "self",
        "ChunkOutcome",
        "Costing",
        "fixtures_dir",
    ]


def test_flags_a_function_imported_by_its_own_name(git_repository: GitRepository) -> None:
    source = "use std::fs::read_to_string;\n\nfn read(p: &str) {\n    read_to_string(p).ok();\n}\n"

    assert _violations(git_repository, source) == [
        (1, "read_to_string - use std::fs::read_to_string;")
    ]


def test_flags_a_function_listed_beside_types_inside_a_test_module(
    git_repository: GitRepository,
) -> None:
    source = (
        "pub struct DatabaseRef;\n"
        "\n"
        "fn display_identity() -> u32 {\n"
        "    1\n"
        "}\n"
        "\n"
        "#[cfg(test)]\n"
        "mod tests {\n"
        "    use super::{DatabaseRef, display_identity};\n"
        "\n"
        "    #[test]\n"
        "    fn t() {\n"
        "        assert_eq!(display_identity(), 1);\n"
        "    }\n"
        "}\n"
    )

    assert _violations(git_repository, source) == [
        (9, "display_identity - use super::{DatabaseRef, display_identity};")
    ]


def test_reports_the_line_a_leaf_sits_on_in_a_statement_over_several_lines(
    git_repository: GitRepository,
) -> None:
    source = (
        "use gt_jam::{\n"
        "    CapturedDay,\n"
        "    parse_day,\n"
        "};\n"
        "\n"
        "fn read() {\n"
        "    parse_day(&[]);\n"
        "}\n"
    )

    assert _violations(git_repository, source) == [(3, "parse_day - parse_day,")]


def test_flags_an_aliased_function_under_its_alias(git_repository: GitRepository) -> None:
    source = "use gt_types::to_pixel as project;\n\nfn draw() {\n    project(1.0);\n}\n"

    assert _violations(git_repository, source) == [
        (1, "project - use gt_types::to_pixel as project;")
    ]


@pytest.mark.parametrize(
    ("what", "source", "violation"),
    [
        (
            "an argument",
            "use crate::tpv_renderer::fix_count_color;\n"
            "\n"
            "fn cells(counts: &[u32]) {\n"
            "    counts.iter().map(fix_count_color);\n"
            "}\n",
            "fix_count_color - use crate::tpv_renderer::fix_count_color;",
        ),
        (
            "the left operand of a comparison",
            "use crate::hooks::default_handler;\n"
            "\n"
            "fn is_default(handler: fn()) -> bool {\n"
            "    default_handler != handler\n"
            "}\n",
            "default_handler - use crate::hooks::default_handler;",
        ),
        (
            "an argument beside a struct literal field, a borrow and an array element",
            "use crate::tpv_renderer::fix_count_color;\n"
            "\n"
            "fn cells(counts: &[u32]) -> Cell {\n"
            "    counts.iter().map(fix_count_color);\n"
            "    let colors = [fix_count_color];\n"
            "    Cell { color: fix_count_color, first: &fix_count_color, colors }\n"
            "}\n",
            "fix_count_color - use crate::tpv_renderer::fix_count_color;",
        ),
    ],
)
def test_flags_a_function_passed_as_a_value(
    git_repository: GitRepository, what: str, source: str, violation: str
) -> None:
    assert _violations(git_repository, source) == [(1, violation)]


@pytest.mark.parametrize(
    ("what", "source"),
    [
        (
            "a module qualifying its call",
            "use gt_types::mercator;\n\nfn draw() {\n    mercator::to_pixel(1.0);\n}\n",
        ),
        (
            "a module beside a parameter of the same name",
            "use crate::transform;\n"
            "\n"
            "fn draw(transform: Transform) {\n"
            "    transform::lod_points(transform);\n"
            "}\n",
        ),
        (
            "a macro",
            "use gt_types::assert_close;\n\nfn check() {\n    assert_close!(1.0, 1.0);\n}\n",
        ),
        (
            "a macro beside a local binding of the same name",
            "use serde_json::json;\n"
            "\n"
            "fn encode() -> String {\n"
            "    let json = json!(1);\n"
            "    json.to_string()\n"
            "}\n",
        ),
        (
            "a field access and a method call",
            "use gt_types::heading;\n"
            "\n"
            "fn f(p: Point) -> f64 {\n"
            "    p.heading.max(p.heading())\n"
            "}\n",
        ),
        (
            "a lowercase type in a generic argument",
            "use uom::si::length::meter;\n\nfn far() {\n    Length::new::<meter>(3.0);\n}\n",
        ),
        (
            "a lowercase type behind a raw pointer",
            "use std::ffi::c_char;\n\nfn title() -> *const c_char {\n    TITLE.as_ptr()\n}\n",
        ),
        (
            "a lowercase type in a cast",
            "use std::ffi::c_char;\n\nfn write(byte: u8) {\n    push(byte as c_char);\n}\n",
        ),
        (
            "a lowercase type as a return type",
            "use std::ffi::c_char;\n\nfn first() -> c_char {\n    0\n}\n",
        ),
        (
            "a lowercase type in an array type",
            "use std::ffi::c_char;\n\npub struct Label {\n    pub text: [c_char; 256],\n}\n",
        ),
        (
            "a lowercase type as a parameter type",
            "use std::ffi::c_char;\n\nfn write(byte: c_char) {\n    push(byte);\n}\n",
        ),
        (
            "a lowercase type in a slice type",
            "use std::ffi::c_char;\n\nfn fill(field: &mut [c_char]) {\n    clear(field);\n}\n",
        ),
        (
            "a lowercase type behind a mutable reference",
            "use std::ffi::c_char;\n\nfn write(byte: &mut c_char) {\n    clear(byte);\n}\n",
        ),
        (
            "a lowercase type behind a reference with a lifetime",
            "use std::ffi::c_char;\n"
            "\n"
            "fn first<'a>(text: &'a Text) -> &'a c_char {\n"
            "    text.first()\n"
            "}\n",
        ),
        (
            "a re-export the file never uses",
            "pub use gt_fmt::render_name_template;\n",
        ),
        (
            "a call the file qualifies with its module",
            "use gt_fmt::name_template;\n\nfn f() {\n    name_template::render(1);\n}\n",
        ),
    ],
)
def test_leaves_a_name_the_file_never_uses_as_a_function(
    git_repository: GitRepository, what: str, source: str
) -> None:
    assert _violations(git_repository, source) == []


@pytest.mark.parametrize(
    ("what", "use_of_the_function"),
    [
        ("a struct literal field", "Cell { color: fix_count_color }"),
        ("a borrow", "&fix_count_color"),
        ("an array element", "[fix_count_color]"),
    ],
)
def test_leaves_a_function_set_only_as_a_field_a_borrow_or_an_array_element(
    git_repository: GitRepository, what: str, use_of_the_function: str
) -> None:
    source = (
        "use crate::tpv_renderer::fix_count_color;\n"
        "\n"
        "fn cell() {\n"
        f"    keep({use_of_the_function});\n"
        "}\n"
    )

    assert _violations(git_repository, source) == []


@pytest.mark.parametrize(
    ("what", "source"),
    [
        (
            "self in a brace list",
            "use gt_snap::merge::{self, ChunkOutcome};\n"
            "\n"
            "fn run() {\n"
            "    merge::chunks(1);\n"
            "}\n",
        ),
        (
            "a trait imported as an anonymous binding",
            "use gt_test_utils::HarnessInteraction as _;\n"
            "\n"
            "fn run(h: Harness) {\n"
            "    h.click(1);\n"
            "}\n",
        ),
    ],
)
def test_leaves_the_forms_code_style_asks_for(
    git_repository: GitRepository, what: str, source: str
) -> None:
    assert _violations(git_repository, source) == []


def test_honors_the_exemption_comment(git_repository: GitRepository) -> None:
    source = (
        "use std::fs::read_to_string; "
        '// [qa-allow-check-function-imports, reason = "ok"]\n'
        "\n"
        "fn read(p: &str) {\n"
        "    read_to_string(p).ok();\n"
        "}\n"
    )

    assert _violations(git_repository, source) == []


@pytest.mark.parametrize(
    ("what", "source"),
    [
        (
            "a definition of the same name in an outer module",
            "fn label(text: &str) -> &str {\n"
            "    text\n"
            "}\n"
            "\n"
            "#[cfg(test)]\n"
            "mod tests {\n"
            "    use gt_ui_types::label;\n"
            "\n"
            "    #[test]\n"
            "    fn t() {\n"
            "        label::of(1);\n"
            "    }\n"
            "}\n",
        ),
        (
            "a module declaration of the same name",
            "mod check;\n\npub use check::{CheckedQuery, check};\n",
        ),
        (
            "a macro_rules definition of the same name",
            "macro_rules! for_each_archive {\n"
            "    () => {};\n"
            "}\n"
            "\n"
            "pub(crate) use for_each_archive;\n",
        ),
        (
            "an attribute macro taking arguments",
            "use serial_test::serial;\n\n#[serial(archive)]\nfn t() {}\n",
        ),
    ],
)
def test_reads_neither_a_definition_nor_an_attribute_as_a_use(
    git_repository: GitRepository, what: str, source: str
) -> None:
    assert _violations(git_repository, source) == []


@pytest.mark.parametrize(
    ("what", "source"),
    [
        (
            "a use inside a block comment",
            "/*\nuse std::fs::read_to_string;\n*/\n\nfn read(p: &str) {\n"
            "    fs::read_to_string(p).ok();\n}\n",
        ),
        (
            "a use inside a raw string",
            'const SOURCE: &str = r#"\nuse std::fs::read_to_string;\n"#;\n'
            "\nfn read(p: &str) {\n    fs::read_to_string(p).ok();\n}\n",
        ),
        (
            "a call inside a line comment",
            "use gt_types::mercator;\n\n// mercator(1.0) was the old spelling.\n"
            "fn draw() {\n    mercator::to_pixel(1.0);\n}\n",
        ),
        (
            "a call inside a string",
            'use gt_types::mercator;\n\nfn draw() {\n    log::warn!("mercator(1.0) failed");\n'
            "    mercator::to_pixel(1.0);\n}\n",
        ),
    ],
)
def test_reads_neither_a_comment_nor_a_string_literal(
    git_repository: GitRepository, what: str, source: str
) -> None:
    assert _violations(git_repository, source) == []


def test_a_quote_in_a_char_literal_opens_no_string(git_repository: GitRepository) -> None:
    source = (
        "use std::fs::read_to_string;\n"
        "\n"
        "fn read(p: &str) -> bool {\n"
        "    read_to_string(p).unwrap_or_default().contains('\"')\n"
        "}\n"
    )

    assert _violations(git_repository, source) == [
        (1, "read_to_string - use std::fs::read_to_string;")
    ]


@pytest.mark.parametrize(
    "source",
    [
        "use gt_types::{Track;\n",
        "use gt_types::{a, {b, c};\n",
        "use gt_types::\n",
        "use ;\n",
        "use a as ;\n",
        "/* use std::fs::read_to_string;\n",
        'const S: &str = r#"use std::fs::read_to_string;\n',
        'const S: &str = "use std::fs::read_to_string;\n',
        "let c = '\n",
    ],
)
def test_reads_malformed_source_without_raising(
    git_repository: GitRepository, source: str
) -> None:
    _violations(git_repository, source)


_FRAGMENTS = (
    "use",
    "pub",
    "gt_types",
    "read_to_string",
    "Track",
    "self",
    "as",
    "mod",
    "macro_rules",
    "_",
    "::",
    ":",
    "{",
    "}",
    ",",
    ";",
    "(",
    ")",
    "<",
    ">",
    "->",
    "&",
    "!",
    "*",
    "[",
    "]",
    "//",
    "/*",
    "*/",
    '"',
    'r#"',
    '"#',
    "'",
    "\\",
    " ",
    "\n",
)

_RANDOM_SOURCES = 2000

_SEED = 20260908


def test_random_source_keeps_the_offsets_and_raises_nothing() -> None:
    rng = random.Random(_SEED)
    for _ in range(_RANDOM_SOURCES):
        source = "".join(rng.choice(_FRAGMENTS) for _ in range(rng.randrange(1, 40)))
        code = _rust.mask_comments_and_strings(source)

        assert len(code) == len(source)
        assert code.count("\n") == source.count("\n")

        statements = list(check_function_imports.use_statements(code.splitlines()))
        code_outside_use_statements = check_function_imports._blank_use_statements(code, statements)

        assert len(code_outside_use_statements) == len(code)

        check_function_imports._is_used_as_a_function(code_outside_use_statements, "read_to_string")
        for _, statement in statements:
            for name, offset in check_function_imports.imported_names(statement):
                assert 0 <= offset <= len(statement)
                check_function_imports._is_used_as_a_function(code_outside_use_statements, name)

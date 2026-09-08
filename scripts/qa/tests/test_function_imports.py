"""Tests for `qa.check_function_imports`: the ban on fully importing a function."""

import random

import pytest
from conftest import GitRepository

from qa import check_function_imports


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
        "    FixtureDay,\n"
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
    ("what", "source"),
    [
        (
            "a module qualifying its call",
            "use gt_types::mercator;\n\nfn draw() {\n    mercator::to_pixel(1.0);\n}\n",
        ),
        (
            "a macro",
            "use gt_types::assert_close;\n\nfn check() {\n    assert_close!(1.0, 1.0);\n}\n",
        ),
        (
            "a lowercase type in a generic argument",
            "use uom::si::length::meter;\n\nfn far() {\n    Length::new::<meter>(3.0);\n}\n",
        ),
        (
            "a re-export the file never calls",
            "pub use gt_fmt::render_name_template;\n",
        ),
        (
            "a call the file qualifies with its module",
            "use gt_fmt::name_template;\n\nfn f() {\n    name_template::render(1);\n}\n",
        ),
    ],
)
def test_leaves_a_name_the_file_never_calls_on_its_own(
    git_repository: GitRepository, what: str, source: str
) -> None:
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
            "an attribute macro taking arguments",
            "use serial_test::serial;\n\n#[serial(archive)]\nfn t() {}\n",
        ),
    ],
)
def test_reads_neither_a_definition_nor_an_attribute_as_a_call(
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
    "_",
    "::",
    "{",
    "}",
    ",",
    ";",
    "(",
    ")",
    "<",
    ">",
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
        code = check_function_imports.mask_comments_and_strings(source)

        assert len(code) == len(source)
        assert code.count("\n") == source.count("\n")

        for _, statement in check_function_imports.use_statements(code.splitlines()):
            for _, offset in check_function_imports.imported_names(statement):
                assert 0 <= offset <= len(statement)

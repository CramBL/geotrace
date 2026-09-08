"""Tests for `qa.check_inline_modules`: the rule that a module is a file."""

import pytest
from conftest import GitRepository

from qa import check_inline_modules


def _violations(repository: GitRepository, source: str) -> list[tuple[int, str]]:
    path = repository.root / "src" / "a.rs"
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(source)
    return [(lineno, text) for _, lineno, text in check_inline_modules._collect(repository.root)]


def test_flags_a_module_written_inline_with_a_body(git_repository: GitRepository) -> None:
    source = "mod parser {\n    pub fn parse() {}\n}\n"

    assert _violations(git_repository, source) == [(1, "parser - mod parser {")]


@pytest.mark.parametrize(
    ("what", "source", "expected"),
    [
        (
            "a module inside the tests module",
            "#[cfg(test)]\nmod tests {\n    mod constellation {\n        #[test]\n"
            "        fn t() {}\n    }\n}\n",
            [(3, "constellation - mod constellation {")],
        ),
        (
            "a tests module inside another module",
            "#[cfg(unix)]\npub(crate) mod installation {\n    mod tests {\n        #[test]\n"
            "        fn t() {}\n    }\n}\n",
            [
                (2, "installation - pub(crate) mod installation {"),
                (3, "tests - mod tests {"),
            ],
        ),
    ],
)
def test_flags_a_nested_module(
    git_repository: GitRepository, what: str, source: str, expected: list[tuple[int, str]]
) -> None:
    assert _violations(git_repository, source) == expected


@pytest.mark.parametrize(
    ("what", "source"),
    [
        (
            "the tests module at file level",
            "#[cfg(test)]\nmod tests {\n    #[test]\n    fn t() {}\n}\n",
        ),
        (
            "a module declared without a body",
            "mod parser;\npub mod render;\n",
        ),
        (
            "a module read from a path attribute",
            '#[cfg(test)]\n#[path = "app/ui_tests.rs"]\nmod tests;\n',
        ),
        (
            "an inline module a path attribute reads its children from",
            '#[cfg(test)]\n#[path = "shared/"]\nmod fixtures {\n    mod capture_manifest;\n}\n',
        ),
        (
            "a block after a module, which closes at its own brace",
            "mod tests {\n    fn helper() {}\n}\n\nfn main() {\n    let x = 1;\n}\n",
        ),
    ],
)
def test_leaves_the_forms_code_style_asks_for(
    git_repository: GitRepository, what: str, source: str
) -> None:
    assert _violations(git_repository, source) == []


def test_honors_the_exemption_comment(git_repository: GitRepository) -> None:
    source = (
        "mod sealed { "
        '// [qa-allow-check-inline-modules, reason = "seals the trait"]\n'
        "    pub trait Sealed {}\n"
        "}\n"
    )

    assert _violations(git_repository, source) == []


@pytest.mark.parametrize(
    ("what", "source"),
    [
        ("a block comment", "/*\nmod parser {\n    fn parse() {}\n}\n*/\n"),
        (
            "a raw string",
            'const SOURCE: &str = r#"\nmod parser {\n    fn parse() {}\n}\n"#;\n',
        ),
    ],
)
def test_reads_neither_a_comment_nor_a_string_literal(
    git_repository: GitRepository, what: str, source: str
) -> None:
    assert _violations(git_repository, source) == []


@pytest.mark.parametrize(
    "source",
    [
        "mod parser {\n",
        "mod {\n}\n",
        "}\n}\nmod parser {\n}\n",
        "#[path]\n",
        "] mod parser {\n}\n",
        "#[cfg(test)\nmod parser {\n}\n",
        "mod\n",
    ],
)
def test_reads_malformed_source_without_raising(
    git_repository: GitRepository, source: str
) -> None:
    _violations(git_repository, source)

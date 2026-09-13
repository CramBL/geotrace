"""Tests for `qa.check_no_tests_support`: test helpers live in `test_util`."""

from pathlib import Path

from conftest import GitRepository

from qa import check_no_tests_support


def _write(root: Path, rel: str, body: str) -> None:
    path = root / rel
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(body)


def _violations(repository: GitRepository) -> list[tuple[str, int, str]]:
    return [
        (path.relative_to(repository.root).as_posix(), lineno, text)
        for path, lineno, text in check_no_tests_support._collect(repository.root)
    ]


def test_flags_every_file_under_a_tests_support_directory(
    git_repository: GitRepository,
) -> None:
    _write(git_repository.root, "crates/gt-x/tests/support/mod.rs", "//! Fixtures.\n")
    _write(git_repository.root, "crates/gt-x/tests/support/generate/mod.rs", "pub fn f() {}\n")

    assert _violations(git_repository) == [
        ("crates/gt-x/tests/support/generate/mod.rs", 1, "pub fn f() {}"),
        ("crates/gt-x/tests/support/mod.rs", 1, "//! Fixtures."),
    ]


def test_skips_a_file_outside_a_tests_support_directory(git_repository: GitRepository) -> None:
    _write(git_repository.root, "crates/gt-x/src/test_util.rs", "//! Fixtures.\n")
    _write(git_repository.root, "crates/gt-x/tests/support_matrix.rs", "#[test]\nfn t() {}\n")

    assert _violations(git_repository) == []


def test_honors_an_exemption_on_the_first_line(git_repository: GitRepository) -> None:
    body = '//! Fixtures. // [qa-allow-check-no-tests-support, reason = "ok"]\n'
    _write(git_repository.root, "crates/gt-x/tests/support/mod.rs", body)

    assert _violations(git_repository) == []

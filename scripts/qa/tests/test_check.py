"""Tests for `qa._check`: the git queries and the added-line filter the gates share."""

from pathlib import Path

import pytest
from conftest import GitRepository

from qa import _check

_REPLACEMENT_MESSAGE = "a replacement subject\n\nA replacement body.\n"

_BODY = "An appended note.\n"


@pytest.mark.parametrize(
    ("hunk", "expected"),
    [
        ("@@ -1,0 +5,3 @@", {5, 6, 7}),
        ("@@ -1 +5 @@", {5}),
        ("@@ -4,2 +3,0 @@", set()),
    ],
)
def test_added_lines_reads_a_hunk_header(hunk: str, expected: set[int]) -> None:
    diff = f"--- a/README.md\n+++ b/README.md\n{hunk}\n+added\n"
    assert _check.added_lines(diff).get("README.md", set()) == expected


def test_added_lines_keys_each_hunk_on_the_file_above_it() -> None:
    diff = """\
--- a/README.md
+++ b/README.md
@@ -1,0 +2,1 @@
+one
--- a/justfile
+++ b/justfile
@@ -8,0 +9,2 @@
+two
+three
"""
    assert _check.added_lines(diff) == {"README.md": {2}, "justfile": {9, 10}}


def test_merge_base_exits_with_one_line_when_the_base_ref_does_not_resolve(
    git_repository: GitRepository,
) -> None:
    git_repository.commit_empty("root")

    with pytest.raises(SystemExit) as raised:
        _check.merge_base_with_head(git_repository.root, "origin/trunk")

    assert str(raised.value) == (
        "error: base ref origin/trunk does not resolve: fetch it, or pass another base"
    )


def test_commits_in_drops_a_fixup_subject_and_keeps_a_body_that_quotes_one(
    git_repository: GitRepository,
) -> None:
    git_repository.commit_empty("root")
    git_repository.commit_empty("fixup! root")
    git_repository.commit_empty("quote a fixup subject\n\nfixup! root is what this body says.")
    git_repository.commit_empty(f"squash! root\n\n{_BODY}")
    git_repository.commit_empty(f"amend! root\n\n{_REPLACEMENT_MESSAGE}")

    commits = _check.commits_in(git_repository.root, "HEAD")

    assert [commit.subject for commit in commits] == [
        "amend! root",
        "squash! root",
        "quote a fixup subject",
        "root",
    ]
    assert commits[0].message == f"amend! root\n\n{_REPLACEMENT_MESSAGE}"
    assert commits[-1].message == "root\n"


@pytest.mark.parametrize(
    ("message", "expected"),
    [
        (f"amend! root\n\n{_REPLACEMENT_MESSAGE}", _REPLACEMENT_MESSAGE),
        ("amend! root\n", ""),
        (f"squash! root\n\n{_BODY}", f"\n\n{_BODY}"),
        (f"a plain subject\n\n{_BODY}", f"a plain subject\n\n{_BODY}"),
    ],
)
def test_message_to_land_returns_the_text_that_reaches_the_branch(
    message: str, expected: str
) -> None:
    assert _check.Commit(hash="8250d2ea", message=message).message_to_land() == expected


def _two_line_check(root: Path) -> _check.Check:
    def collect(scanned: Path) -> list[_check.Violation]:
        return [(scanned / "src" / "a.rs", 1, "first"), (scanned / "src" / "a.rs", 2, "second")]

    return _check.Check(
        name="check-example",
        title="an example violation found",
        collect=collect,
        note=["a note"],
        help=["a help line"],
    )


def test_run_check_on_added_reports_the_violation_on_an_added_line(
    tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    failed = _check.run_check_on_added(_two_line_check(tmp_path), tmp_path, {"src/a.rs": {2}})

    assert failed
    printed = capsys.readouterr().out
    assert "second" in printed
    assert "first" not in printed


def test_run_check_on_added_passes_when_the_change_touched_another_file(tmp_path: Path) -> None:
    assert not _check.run_check_on_added(
        _two_line_check(tmp_path), tmp_path, {"src/b.rs": {1, 2}}
    )

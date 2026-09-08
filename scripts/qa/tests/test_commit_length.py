"""Tests for `qa.check_commit_length`: the ceiling on a commit message body."""

import pytest
from conftest import GitRepository

from qa import check_commit_length
from qa._check import Commit, commits_in

_CEILING = check_commit_length.MAX_BODY_LINES


def _body_of(lines: int) -> str:
    return "\n".join(f"line {number}" for number in range(1, lines + 1))


@pytest.mark.parametrize(("lines", "over"), [(_CEILING, False), (_CEILING + 1, True)])
def test_the_ceiling_is_the_longest_body_that_passes(lines: int, over: bool) -> None:
    commit = Commit(hash="8250d2ea", message=f"a subject\n\n{_body_of(lines)}\n")

    assert check_commit_length.body_line_count(commit) == lines
    assert bool(check_commit_length.over_ceiling([commit])) == over


@pytest.mark.parametrize(
    ("what", "message", "expected"),
    [
        ("a subject on its own", "a subject\n", 0),
        ("a blank line between paragraphs", "a subject\n\nfirst\n\nsecond\n", 3),
        (
            "an amend replacement message",
            f"amend! a subject\n\na new subject\n\n{_body_of(4)}\n",
            4,
        ),
    ],
)
def test_counts_the_lines_after_the_subject(what: str, message: str, expected: int) -> None:
    assert check_commit_length.body_line_count(Commit(hash="8250d2ea", message=message)) == expected


def test_a_fixup_body_never_reaches_the_ceiling(git_repository: GitRepository) -> None:
    git_repository.commit_empty("root")
    git_repository.commit_empty(f"fixup! root\n\n{_body_of(_CEILING + 10)}\n")

    assert check_commit_length.over_ceiling(commits_in(git_repository.root, "HEAD")) == []

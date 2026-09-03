"""Tests for `qa.lint_prose`: the parsing and formatting the prose gate is built from.

No test runs Vale or reaches the network: `_VALE_JSON` is a captured reply.
"""

import subprocess
from pathlib import Path

import pytest

from qa import lint_prose

_VALE_JSON = """\
{
  "justfile": [
    {
      "Span": [3, 10],
      "Check": "GeoTrace.Overused",
      "Message": "Overused in generated prose: 'seamless'.",
      "Severity": "error",
      "Line": 12
    },
    {
      "Span": [17, 17],
      "Check": "GeoTrace.Semicolon",
      "Message": "Semicolon: split into two sentences.",
      "Severity": "error",
      "Line": 40
    }
  ]
}
"""

_JUST_SOURCE = """\
# Lint the tracked Markdown files.
[doc("Every surface, the whole backlog.")]
vale-docs:
    git ls-files -z '*.md'
"""

_CMAKE_SOURCE = """\
# The C SDK builds as a static library.
project(geotrace_c VERSION 0.5.1)
#Tight comment.
"""

_YAML_SOURCE = """\
# A pull request is gated against its base.
jobs:
  vale:
    runs-on: ubuntu-latest # trailing comments stay with the code
"""


def test_annotation_anchors_a_file_alert_on_its_line() -> None:
    alert = lint_prose.parse_alerts(_VALE_JSON)[0]
    assert alert.annotation() == (
        "::error file=justfile,line=12::GeoTrace.Overused: "
        "Overused in generated prose: 'seamless'."
    )


def test_annotation_anchors_a_commit_alert_on_its_hash() -> None:
    alert = lint_prose.parse_alerts(_VALE_JSON, where="8250d2ea", commit=True)[0]
    assert alert.annotation() == (
        "::error title=commit 8250d2ea::GeoTrace.Overused: "
        "Overused in generated prose: 'seamless'."
    )


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
    assert lint_prose.added_lines(diff).get("README.md", set()) == expected


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
    assert lint_prose.added_lines(diff) == {"README.md": {2}, "justfile": {9, 10}}


def test_keep_added_drops_an_alert_off_an_added_line() -> None:
    alerts = lint_prose.parse_alerts(_VALE_JSON)
    kept = lint_prose.keep_added(alerts, {"justfile": {12}})
    assert [alert.check for alert in kept] == ["GeoTrace.Overused"]


def test_parse_alerts_names_a_stdin_reply_after_the_file_it_read() -> None:
    alerts = lint_prose.parse_alerts(
        _VALE_JSON.replace("justfile", "stdin.md"), where="scripts/x.just"
    )
    assert [alert.where for alert in alerts] == ["scripts/x.just", "scripts/x.just"]


@pytest.mark.parametrize(
    ("source", "expected"),
    [
        (
            _JUST_SOURCE,
            "Lint the tracked Markdown files.\nEvery surface, the whole backlog.\n\n\n",
        ),
        (_CMAKE_SOURCE, "The C SDK builds as a static library.\n\nTight comment.\n"),
        (_YAML_SOURCE, "A pull request is gated against its base.\n\n\n\n"),
    ],
)
def test_comment_text_keeps_the_comments_and_blanks_the_rest(source: str, expected: str) -> None:
    assert lint_prose.comment_text(source) == expected


@pytest.mark.parametrize(
    ("reported", "expected"),
    [
        ("vale version 3.18.0\n", "3.18.0"),
        ("vale version v3.18.0\n", "3.18.0"),
        ("no version here\n", None),
    ],
)
def test_normalize_version_reads_the_dotted_version(reported: str, expected: str | None) -> None:
    assert lint_prose.normalize_version(reported) == expected


def test_summary_counts_what_the_run_covered_and_found() -> None:
    totals = lint_prose.RunTotals(files=4, lines=217, commits=1, errors=5, warnings=0)
    assert totals.summary("origin/trunk") == (
        "vale: 4 files, 217 added lines, 1 commit checked: 5 errors, 0 warnings"
    )


def test_summary_says_nothing_to_check_when_the_range_is_empty() -> None:
    totals = lint_prose.RunTotals(files=0, lines=0, commits=0, errors=0, warnings=0)
    assert totals.summary("HEAD") == (
        "vale: nothing to check since HEAD: "
        "no added lines in a linted file, and no commits in the range"
    )


def _repository_with_one_commit(root: Path) -> None:
    for args in (
        ["init", "--quiet"],
        ["commit", "--quiet", "--allow-empty", "-m", "root"],
    ):
        subprocess.run(
            ["git", "-c", "user.email=qa@example.com", "-c", "user.name=QA", *args],
            cwd=root,
            check=True,
            capture_output=True,
        )


def test_merge_base_exits_with_one_line_when_the_base_ref_does_not_resolve(tmp_path: Path) -> None:
    _repository_with_one_commit(tmp_path)

    with pytest.raises(SystemExit) as raised:
        lint_prose._merge_base_with_head(tmp_path, "origin/trunk")

    assert str(raised.value) == (
        "error: base ref origin/trunk does not resolve: fetch it, or pass another base"
    )

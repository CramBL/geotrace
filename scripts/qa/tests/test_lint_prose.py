"""Tests for `qa.lint_prose`: the parsing and formatting the prose gate is built from.

No test runs Vale or reaches the network: `_VALE_JSON` and `_CONTRASTIVE_JSON`
are captured replies.
"""

from collections.abc import Sequence
from pathlib import Path

import pytest
from conftest import GitRepository

from qa import lint_prose
from qa._check import Commit

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

_CONTRASTIVE_JSON = """\
{
  "stdin.commit": [
    {
      "Span": [22, 32],
      "Check": "GeoTrace.Contrastive",
      "Message": "Contrastive 'rather than': state the current behaviour without the alternative.",
      "Severity": "error",
      "Line": 3
    }
  ]
}
"""

_STUB_ENGINE = lint_prose.Engine(argv=["vale"], description="vale")

_REPLACEMENT_MESSAGE = "a replacement subject\n\nA replacement body.\n"

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


def test_script_files_reads_every_toml_file_and_only_the_workflow_yaml(
    git_repository: GitRepository,
) -> None:
    for rel in (
        "justfile",
        "Cargo.toml",
        "crates/gt-types/Cargo.toml",
        "lychee.toml",
        ".github/workflows/ci.yml",
        ".config/settings.yml",
        "README.md",
    ):
        path = git_repository.root / rel
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text("# A comment.\n")

    assert lint_prose.script_files(git_repository.root) == [
        ".github/workflows/ci.yml",
        "Cargo.toml",
        "crates/gt-types/Cargo.toml",
        "justfile",
        "lychee.toml",
    ]


def test_lint_commit_reads_an_amend_replacement_and_names_the_commit(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    commit = Commit(hash="8250d2ea", message=f"amend! root\n\n{_REPLACEMENT_MESSAGE}")
    read_by_vale: list[str | None] = []

    def stub_vale(
        engine: lint_prose.Engine, root: Path, args: Sequence[str], stdin: str | None = None
    ) -> str:
        read_by_vale.append(stdin)
        return _CONTRASTIVE_JSON

    monkeypatch.setattr(lint_prose, "_run_vale", stub_vale)

    alerts = lint_prose._lint_commit(_STUB_ENGINE, Path("."), commit)

    assert read_by_vale == [_REPLACEMENT_MESSAGE]
    assert [alert.annotation() for alert in alerts] == [
        "::error title=commit 8250d2ea::GeoTrace.Contrastive: "
        "Contrastive 'rather than': state the current behaviour without the alternative."
    ]


def test_lint_commit_runs_no_vale_on_an_amend_without_a_body(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    def fail_on_call(*args: object, **kwargs: object) -> str:
        raise AssertionError("vale ran on a commit with no body")

    monkeypatch.setattr(lint_prose, "_run_vale", fail_on_call)

    commit = Commit(hash="ad2e0f4", message="amend! root\n")

    assert lint_prose._lint_commit(_STUB_ENGINE, Path("."), commit) == []

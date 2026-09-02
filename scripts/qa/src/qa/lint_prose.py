"""Prose linting with Vale, one command per surface.

`added` is the gate CI runs: it keeps the alerts on lines a change adds, prints
them as GitHub annotations, closes with a summary line and exits non-zero on an
error-level alert. `VALE_VERBOSE=1`, and `GITHUB_ACTIONS` on its own, add the
detail of the run in GitHub log groups.

The other commands list the whole backlog of their surface and always exit 0.
"""

import argparse
import itertools
import json
import os
import re
import shutil
import subprocess
import sys
from collections import Counter
from collections.abc import Iterable, Sequence
from dataclasses import dataclass
from pathlib import Path

from qa._check import (
    c_family_files,
    just_and_cmake_files,
    markdown_files,
    python_files,
    repo_root,
    rs_files,
    yaml_files,
)

PINNED_VERSION = "3.18.0"
_IMAGE = "docker.io/jdkato/vale:v" + PINNED_VERSION

# The one style package .vale.ini pins. Its directory under .vale/styles/ is
# what a completed sync leaves behind.
_PINNED_STYLE_PACKAGE = "Slop"

# A long file list goes in batches: an argument list is bounded by the OS.
_BATCH = 200

_DEFAULT_BASE = "origin/trunk"


@dataclass(frozen=True)
class Engine:
    """The `argv` prefix that runs Vale, and that prefix as one printable line."""

    argv: list[str]
    description: str


_VERSION = re.compile(r"\d+(?:\.\d+)+")


def normalize_version(output: str) -> str | None:
    """The dotted version in `vale --version` output, which may write a leading `v`."""
    found = _VERSION.search(output)
    return None if found is None else found.group(0)


def _reported_version(binary: str) -> str | None:
    try:
        reported = subprocess.run(
            [binary, "--version"], capture_output=True, text=True, check=True
        ).stdout
    except (OSError, subprocess.CalledProcessError):
        return None
    return normalize_version(reported)


def select_engine(root: Path) -> Engine:
    """A native Vale at the pinned version, otherwise the pinned container image.

    Only the pinned version runs natively: the phrase exceptions and the
    Markdown parsing of comments differ between versions.
    """
    binary = shutil.which("vale")
    if binary is not None:
        version = _reported_version(binary)
        if version == PINNED_VERSION:
            return Engine([binary], binary)
        if version is not None:
            print(
                f"vale {version} on PATH is not the pinned {PINNED_VERSION}: "
                "running the container image",
                file=sys.stderr,
            )
    runner = shutil.which("podman") or shutil.which("docker")
    if runner is None:
        raise SystemExit("error: neither podman nor docker found: install one to run vale")
    argv = [runner, "run", "--rm", "-i", "-v", f"{root}:/docs", "-w", "/docs", _IMAGE]
    return Engine(argv, " ".join(argv))


def _run_vale(engine: Engine, root: Path, args: Sequence[str], stdin: str | None = None) -> str:
    result = subprocess.run(
        engine.argv + list(args), cwd=root, input=stdin, capture_output=True, text=True
    )
    if result.returncode != 0:
        sys.stderr.write(result.stderr)
        raise SystemExit(f"error: vale exited {result.returncode}")
    return result.stdout


def _git(root: Path, args: Sequence[str]) -> str:
    return subprocess.run(
        ["git", *args], cwd=root, capture_output=True, text=True, check=True
    ).stdout


@dataclass(frozen=True)
class Alert:
    """One Vale alert, against a repository file or against a commit message."""

    where: str
    line: int
    column: int
    severity: str
    check: str
    message: str
    commit: bool = False

    def listing(self) -> str:
        return f"{self.where}:{self.line}:{self.column}:{self.check}:{self.message}"

    def annotation(self) -> str:
        kind = "error" if self.severity == "error" else "warning"
        if self.commit:
            return f"::{kind} title=commit {self.where}::{self.check}: {self.message}"
        return f"::{kind} file={self.where},line={self.line}::{self.check}: {self.message}"


def parse_alerts(output: str, where: str | None = None, commit: bool = False) -> list[Alert]:
    """Vale's JSON as alerts. `where` replaces Vale's own key, which is `stdin.md`
    for a file read from stdin."""
    if not output.strip():
        return []
    found = []
    for key, entries in json.loads(output).items():
        for entry in entries:
            found.append(
                Alert(
                    where=key if where is None else where,
                    line=int(entry["Line"]),
                    column=int(entry["Span"][0]),
                    severity=str(entry["Severity"]),
                    check=str(entry["Check"]),
                    message=str(entry["Message"]),
                    commit=commit,
                )
            )
    return found


def keep_added(alerts: Iterable[Alert], added: dict[str, set[int]]) -> list[Alert]:
    """The alerts on a line the change adds."""
    return [alert for alert in alerts if alert.line in added.get(alert.where, set())]


_JUST_DOC = re.compile(r"""^\s*\[doc\((["'])(.*)\1\)\]\s*$""")
_HASH_COMMENT = re.compile(r"^\s*#[ ]?(.*)$")


def comment_text(source: str) -> str:
    """`source` with every line that is not a `#` comment or a just `[doc("...")]`
    attribute blanked: Vale then reads the comments alone and reports the file's
    own line numbers. For justfiles, CMake files and YAML."""
    kept = []
    for line in source.splitlines():
        doc = _JUST_DOC.match(line)
        if doc is not None:
            kept.append(doc.group(2))
            continue
        comment = _HASH_COMMENT.match(line)
        kept.append("" if comment is None else comment.group(1))
    return "\n".join(kept) + "\n"


def _relative(root: Path, paths: Iterable[Path]) -> list[str]:
    return sorted(path.relative_to(root).as_posix() for path in paths)


def docs_files(root: Path) -> list[str]:
    """The Markdown files, read whole."""
    return _relative(root, markdown_files(root))


def source_files(root: Path) -> list[str]:
    """The Rust, Python, C and C++ sources, whose comments Vale finds on its own."""
    return _relative(
        root, itertools.chain(rs_files(root), python_files(root), c_family_files(root))
    )


def script_files(root: Path) -> list[str]:
    """The justfiles, CMake files and workflow files, read through `comment_text`."""
    workflows = root / ".github" / "workflows"
    return _relative(
        root,
        itertools.chain(
            just_and_cmake_files(root),
            (path for path in yaml_files(root) if path.parent == workflows),
        ),
    )


def _lint_whole_files(engine: Engine, root: Path, paths: Sequence[str]) -> list[Alert]:
    found: list[Alert] = []
    for start in range(0, len(paths), _BATCH):
        batch = paths[start : start + _BATCH]
        found.extend(parse_alerts(_run_vale(engine, root, ["--no-exit", "--output=JSON", *batch])))
    return found


def _lint_comments(engine: Engine, root: Path, path: str) -> list[Alert]:
    text = comment_text((root / path).read_text(errors="replace"))
    output = _run_vale(engine, root, ["--no-exit", "--output=JSON", "--ext=.md"], stdin=text)
    return parse_alerts(output, where=path)


@dataclass(frozen=True)
class Commit:
    """One commit of the range, by its short hash and subject."""

    hash: str
    subject: str


def commits_in(root: Path, revision_range: str) -> list[Commit]:
    """The commits of `revision_range`, without the merge, `fixup!`, `squash!` and
    `amend!` commits: git and GitHub generate those subjects."""
    output = _git(
        root,
        [
            "log",
            "--no-merges",
            "--extended-regexp",
            "--invert-grep",
            "--grep=^(fixup|squash|amend)! ",
            "--format=%h%x09%s",
            revision_range,
        ],
    )
    found = []
    for line in output.splitlines():
        if line:
            short_hash, _, subject = line.partition("\t")
            found.append(Commit(short_hash, subject))
    return found


def _lint_commit(engine: Engine, root: Path, commit: Commit) -> list[Alert]:
    message = _git(root, ["log", "-1", "--format=%B", commit.hash])
    output = _run_vale(
        engine, root, ["--no-exit", "--output=JSON", "--ext=.commit"], stdin=message
    )
    return parse_alerts(output, where=commit.hash, commit=True)


_DIFF_FILE = re.compile(r"^\+\+\+ b/(.*)$")
_DIFF_HUNK = re.compile(r"^@@ -\S+ \+(\d+)(?:,(\d+))? @@")


def added_lines(diff: str) -> dict[str, set[int]]:
    """The line numbers each file gains, read from a `git diff -U0` text. A hunk
    without a count adds one line, and a hunk of count zero adds none."""
    added: dict[str, set[int]] = {}
    current = None
    for line in diff.splitlines():
        header = _DIFF_FILE.match(line)
        if header is not None:
            current = header.group(1)
            continue
        hunk = _DIFF_HUNK.match(line)
        if hunk is None or current is None:
            continue
        start = int(hunk.group(1))
        count = 1 if hunk.group(2) is None else int(hunk.group(2))
        if count:
            added.setdefault(current, set()).update(range(start, start + count))
    return added


def _plural(count: int, noun: str) -> str:
    return f"{count} {noun}" if count == 1 else f"{count} {noun}s"


@dataclass(frozen=True)
class RunTotals:
    """What one `added` run covered, and what it found."""

    files: int
    lines: int
    commits: int
    errors: int
    warnings: int

    def summary(self, base: str) -> str:
        if self.files == 0 and self.commits == 0:
            return (
                f"vale: nothing to check since {base}: "
                "no added lines in a linted file, and no commits in the range"
            )
        return (
            f"vale: {_plural(self.files, 'file')}, {_plural(self.lines, 'added line')}, "
            f"{_plural(self.commits, 'commit')} checked: "
            f"{_plural(self.errors, 'error')}, {_plural(self.warnings, 'warning')}"
        )


@dataclass(frozen=True)
class Run:
    """One `added` run: what it covered and every alert it raised."""

    base: str
    merge_base: str
    added: dict[str, set[int]]
    prose: list[str]
    scripts: list[str]
    commits: list[Commit]
    alerts: list[Alert]

    def surface_listing(self, label: str, paths: Sequence[str]) -> list[str]:
        listing = [f"{label} ({_plural(len(paths), 'file')}):"]
        listing.extend(f"  {len(self.added[path]):6d}  {path}" for path in paths)
        return listing

    def totals(self) -> RunTotals:
        errors = sum(1 for alert in self.alerts if alert.severity == "error")
        return RunTotals(
            files=len(self.prose) + len(self.scripts),
            lines=sum(len(self.added[path]) for path in self.prose + self.scripts),
            commits=len(self.commits),
            errors=errors,
            warnings=len(self.alerts) - errors,
        )


def _untracked_added(root: Path) -> dict[str, set[int]]:
    """Every line of every untracked file: a change adds an untracked file in full."""
    listed = _git(root, ["ls-files", "--others", "--exclude-standard", "-z"])
    added = {}
    for path in listed.split("\0"):
        if not path:
            continue
        count = len((root / path).read_bytes().splitlines())
        if count:
            added[path] = set(range(1, count + 1))
    return added


def _collect_run(engine: Engine, root: Path, base: str) -> Run:
    merge_base = _git(root, ["merge-base", base, "HEAD"]).strip()
    diff = _git(root, ["diff", "-U0", "--no-color", "--diff-filter=AM", merge_base])
    added = added_lines(diff)
    added.update(_untracked_added(root))

    linted = docs_files(root) + source_files(root)
    prose = [path for path in linted if path in added]
    scripts = [path for path in script_files(root) if path in added]
    commits = commits_in(root, f"{base}..HEAD")

    alerts = keep_added(_lint_whole_files(engine, root, prose), added)
    for path in scripts:
        alerts.extend(keep_added(_lint_comments(engine, root, path), added))
    for commit in commits:
        alerts.extend(_lint_commit(engine, root, commit))
    return Run(
        base=base,
        merge_base=merge_base,
        added=added,
        prose=prose,
        scripts=scripts,
        commits=commits,
        alerts=alerts,
    )


def _style_rule_counts(root: Path, styles: Sequence[str]) -> list[str]:
    counts = []
    for style in styles:
        rules = sorted((root / ".vale" / "styles" / style).glob("*.yml"))
        if rules:
            counts.append(f"style {style}: {_plural(len(rules), 'rule file')}")
        else:
            counts.append(f"style {style}: built into vale")
    return counts


def _print_group(name: str, lines: Iterable[str]) -> None:
    print(f"::group::{name}")
    for line in lines:
        print(line)
    print("::endgroup::")


def _print_detail(engine: Engine, root: Path, run: Run) -> None:
    totals = run.totals()
    config = json.loads(_run_vale(engine, root, ["ls-config"]))
    off = sorted(name for name, on in config["GChecks"].items() if not on)
    _print_group(
        "vale: range",
        [
            f"base ref: {run.base}",
            "merge base: " + _git(root, ["log", "-1", "--format=%h %s", run.merge_base]).strip(),
        ],
    )
    _print_group(
        "vale: engine",
        [f"command: {engine.description}", _run_vale(engine, root, ["--version"]).strip()],
    )
    _print_group(
        "vale: config",
        ["config: " + ", ".join(config["ConfigFiles"])]
        + _style_rule_counts(root, config["GBaseStyles"])
        + ["off: " + ", ".join(off)],
    )
    _print_group(
        f"vale: files checked ({_plural(totals.files, 'file')}, "
        f"{_plural(totals.lines, 'added line')})",
        run.surface_listing("prose", run.prose) + run.surface_listing("scripts", run.scripts),
    )
    _print_group(
        f"vale: commits checked ({_plural(totals.commits, 'commit')})",
        [f"  {commit.hash} {commit.subject}" for commit in run.commits],
    )
    by_rule = Counter(alert.check for alert in run.alerts).most_common()
    _print_group(
        "vale: alerts by rule",
        [f"{count:7d} {check}" for check, count in by_rule] or ["no alert on an added line"],
    )


def cmd_added(engine: Engine, root: Path, base: str) -> int:
    run = _collect_run(engine, root, base)
    for alert in run.alerts:
        print(alert.annotation())
    if os.environ.get("VALE_VERBOSE") or os.environ.get("GITHUB_ACTIONS"):
        _print_detail(engine, root, run)
    totals = run.totals()
    print(totals.summary(base))
    return 1 if totals.errors else 0


def _print_listing(alerts: Iterable[Alert]) -> None:
    for alert in alerts:
        print(alert.listing())


def _sync(engine: Engine, root: Path) -> int:
    return subprocess.run(engine.argv + ["sync"], cwd=root).returncode


def _ensure_synced(engine: Engine, root: Path) -> None:
    if not (root / ".vale" / "styles" / _PINNED_STYLE_PACKAGE).is_dir():
        _sync(engine, root)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo-root", type=Path, default=repo_root())
    sub = parser.add_subparsers(dest="cmd", required=True)
    sub.add_parser("sync", help="download the style packages .vale.ini pins")
    sub.add_parser("docs", help="lint the tracked Markdown files")
    sub.add_parser("source", help="lint the comments of the Rust, Python, C and C++ sources")
    sub.add_parser("scripts", help="lint the comments of justfiles, CMake files and workflows")
    commits = sub.add_parser("commits", help="lint every commit message in RANGE")
    commits.add_argument("range", nargs="?", default=f"{_DEFAULT_BASE}..HEAD")
    added = sub.add_parser("added", help="the gate: lint the lines added since BASE")
    added.add_argument("base", nargs="?", default=_DEFAULT_BASE)
    args = parser.parse_args()

    root: Path = args.repo_root.resolve()
    engine = select_engine(root)
    if args.cmd == "sync":
        sys.exit(_sync(engine, root))

    _ensure_synced(engine, root)
    if args.cmd == "docs":
        _print_listing(_lint_whole_files(engine, root, docs_files(root)))
    elif args.cmd == "source":
        _print_listing(_lint_whole_files(engine, root, source_files(root)))
    elif args.cmd == "scripts":
        alerts: list[Alert] = []
        for path in script_files(root):
            alerts.extend(_lint_comments(engine, root, path))
        _print_listing(alerts)
    elif args.cmd == "commits":
        message_alerts: list[Alert] = []
        for commit in commits_in(root, args.range):
            message_alerts.extend(_lint_commit(engine, root, commit))
        _print_listing(message_alerts)
    else:
        sys.exit(cmd_added(engine, root, args.base))

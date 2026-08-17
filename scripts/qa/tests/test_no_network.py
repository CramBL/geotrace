"""Tests for `qa.check_no_network`: keeping tests and examples off the network."""

import subprocess
from pathlib import Path

import pytest

from qa import check_no_network
from qa._check import repo_root


def _write(root: Path, rel: str, body: str) -> Path:
    path = root / rel
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(body)
    return path


def _init_repo(root: Path) -> None:
    subprocess.run(["git", "init", "-q"], cwd=root, check=True)
    subprocess.run(["git", "add", "-A"], cwd=root, check=True)


def test_flags_every_construct_in_a_test(tmp_path: Path) -> None:
    body = (
        "fn t() {\n"
        "    let a = HttpTransport::new(None);\n"
        "    let b = TransportSource::Network;\n"
        "    let c = reqwest::blocking::Client::new();\n"
        '    let d = "https://example.com";\n'
        "}\n"
    )
    _write(tmp_path, "crates/gt-x/tests/live.rs", body)
    _init_repo(tmp_path)

    assert [v[1] for v in check_no_network._collect(tmp_path)] == [2, 3, 4, 5]


def test_flags_an_example(tmp_path: Path) -> None:
    _write(tmp_path, "crates/gt-x/examples/fetch.rs", 'fn main() { get("http://a.b"); }\n')
    _init_repo(tmp_path)

    assert [v[1] for v in check_no_network._collect(tmp_path)] == [1]


def test_skips_production_code(tmp_path: Path) -> None:
    _write(tmp_path, "crates/gt-x/src/lib.rs", "fn f() { HttpTransport::new(None); }\n")
    _init_repo(tmp_path)

    assert check_no_network._collect(tmp_path) == []


def test_honors_an_exemption(tmp_path: Path) -> None:
    body = 'fn t() { let u = "https://a.b"; } // [qa-allow-check-no-network, reason = "ok"]\n'
    _write(tmp_path, "crates/gt-x/tests/live.rs", body)
    _init_repo(tmp_path)

    assert check_no_network._collect(tmp_path) == []


def test_an_allowlisted_file_is_exempt_for_its_constructs_alone(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    rel = "crates/gt-x/tests/archive.rs"
    body = 'fn t() {\n    let host = "https://a.b";\n    reqwest::get(host);\n}\n'
    _write(tmp_path, rel, body)
    _init_repo(tmp_path)
    monkeypatch.setitem(check_no_network._ALLOWED, rel, check_no_network._URL_LITERAL_ONLY)

    assert [v[1] for v in check_no_network._collect(tmp_path)] == [3]


def test_every_allowlisted_file_exists() -> None:
    root = repo_root()
    assert [rel for rel in sorted(check_no_network._ALLOWED) if not (root / rel).is_file()] == []

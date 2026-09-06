"""Tests for `qa.check_fixed_width_integers`: banning `short` and `long` in C sources."""

import subprocess
from pathlib import Path

from qa import check_fixed_width_integers


def _write(root: Path, rel: str, body: str) -> Path:
    path = root / rel
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(body)
    return path


def _init_repo(root: Path) -> None:
    subprocess.run(["git", "init", "-q"], cwd=root, check=True)
    subprocess.run(["git", "add", "-A"], cwd=root, check=True)


def test_flags_a_declaration_in_a_source_and_a_header(tmp_path: Path) -> None:
    _write(tmp_path, "sdk/c/examples/a.c", "static long seconds(void) {\n    return 0;\n}\n")
    _write(tmp_path, "sdk/c/examples/a.h", "struct Row {\n    unsigned short prn;\n};\n")
    _init_repo(tmp_path)

    violations = check_fixed_width_integers._collect(tmp_path)
    assert [(v[0].name, v[1]) for v in violations] == [("a.c", 1), ("a.h", 2)]


def test_allows_the_word_in_a_comment_and_a_literal(tmp_path: Path) -> None:
    body = (
        "/* A long London track, one fix every 30 s. */\n"
        "static void fail(void) {\n"
        '    puts("field too long"); // the long form\n'
        "}\n"
        "/* One more\n"
        "   long comment. */\n"
    )
    _write(tmp_path, "sdk/c/examples/a.c", body)
    _init_repo(tmp_path)

    assert check_fixed_width_integers._collect(tmp_path) == []


def test_reports_the_line_a_declaration_sits_on_after_a_block_comment(tmp_path: Path) -> None:
    body = "/* One\n   two\n   three */\nstatic long value = 0;\n"
    _write(tmp_path, "sdk/c/examples/a.c", body)
    _init_repo(tmp_path)

    assert [v[1] for v in check_fixed_width_integers._collect(tmp_path)] == [4]


def test_honors_the_exemption(tmp_path: Path) -> None:
    body = (
        "static long value(const char *text) {"
        ' // [qa-allow-check-fixed-width-integers, reason = "strtol returns long"]\n'
        "    return strtol(text, NULL, 10);\n"
        "}\n"
    )
    _write(tmp_path, "sdk/c/examples/a.c", body)
    _init_repo(tmp_path)

    assert check_fixed_width_integers._collect(tmp_path) == []


def test_leaves_cpp_sources_to_clang_tidy(tmp_path: Path) -> None:
    _write(tmp_path, "sdk/cpp/examples/a.cpp", "long seconds = 0;\n")
    _write(tmp_path, "sdk/cpp/include/a.hpp", "long seconds = 0;\n")
    _init_repo(tmp_path)

    assert check_fixed_width_integers._collect(tmp_path) == []

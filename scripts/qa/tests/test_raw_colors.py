"""Tests for `qa.check_raw_colors`: banning named chromatic Color32 constants."""

from pathlib import Path

from qa import check_raw_colors


def _write(root: Path, rel: str, body: str) -> Path:
    path = root / rel
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(body)
    return path


def _init_repo(root: Path) -> None:
    import subprocess

    subprocess.run(["git", "init", "-q"], cwd=root, check=True)
    subprocess.run(["git", "add", "-A"], cwd=root, check=True)


def test_flags_production_primary(tmp_path: Path) -> None:
    _write(tmp_path, "src/a.rs", "fn f() {\n    let c = Color32::YELLOW;\n}\n")
    _init_repo(tmp_path)
    violations = check_raw_colors._collect(tmp_path)
    assert [v[1] for v in violations] == [2]


def test_allows_neutral_and_from_rgb(tmp_path: Path) -> None:
    body = "fn f() {\n    let w = Color32::WHITE;\n    let g = Color32::from_rgb(1, 2, 3);\n}\n"
    _write(tmp_path, "src/a.rs", body)
    _init_repo(tmp_path)
    assert check_raw_colors._collect(tmp_path) == []


def test_skips_cfg_test_module(tmp_path: Path) -> None:
    body = (
        "fn f() {}\n"
        "#[cfg(test)]\n"
        "mod tests {\n"
        "    fn t() {\n"
        "        let c = Color32::BLUE;\n"
        "    }\n"
        "}\n"
    )
    _write(tmp_path, "src/a.rs", body)
    _init_repo(tmp_path)
    assert check_raw_colors._collect(tmp_path) == []


def test_scans_code_after_cfg_test_block(tmp_path: Path) -> None:
    body = (
        "#[cfg(test)]\n"
        "mod tests {\n"
        "    fn t() { let ok = Color32::BLUE; }\n"
        "}\n"
        "fn after() {\n"
        "    let bad = Color32::RED;\n"
        "}\n"
    )
    _write(tmp_path, "src/a.rs", body)
    _init_repo(tmp_path)
    assert [v[1] for v in check_raw_colors._collect(tmp_path)] == [6]


def test_honors_exemption(tmp_path: Path) -> None:
    body = 'fn f() {\n    let c = Color32::RED; // [qa-allow-check-raw-colors, reason = "ok"]\n}\n'
    _write(tmp_path, "src/a.rs", body)
    _init_repo(tmp_path)
    assert check_raw_colors._collect(tmp_path) == []


def test_skips_palette_crate(tmp_path: Path) -> None:
    _write(tmp_path, "crates/gt-ui-theme/src/lib.rs", "fn f() { let c = Color32::RED; }\n")
    _init_repo(tmp_path)
    assert check_raw_colors._collect(tmp_path) == []


def test_flags_unthemed_theme_constants(tmp_path: Path) -> None:
    body = (
        "fn f() {\n"
        "    a.color(gt_ui_theme::WARNING_AMBER);\n"
        "    b.color(ERROR_INDICATOR);\n"
        "}\n"
    )
    _write(tmp_path, "src/a.rs", body)
    _init_repo(tmp_path)
    assert [v[1] for v in check_raw_colors._collect(tmp_path)] == [2, 3]


def test_light_variant_constant_is_allowed(tmp_path: Path) -> None:
    # WARNING_AMBER_LIGHT is a distinct, deliberately-picked light colour, so the
    # word boundary must not flag it as a bare WARNING_AMBER use.
    _write(tmp_path, "src/a.rs", "fn f() { x(gt_ui_theme::WARNING_AMBER_LIGHT); }\n")
    _init_repo(tmp_path)
    assert check_raw_colors._collect(tmp_path) == []

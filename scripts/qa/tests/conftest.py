"""Fixtures the QA tests share."""

import subprocess
from collections.abc import Sequence
from dataclasses import dataclass
from pathlib import Path

import pytest


@dataclass(frozen=True)
class GitRepository:
    """An initialized repository under `root`, for a test that reads git."""

    root: Path

    def run(self, args: Sequence[str]) -> None:
        subprocess.run(
            ["git", "-c", "user.email=qa@example.com", "-c", "user.name=QA", *args],
            cwd=self.root,
            check=True,
            capture_output=True,
        )

    def commit_empty(self, message: str) -> None:
        self.run(["commit", "--quiet", "--allow-empty", "-m", message])


@pytest.fixture
def git_repository(tmp_path: Path) -> GitRepository:
    repository = GitRepository(tmp_path)
    repository.run(["init", "--quiet"])
    return repository

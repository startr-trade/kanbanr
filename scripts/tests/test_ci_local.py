"""Unit tests for scripts/ci_local.py (FEAT-198). Run by `make ci`, or:

    python3 -m unittest scripts/tests/test_ci_local.py
"""

from __future__ import annotations

import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import ci_local  # noqa: E402


def git(repo: Path, *args: str) -> str:
    return subprocess.run(
        ["git", *args], cwd=repo, check=True, capture_output=True, text=True
    ).stdout.strip()


class VerdictOnly(unittest.TestCase):
    """make ci lets a person's verdicts through and nothing else."""

    def setUp(self) -> None:
        self.tmp = tempfile.TemporaryDirectory()
        self.board = Path(self.tmp.name)
        git(self.board, "init", "-q", "-b", "main")
        git(self.board, "config", "user.name", "T")
        git(self.board, "config", "user.email", "t@x")
        git(self.board, "config", "commit.gpgsign", "false")
        self.write("projects/p/config.yaml", "name: p\n")
        self.write("projects/p/features/Planned/FEAT-1.yaml", "code: FEAT-1\n")
        self.commit("create project")
        self.start = git(self.board, "rev-parse", "HEAD")

    def tearDown(self) -> None:
        self.tmp.cleanup()

    def write(self, rel: str, text: str) -> None:
        path = self.board / rel
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text)

    def commit(self, subject: str) -> None:
        git(self.board, "add", "-A")
        git(self.board, "commit", "-q", "-m", subject)

    def head(self) -> str:
        return git(self.board, "rev-parse", "HEAD")

    def test_verdict_only(self) -> None:
        self.assertEqual(ci_local.verdict_only(self.board, self.start, self.start), [])

        self.write("projects/p/features/Planned/FEAT-1.yaml", "code: FEAT-1\napproved: yes\n")
        self.write("projects/p/activity/2026-10-20.yaml", "- approve\n")
        self.commit("approve definition of FEAT-1")
        self.write("projects/p/features/Planned/FEAT-1.yaml", "code: FEAT-1\nsigned: yes\n")
        self.commit("sign off design-review on FEAT-1")
        self.assertEqual(
            ci_local.verdict_only(self.board, self.start, self.head()),
            ["approve definition of FEAT-1", "sign off design-review on FEAT-1"],
        )

        # A verdict's subject on a commit that touches the project's config is not a verdict.
        middle = self.head()
        self.write("projects/p/config.yaml", "name: p\nchanged: yes\n")
        self.commit("sign off release on FEAT-1")
        self.assertIsNone(ci_local.verdict_only(self.board, middle, self.head()))

        # Anything else is not a verdict.
        later = self.head()
        self.write("projects/shop/config.yaml", "name: shop\n")
        self.commit("create project")
        self.assertIsNone(ci_local.verdict_only(self.board, later, self.head()))

    def test_rewritten_history_is_not_a_verdict(self) -> None:
        self.write("projects/p/features/Planned/FEAT-1.yaml", "code: FEAT-1\nx: 1\n")
        self.commit("sign off design-review on FEAT-1")
        first = self.head()
        git(self.board, "reset", "-q", "--hard", self.start)
        self.write("projects/p/features/Planned/FEAT-1.yaml", "code: FEAT-1\nx: 2\n")
        self.commit("sign off design-review on FEAT-1")
        self.assertIsNone(ci_local.verdict_only(self.board, first, self.head()))


if __name__ == "__main__":
    unittest.main()

"""Unit tests for scripts/check-tracked-files.sh (MNT-006). Run by `make ci`, or:

    python3 -m unittest scripts/tests/test_check_tracked_files.py
"""

from __future__ import annotations

import subprocess
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).resolve().parents[1] / "check-tracked-files.sh"


def git(repo: Path, *args: str) -> None:
    subprocess.run(["git", *args], cwd=repo, check=True, capture_output=True)


class CheckTrackedFiles(unittest.TestCase):
    def setUp(self) -> None:
        self.tmp = tempfile.TemporaryDirectory()
        self.repo = Path(self.tmp.name)
        git(self.repo, "init", "-q", "-b", "main")

    def tearDown(self) -> None:
        self.tmp.cleanup()

    def touch(self, rel: str) -> None:
        path = self.repo / rel
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text("x\n")

    def run_script(self) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            ["sh", str(SCRIPT)], cwd=self.repo, capture_output=True, text=True
        )

    def test_a_tracked_orig_or_rej_is_refused_and_named(self) -> None:
        self.touch("src/a.rs")
        self.touch("src/tests/b.rs.orig")
        self.touch("c.rej")
        git(self.repo, "add", "-A", "-f")
        out = self.run_script()
        self.assertEqual(out.returncode, 1, out.stdout + out.stderr)
        self.assertIn("tracked merge leftover: src/tests/b.rs.orig", out.stdout)
        self.assertIn("tracked merge leftover: c.rej", out.stdout)
        self.assertNotIn("a.rs", out.stdout)
        self.assertIn("git rm", out.stdout)

    def test_untracked_and_ignored_leftovers_and_a_clean_tree_pass(self) -> None:
        self.touch("src/a.rs")
        git(self.repo, "add", "-A")
        clean = self.run_script()
        self.assertEqual(clean.returncode, 0, clean.stdout + clean.stderr)
        self.assertIn("no tracked *.orig or *.rej files", clean.stdout)
        self.touch("loose.orig")  # untracked
        (self.repo / ".gitignore").write_text("*.rej\n")
        self.touch("ignored.rej")  # ignored
        git(self.repo, "add", ".gitignore")
        out = self.run_script()
        self.assertEqual(out.returncode, 0, out.stdout + out.stderr)


if __name__ == "__main__":
    unittest.main()

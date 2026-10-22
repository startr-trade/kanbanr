"""Unit tests for scripts/ci_local.py (FEAT-198, FEAT-218). Run by `make ci`, or:

    python3 -m unittest scripts/tests/test_ci_local.py
"""

from __future__ import annotations

import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import ci_local  # noqa: E402


def git(repo: Path, *args: str, at: str | None = None) -> str:
    env = {**os.environ, "GIT_COMMITTER_DATE": at, "GIT_AUTHOR_DATE": at} if at else None
    return subprocess.run(
        ["git", *args], cwd=repo, check=True, capture_output=True, text=True, env=env
    ).stdout.strip()


class Board(unittest.TestCase):
    """A scratch board repository with one project and one item."""

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

    def commit(self, subject: str, body: str = "", at: str | None = None) -> None:
        git(self.board, "add", "-A")
        message = ["-m", subject] + (["-m", body] if body else [])
        git(self.board, "commit", "-q", *message, at=at)

    def head(self) -> str:
        return git(self.board, "rev-parse", "HEAD")


class VerdictOnly(Board):
    """make ci lets a person's verdicts through and nothing else."""

    def test_verdict_only(self) -> None:
        self.assertEqual(ci_local.allowed_during_run(self.board, self.start, self.start), [])

        self.write("projects/p/features/Planned/FEAT-1.yaml", "code: FEAT-1\napproved: yes\n")
        self.write("projects/p/activity/2026-10-20.yaml", "- approve\n")
        self.commit("approve definition of FEAT-1")
        self.write("projects/p/features/Planned/FEAT-1.yaml", "code: FEAT-1\nsigned: yes\n")
        self.commit("sign off design-review on FEAT-1")
        self.assertEqual(
            ci_local.allowed_during_run(self.board, self.start, self.head()),
            ["verdict: approve definition of FEAT-1", "verdict: sign off design-review on FEAT-1"],
        )

        # A verdict's subject on a commit that touches the project's config is not a verdict.
        middle = self.head()
        self.write("projects/p/config.yaml", "name: p\nchanged: yes\n")
        self.commit("sign off release on FEAT-1")
        self.assertIsNone(ci_local.allowed_during_run(self.board, middle, self.head()))

        # Anything else is not a verdict.
        later = self.head()
        self.write("projects/shop/config.yaml", "name: shop\n")
        self.commit("create project")
        self.assertIsNone(ci_local.allowed_during_run(self.board, later, self.head()))

    def test_rewritten_history_is_not_a_verdict(self) -> None:
        self.write("projects/p/features/Planned/FEAT-1.yaml", "code: FEAT-1\nx: 1\n")
        self.commit("sign off design-review on FEAT-1")
        first = self.head()
        git(self.board, "reset", "-q", "--hard", self.start)
        self.write("projects/p/features/Planned/FEAT-1.yaml", "code: FEAT-1\nx: 2\n")
        self.commit("sign off design-review on FEAT-1")
        self.assertIsNone(ci_local.allowed_during_run(self.board, first, self.head()))


SCOPES = """- id: SC-1
  role: build
  item: FEAT-1
  operations: [task.state, test.state, commit]
  issued_at: 2026-10-22T01:00:00.123456789Z
  expires_at: 2026-10-22T09:00:00.123456789Z
  claimed_at: 2026-10-22T01:01:00.5Z
- id: SC-2
  role: design
  item: FEAT-1
  operations: []
  issued_at: 2026-10-22T01:00:00Z
  expires_at: 2026-10-22T09:00:00Z
"""
TRAILER = "Agent-Scope: SC-1 (build, by the agent for FEAT-1)"


class ScopedWrites(Board):
    """make ci lets a build agent's own progress records through, under an active scope (FEAT-218)."""

    def setUp(self) -> None:
        super().setUp()
        self.write("projects/p/features/Planned/FEAT-2.yaml", "code: FEAT-2\n")
        self.write("projects/p/scopes.yaml", "[]\n")
        self.commit("create FEAT-2", at="2026-10-22T00:00:00Z")
        self.start = self.head()

    def agent_task(self, n: int, at: str = "2026-10-22T02:00:00Z", body: str = TRAILER) -> None:
        self.write("projects/p/features/Planned/FEAT-1.yaml", f"code: FEAT-1\ntask: {n}\n")
        self.write("projects/p/activity/2026-10-22.yaml", f"- task {n}\n")
        self.write("projects/p/index.yaml", f"n: {n}\n")
        self.commit(f"update task T{n} (FEAT-1/TL-001)", body, at)

    def test_scoped_agent_writes_pass(self) -> None:
        self.write("projects/p/scopes.yaml", SCOPES)
        self.write("projects/p/activity/2026-10-22.yaml", "- issue\n")
        self.commit("issue agent scope SC-1 (build, FEAT-1)", at="2026-10-22T01:00:00Z")
        self.agent_task(1, at="2026-10-22T01:01:00Z")  # the claim's own second
        self.write("projects/p/features/Planned/FEAT-1.yaml", "code: FEAT-1\ngreen: yes\n")
        self.write("projects/p/events/2026-10.yaml", "- test\n")
        self.commit("test unit_one of FEAT-1/R-1 is now green", TRAILER, "2026-10-22T02:10:00Z")
        self.write("projects/p/features/Planned/FEAT-1.yaml", "code: FEAT-1\nrun: yes\n")
        self.commit("record 2 test result(s) from a run", TRAILER, "2026-10-22T02:15:00Z")
        self.write("projects/p/features/Planned/FEAT-1.yaml", "code: FEAT-1\napproved: yes\n")
        self.commit("approve definition of FEAT-1", at="2026-10-22T02:20:00Z")
        self.write("projects/p/scopes.yaml", SCOPES + "  # revoked\n")
        self.commit("revoke agent scope SC-1: done", at="2026-10-22T02:30:00Z")
        self.assertEqual(
            ci_local.allowed_during_run(self.board, self.start, self.head()),
            [
                "verdict: issue agent scope SC-1 (build, FEAT-1)",
                "SC-1: update task T1 (FEAT-1/TL-001)",
                "SC-1: test unit_one of FEAT-1/R-1 is now green",
                "SC-1: record 2 test result(s) from a run",
                "verdict: approve definition of FEAT-1",
                "verdict: revoke agent scope SC-1: done",
            ],
        )

    def assert_fails(self, why: str) -> None:
        self.assertIsNone(ci_local.allowed_during_run(self.board, self.base, self.head()), why)
        git(self.board, "reset", "-q", "--hard", self.base)

    def test_agent_writes_outside_their_scope_fail(self) -> None:
        self.write("projects/p/scopes.yaml", SCOPES)
        self.commit("issue agent scope SC-1 (build, FEAT-1)", at="2026-10-22T01:00:00Z")
        self.base = self.head()

        self.agent_task(1, body="Agent-Scope: SC-9 (build, by the agent for FEAT-1)")
        self.assert_fails("an unknown scope")
        self.agent_task(1, at="2026-10-22T01:00:30Z")
        self.assert_fails("before it was claimed")
        self.agent_task(1, at="2026-10-22T10:00:00Z")
        self.assert_fails("after it expired")
        self.agent_task(1, body="Agent-Scope: SC-2 (design, by the agent for FEAT-1)")
        self.assert_fails("a design scope")
        self.agent_task(1, body="")
        self.assert_fails("no trailer")

        self.write("projects/p/features/Planned/FEAT-2.yaml", "code: FEAT-2\nx: 1\n")
        self.commit("update task T1 (FEAT-1/TL-001)", TRAILER, "2026-10-22T02:00:00Z")
        self.assert_fails("another item's file")
        self.write("projects/p/features/Planned/FEAT-1.yaml", "code: FEAT-1\nx: 1\n")
        self.write("projects/p/docs/notes.md", "x\n")
        self.commit("update task T1 (FEAT-1/TL-001)", TRAILER, "2026-10-22T02:00:00Z")
        self.assert_fails("a docs file")
        self.write("projects/p/features/Planned/FEAT-1.yaml", "code: FEAT-1\nx: 1\n")
        self.commit("move FEAT-1 to In Progress", TRAILER, "2026-10-22T02:00:00Z")
        self.assert_fails("not a task or test update")
        self.write("projects/p/features/Planned/FEAT-1.yaml", "code: FEAT-1\nx: 1\n")
        self.commit("update task T1 (FEAT-2/TL-001)", TRAILER, "2026-10-22T02:00:00Z")
        self.assert_fails("another item's task")

        self.write("projects/p/scopes.yaml", SCOPES.replace(
            "  claimed_at: 2026-10-22T01:01:00.5Z\n",
            "  claimed_at: 2026-10-22T01:01:00.5Z\n  revoked_at: 2026-10-22T01:30:00Z\n",
        ))
        self.commit("revoke agent scope SC-1: done", at="2026-10-22T01:30:00Z")
        self.base = self.head()
        self.agent_task(1)
        self.assert_fails("after it was revoked")

        self.write("projects/p/scopes.yaml", SCOPES)
        self.write("projects/p/config.yaml", "name: p\nx: 1\n")
        self.commit("issue agent scope SC-3 (build, FEAT-1)", at="2026-10-22T01:40:00Z")
        self.assert_fails("a scope act that touches the config")


if __name__ == "__main__":
    unittest.main()

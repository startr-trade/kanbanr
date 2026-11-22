"""Unit tests for scripts/ci_local.py (FEAT-198, FEAT-218). Run by `make ci`, or:

    python3 -m unittest scripts/tests/test_ci_local.py
"""

from __future__ import annotations

import os
import subprocess
import sys
import tempfile
import unittest
from unittest import mock
from datetime import datetime, timezone
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

    def capture_record(self, body: str) -> list[str] | None:
        self.write("projects/p/scopes.yaml", SCOPES)
        self.commit("issue agent scope SC-1 (build, FEAT-1)", at="2026-10-22T01:00:00Z")
        base = self.head()
        self.write("projects/p/features/Planned/FEAT-1.yaml", "code: FEAT-1\nrun: yes\n")
        self.write("projects/p/events/2026-10.yaml", "- run\n")
        self.commit("record 6 test result(s) from a run", body, "2026-10-22T02:15:00Z")
        return ci_local.allowed_during_run(self.board, base, self.head())

    def test_a_capture_record_with_the_trailer_passes(self) -> None:
        allowed = self.capture_record(TRAILER)
        self.assertIsNotNone(allowed)
        self.assertIn("SC-1: record 6 test result(s) from a run", allowed or [])

    def test_a_capture_record_with_a_timings_file_passes(self) -> None:
        """FTR-058 R-10: the run's timings file is part of a capture record; a gates file is not."""
        self.write("projects/p/scopes.yaml", SCOPES)
        self.commit("issue agent scope SC-1 (build, FEAT-1)", at="2026-10-22T01:00:00Z")
        base = self.head()
        self.write("projects/p/features/Planned/FEAT-1.yaml", "code: FEAT-1\nrun: yes\n")
        self.write("projects/p/timings/20261022T021500Z-0123456.yaml", "rows: []\n")
        self.commit("record 6 test result(s) from a run", TRAILER, "2026-10-22T02:15:00Z")
        allowed = ci_local.allowed_during_run(self.board, base, self.head())
        self.assertIn("SC-1: record 6 test result(s) from a run", allowed or [])
        self.write("projects/p/gates/x.yaml", "gate: x\n")
        self.commit("record 6 test result(s) from a run", TRAILER, "2026-10-22T02:16:00Z")
        self.assertIsNone(ci_local.allowed_during_run(self.board, base, self.head()))

    def test_an_unmarked_capture_record_still_fails(self) -> None:
        self.assertIsNone(self.capture_record(""))

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


class ImageBuild(unittest.TestCase):
    """The release image's package layer is never taken from the cache (MNT-006)."""

    def dockerfile(self) -> str:
        return (ci_local.ROOT / "docker/Dockerfile").read_text()

    def test_the_gate_pulls_and_refreshes_the_package_layer(self) -> None:
        command = ci_local.image_build_command("abc", "2026-11-08", "tok")
        self.assertIn("--pull", command)
        self.assertIn("KANBANR_APT_REFRESH=tok", command)
        self.assertEqual(command[:2], ["docker", "build"])

    def test_each_run_refreshes_with_a_new_value(self) -> None:
        self.assertNotEqual(ci_local.refresh_token(), ci_local.refresh_token())

    def test_the_dockerfile_declares_the_refresh_before_its_package_layer(self) -> None:
        text = self.dockerfile()
        self.assertIsNone(ci_local.dockerfile_refreshes_packages(text))
        declaration = 'ARG KANBANR_APT_REFRESH=""\n'
        self.assertIn(declaration, text)
        missing = text.replace(declaration, "")
        self.assertIn("does not declare", ci_local.dockerfile_refreshes_packages(missing) or "")
        run = text.index("RUN echo \"package refresh")
        after = text.replace(declaration, "")
        end = after.rindex("rm -rf /var/lib/apt/lists/*\n") + len("rm -rf /var/lib/apt/lists/*\n")
        after = after[:end] + declaration + after[end:]
        self.assertIn("after the apt-get upgrade", ci_local.dockerfile_refreshes_packages(after) or "")
        unused = text.replace('RUN echo "package refresh: ${KANBANR_APT_REFRESH:-none}"', "RUN true")
        self.assertNotEqual(unused, text)
        self.assertIn("does not use", ci_local.dockerfile_refreshes_packages(unused) or "")
        self.assertGreater(run, 0)


class RecordsItsVerdict(unittest.TestCase):
    """`make ci` records its own verdict as the gate `local` for HEAD (FTR-002 R-4)."""

    HEAD = "a" * 40

    def test_a_pass_on_a_clean_tree_is_recorded(self) -> None:
        args = ci_local.gate_record_args(True, self.HEAD, False, [])
        self.assertEqual(args[:3], ["gate", "record", "local"])
        self.assertIn("--passed", args)
        self.assertNotIn("--failed", args)
        self.assertEqual(args[args.index("--source") + 1], "local")
        self.assertEqual(args[args.index("--commit") + 1], self.HEAD)

    def test_a_failure_names_the_failing_steps(self) -> None:
        args = ci_local.gate_record_args(False, self.HEAD, False, ["lint", "tests"])
        self.assertIn("--failed", args)
        self.assertNotIn("--passed", args)
        self.assertEqual(args[args.index("--note") + 1], "failed: lint, tests")


class NoPassForADirtyTree(unittest.TestCase):
    """A pass for a tree with uncommitted changes is not about HEAD (FTR-002 R-4)."""

    HEAD = "b" * 40

    def test_no_pass_is_recorded(self) -> None:
        self.assertIsNone(ci_local.gate_record_args(True, self.HEAD, True, []))

    def test_a_failure_is_still_recorded(self) -> None:
        args = ci_local.gate_record_args(False, self.HEAD, True, ["tests"])
        self.assertIn("--failed", args)


class CommandLine(unittest.TestCase):
    """The script's command line (DEF-010): help and a mistyped flag never start the gate."""

    SCRIPT = Path(ci_local.__file__).resolve()
    FIRST_LINE = "make ci: a clean copy"

    def run_script(self, *cmd: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            list(cmd), cwd=ci_local.ROOT, capture_output=True, text=True, timeout=30
        )

    def test_help_prints_usage_and_runs_nothing(self) -> None:
        for flag in ("--help", "-h"):
            r = self.run_script(sys.executable, str(self.SCRIPT), flag)
            self.assertEqual(r.returncode, 0, r.stderr)
            self.assertIn("usage:", r.stdout)
            self.assertNotIn(self.FIRST_LINE, r.stdout)

    def test_unknown_argument_is_refused_and_runs_nothing(self) -> None:
        r = self.run_script(sys.executable, str(self.SCRIPT), "--bogus")
        self.assertEqual(r.returncode, 2)
        self.assertIn("unrecognized arguments: --bogus", r.stderr)
        self.assertNotIn(self.FIRST_LINE, r.stdout)

    def test_ci_local_sh_passes_help_through(self) -> None:
        r = self.run_script("sh", "scripts/ci-local.sh", "--help")
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertIn("usage:", r.stdout)


class WorkflowsAreClassified(unittest.TestCase):
    """DEF-052 R-2: make ci knows every workflow step and every repository variable."""

    WORKFLOWS = ci_local.ROOT / ".github" / "workflows"

    def test_ci_local_every_step_is_classified_and_every_vars_value_has_a_local_stand_in(self) -> None:
        import re

        import yaml

        files = sorted(self.WORKFLOWS.glob("*.yml"))
        unclassified = []
        for path in files:
            plan = ci_local.PLAN.get(path.name)
            if plan is None:
                unclassified.append(f"{path.name}: the workflow is not in PLAN")
                continue
            for job_id, job in (yaml.safe_load(path.read_text()).get("jobs") or {}).items():
                for step in job.get("steps", []):
                    if "run" in step and step.get("name") not in plan.get(job_id, {}):
                        unclassified.append(f"{path.name} {job_id}: {step.get('name')}")
        self.assertEqual(unclassified, [], "steps not classified in ci_local.PLAN")
        names = {n for p in files for n in re.findall(r"\bvars\.([A-Za-z_][A-Za-z0-9_]*)", p.read_text())}
        missing = sorted(n for n in names if f"vars.{n}" not in ci_local.LOCAL_VARS)
        self.assertEqual(missing, [], "vars.* with no local stand-in in ci_local.LOCAL_VARS")


STUB_CARGO = """#!/bin/sh
printf '%s\\n' "$*" >> "$STUB_LOG"
case "${1:-} ${2:-}" in
    "nextest --version") exit "${STUB_NEXTEST_VERSION:-0}" ;;
    "nextest run") exit "${STUB_NEXTEST_RUN:-0}" ;;
esac
exit 0
"""


@unittest.skipIf(os.name == "nt", "scripts/test.sh is POSIX sh; the Windows runner runs it through Git Bash in CI")
class TestScript(unittest.TestCase):
    """MNT-010 R-1 to R-3: scripts/test.sh picks nextest or cargo test, and doctests follow nextest."""

    SCRIPT = ci_local.ROOT / "scripts" / "test.sh"
    VERSION = "nextest --version"
    NEXTEST = "nextest run --locked --workspace --no-fail-fast"
    DOCTESTS = "test --locked --workspace --doc"
    CARGO = "test --locked --workspace --no-fail-fast"

    def setUp(self) -> None:
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.bin = Path(tmp.name) / "bin"
        self.bin.mkdir()
        stub = self.bin / "cargo"
        stub.write_text(STUB_CARGO)
        stub.chmod(0o755)
        self.log = Path(tmp.name) / "cargo.log"

    def run_script(self, *args: str, nextest: bool = True, run: int = 0, **env: str) -> subprocess.CompletedProcess[str]:
        self.log.write_text("")
        environ = {
            "PATH": f"{self.bin}:/usr/bin:/bin",
            "STUB_LOG": str(self.log),
            "STUB_NEXTEST_VERSION": "0" if nextest else "1",
            "STUB_NEXTEST_RUN": str(run),
            **env,
        }
        return subprocess.run(
            ["sh", str(self.SCRIPT), *args], capture_output=True, text=True, env=environ, timeout=30
        )

    def logged(self) -> list[str]:
        return self.log.read_text().splitlines()

    def test_nextest_runs_when_it_answers_and_doctests_follow(self) -> None:
        r = self.run_script()
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertEqual(self.logged(), [self.VERSION, self.NEXTEST, self.DOCTESTS])
        self.assertIn("test.sh: cargo-nextest", r.stdout)
        r = self.run_script("-p", "kanbanr-core", CARGO_BUILD_JOBS="3")
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertEqual(
            self.logged(),
            [self.VERSION, f"{self.NEXTEST} --build-jobs 3 -p kanbanr-core", f"{self.DOCTESTS} --jobs 3"],
        )

    def test_cargo_test_runs_when_nextest_is_absent(self) -> None:
        r = self.run_script(nextest=False)
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertEqual(self.logged(), [self.VERSION, self.CARGO])
        self.assertIn("test.sh: cargo test (cargo-nextest not installed)", r.stdout)
        r = self.run_script(nextest=False, CARGO_BUILD_JOBS="3")
        self.assertEqual(self.logged(), [self.VERSION, f"{self.CARGO} --jobs 3"])

    def test_doctests_run_after_a_failed_nextest_run_and_the_script_fails(self) -> None:
        r = self.run_script(run=100)
        self.assertEqual(r.returncode, 100)
        self.assertEqual(self.logged(), [self.VERSION, self.NEXTEST, self.DOCTESTS])

    def test_the_runner_can_be_chosen(self) -> None:
        r = self.run_script(KANBANR_TEST_RUNNER="cargo")
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertEqual(self.logged(), [self.CARGO])
        r = self.run_script(nextest=False, KANBANR_TEST_RUNNER="nextest")
        self.assertEqual(r.returncode, 2)
        self.assertIn("cargo-nextest is not installed: cargo install cargo-nextest --locked", r.stderr)
        self.assertNotIn(self.NEXTEST, self.logged())
        r = self.run_script(KANBANR_TEST_RUNNER="other")
        self.assertEqual(r.returncode, 2)
        self.assertIn("KANBANR_TEST_RUNNER is nextest or cargo", r.stderr)
        self.assertEqual(self.logged(), [])

    def test_the_ignored_suite_runs_through_the_script_both_ways(self) -> None:
        args = ("--ignored", "-p", "kanbanr-cli", "--test", "build_stamp")
        r = self.run_script(*args)
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertEqual(
            self.logged(),
            [self.VERSION, "nextest run --locked --no-fail-fast --run-ignored only -p kanbanr-cli --test build_stamp"],
        )
        r = self.run_script(*args, nextest=False)
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertEqual(self.logged(), [self.VERSION, "test --locked -p kanbanr-cli --test build_stamp -- --ignored"])


class KeptRuns(unittest.TestCase):
    """MNT-010 R-6: make ci keeps the Test step's full output and its JUnit report, one folder a run."""

    HEAD = "0123456789abcdef0123456789abcdef01234567"

    def setUp(self) -> None:
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.tmp = Path(tmp.name)
        self.runs = self.tmp / "runs"
        self.now = datetime(2026, 11, 17, 9, 5, 7, tzinfo=timezone.utc)

    def test_the_test_steps_output_and_report_are_kept_per_run(self) -> None:
        junit = self.tmp / "junit.xml"
        junit.write_text('<testsuites><testsuite name="b"><testcase name="t" time="0.1"/></testsuite></testsuites>')
        output = "PASS [   0.100s] (1/1) b t\nsummé\n"
        first = ci_local.keep_run(output, junit, self.HEAD, self.now, self.runs)
        self.assertEqual(first, self.runs / "20261117T090507Z-0123456")
        self.assertEqual((first / "test.log").read_text(encoding="utf-8"), output)
        self.assertEqual((first / "junit.xml").read_text(), junit.read_text())
        self.assertFalse((first / "junit.missing").exists())
        second = ci_local.keep_run("again\n", junit, self.HEAD, self.now, self.runs)
        self.assertEqual(second, self.runs / "20261117T090507Z-0123456-2")
        self.assertEqual((first / "test.log").read_text(encoding="utf-8"), output)
        self.assertEqual((second / "test.log").read_text(encoding="utf-8"), "again\n")
        third = ci_local.keep_run("third\n", junit, self.HEAD, self.now, self.runs)
        self.assertEqual(third, self.runs / "20261117T090507Z-0123456-3")
        self.assertEqual(sorted(p.name for p in self.runs.iterdir()),
                         ["20261117T090507Z-0123456", "20261117T090507Z-0123456-2", "20261117T090507Z-0123456-3"])

    def test_a_run_without_a_report_says_so_in_its_folder(self) -> None:
        kept = ci_local.keep_run("cargo test output\n", self.tmp / "absent.xml", self.HEAD, self.now, self.runs)
        self.assertEqual((kept / "test.log").read_text(encoding="utf-8"), "cargo test output\n")
        self.assertFalse((kept / "junit.xml").exists())
        self.assertEqual((kept / "junit.missing").read_text(),
                         "no JUnit report: the step ran cargo test, which writes none")


    def test_a_clean_run_captures_its_kept_report_at_head(self) -> None:
        """FTR-058 R-10: the kept JUnit report is captured at HEAD, only on a clean tree."""
        folder = self.tmp / "run"
        folder.mkdir()
        self.assertIsNone(ci_local.capture_kept_run(folder, self.HEAD))
        (folder / "junit.xml").write_text("<testsuites/>")
        self.assertEqual(
            ci_local.capture_kept_run(folder, self.HEAD),
            ["capture", "--junit", str(folder / "junit.xml"), "--rev", self.HEAD],
        )
        calls: list[list[str]] = []

        def fake_sh(cmd, cwd, env=None, **_):
            calls.append(list(cmd))
            if cmd[:2] == ["git", "rev-parse"]:
                return True, self.HEAD + "\n"
            if cmd[:2] == ["git", "status"]:
                return True, dirty[0]
            return True, "kanbanr: recorded 3 test result(s) and 3 duration(s)\n"

        dirty = [""]
        with mock.patch.object(ci_local, "sh", fake_sh), mock.patch.object(
            ci_local.shutil, "which", return_value="/bin/kanbanr"
        ):
            ci_local.record_kept_run(folder)
            self.assertEqual(sum(c[0] == "/bin/kanbanr" for c in calls), 1)
            self.assertIn("--junit", calls[-1])
            calls.clear()
            dirty = [" M x\n"]
            ci_local.record_kept_run(folder)
            self.assertEqual(sum(c[0] == "/bin/kanbanr" for c in calls), 0)


class HeavySteps(unittest.TestCase):
    """R-5 (MNT-007): heavy steps take the machine's build lock and a memory cap when the tools exist."""

    SESSION = {"XDG_RUNTIME_DIR": "/run/user/1000"}

    def setUp(self) -> None:
        scratch = tempfile.TemporaryDirectory()
        self.addCleanup(scratch.cleanup)
        self.meminfo = Path(scratch.name) / "meminfo"
        # 40 GiB, so 60% of it is 24 GiB.
        self.meminfo.write_text("MemTotal:       41943040 kB\nMemFree:         1000000 kB\n")
        patches = [
            mock.patch.object(ci_local, "MEMINFO", self.meminfo),
            mock.patch.object(ci_local.os, "cpu_count", return_value=12),
            mock.patch.dict(ci_local.os.environ, {}, clear=False),
        ]
        for patch in patches:
            patch.start()
            self.addCleanup(patch.stop)
        for var in ("KANBANR_CI_MEMORY_MAX", "CARGO_BUILD_JOBS"):
            ci_local.os.environ.pop(var, None)

    def which(self, *present: str):
        return mock.patch.object(ci_local.shutil, "which",
                                 side_effect=lambda tool: f"/usr/bin/{tool}" if tool in present else None)

    @unittest.skipUnless(ci_local.shutil.which("systemd-run") and os.environ.get("XDG_RUNTIME_DIR"),
                         "needs a systemd user session")
    def test_a_heavy_step_reaches_bash_with_its_shell_variables_unexpanded(self) -> None:
        # release.yml's tag check uses ${tag#v}; systemd-run 254+ expanded it to "" (v0.8.0 gate).
        script = 'tag=v1.2.3; echo "[${tag#v}] [$tag]"'
        cmd = ci_local.heavy(["bash", "--noprofile", "--norc", "-c", script], dict(os.environ))
        cmd = [c for c in cmd if c not in ("flock", str(ci_local.LOCK))]  # the probe needs no lock
        out = subprocess.run(cmd, capture_output=True, text=True, timeout=60)
        self.assertEqual(out.stdout.strip(), "[1.2.3] [v1.2.3]", out.stderr)

    def test_heavy_steps_take_the_lock_and_the_memory_cap_when_the_tools_exist(self) -> None:
        cmd = ["bash", "-c", "cargo test"]
        cap = str(int(41943040 * 1024 * 0.6))
        with self.which("flock", "systemd-run"), \
                mock.patch.object(ci_local, "systemd_run_expands", return_value=True):
            self.assertEqual(ci_local.heavy(cmd, self.SESSION),
                             ["systemd-run", "--user", "--scope", "-q", "--expand-environment=no",
                              "-p", f"MemoryMax={cap}",
                              "flock", str(Path.home() / ".cache/heavy-build.lock"), *cmd])
            self.assertIsNone(ci_local.heavy_notice(self.SESSION))
            ci_local.os.environ["KANBANR_CI_MEMORY_MAX"] = "8G"
            self.assertIn("MemoryMax=8G", ci_local.heavy(cmd, self.SESSION))
        del ci_local.os.environ["KANBANR_CI_MEMORY_MAX"]
        self.meminfo.unlink()
        with self.which("flock", "systemd-run"):
            self.assertIn("MemoryMax=max", ci_local.heavy(cmd, self.SESSION))

        # Which steps are heavy: Rust, the monitor's build and the image; not a Python check.
        self.assertTrue(ci_local.is_heavy("cargo clippy --locked --workspace --all-targets"))
        self.assertTrue(ci_local.is_heavy("cd web\nnpm ci\nnpm run build"))
        self.assertTrue(ci_local.is_heavy("docker build -f docker/Dockerfile ."))
        # The Rust tests run through scripts/test.sh (MNT-010), whose text names no cargo.
        self.assertTrue(ci_local.is_heavy("../scripts/test.sh"))
        self.assertTrue(ci_local.is_heavy("../scripts/test.sh --ignored -p kanbanr-cli --test build_stamp"))
        self.assertFalse(ci_local.is_heavy("python3 scripts/check_supply_chain.py"))
        self.assertFalse(ci_local.is_heavy("npm test"))

        # Two CPUs are left free unless CARGO_BUILD_JOBS says otherwise; never fewer than one job.
        self.assertEqual(ci_local.build_jobs(), "10")
        with mock.patch.object(ci_local.os, "cpu_count", return_value=2):
            self.assertEqual(ci_local.build_jobs(), "1")
        ci_local.os.environ["CARGO_BUILD_JOBS"] = "4"
        self.assertEqual(ci_local.build_jobs(), "4")

        # sh(heavy=True) runs the wrapped command; sh() without it runs the command as given.
        with self.which("flock", "systemd-run"), \
                mock.patch.object(ci_local.subprocess, "run",
                                  return_value=subprocess.CompletedProcess([], 0, "")) as run:
            ci_local.sh(["cargo", "build"], ci_local.ROOT, dict(self.SESSION), heavy=True)
            self.assertEqual(run.call_args.args[0][0], "systemd-run")
            self.assertEqual(run.call_args.args[0][-2:], ["cargo", "build"])
            ci_local.sh(["python3", "-V"], ci_local.ROOT, dict(self.SESSION))
            self.assertEqual(run.call_args.args[0], ["python3", "-V"])

    def test_heavy_steps_run_plainly_without_flock_or_systemd_run(self) -> None:
        cmd = ["bash", "-c", "cargo test"]
        with self.which():
            self.assertEqual(ci_local.heavy(cmd, self.SESSION), cmd)
            notice = ci_local.heavy_notice(self.SESSION)
            self.assertIn("no flock", notice)
            self.assertIn("no systemd-run", notice)
            self.assertNotIn("\n", notice)
        # A user manager is needed for the cap: systemd-run alone, without a session, is not enough.
        with self.which("flock", "systemd-run"):
            self.assertEqual(ci_local.heavy(cmd, {}), ["flock", str(ci_local.LOCK), *cmd])
            self.assertIn("memory cap", ci_local.heavy_notice({}))
            self.assertNotIn("build lock", ci_local.heavy_notice({}))
        with self.which("systemd-run"):
            wrapped = ci_local.heavy(cmd, self.SESSION)
            self.assertEqual(wrapped[0], "systemd-run")
            self.assertNotIn("flock", wrapped)
            self.assertEqual(wrapped[-3:], cmd)

    def test_the_deploy_e2e_steps_hold_the_lock_and_their_image_build_does_not_wait_on_it(self) -> None:
        """FTR-030 T8: kind and compose runs load the machine like a build, so they hold the lock.

        Their image build takes the lock itself (deploy/tests/lib.sh ensure_image) when run by hand;
        under make ci the step already holds it, and a second flock on the same file would wait forever.
        """
        self.assertTrue(ci_local.is_heavy("deploy/tests/e2e-kind.sh"))
        self.assertTrue(ci_local.is_heavy("deploy/tests/compose-smoke.sh"))
        self.assertFalse(ci_local.is_heavy("deploy/tests/validate.sh"))
        self.assertFalse(ci_local.is_heavy("deploy/tests/compose-config.sh"))
        with self.which("flock"), \
                mock.patch.object(ci_local.subprocess, "run",
                                  return_value=subprocess.CompletedProcess([], 0, "")) as run:
            ci_local.sh(["bash", "-c", "deploy/tests/e2e-kind.sh"], ci_local.ROOT, {"PATH": "/bin"}, heavy=True)
            self.assertEqual(run.call_args.args[0][0], "flock")
            self.assertEqual(run.call_args.kwargs["env"].get("KANBANR_HEAVY_LOCK_HELD"), "1")
            ci_local.sh(["bash", "-c", "python3 -V"], ci_local.ROOT, {"PATH": "/bin"})
            self.assertNotIn("KANBANR_HEAVY_LOCK_HELD", run.call_args.kwargs["env"])
        with self.which(), \
                mock.patch.object(ci_local.subprocess, "run",
                                  return_value=subprocess.CompletedProcess([], 0, "")) as run:
            ci_local.sh(["bash", "-c", "deploy/tests/e2e-kind.sh"], ci_local.ROOT, {"PATH": "/bin"}, heavy=True)
            self.assertNotIn("KANBANR_HEAVY_LOCK_HELD", run.call_args.kwargs["env"])

    @unittest.skipIf(os.name == "nt", "deploy/tests/lib.sh is bash; the deploy checks run on Linux")
    def test_lib_sh_ensure_image_takes_the_lock_unless_its_caller_holds_it(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            bin_dir, log = Path(tmp) / "bin", Path(tmp) / "calls.log"
            bin_dir.mkdir()
            for tool in ("flock", "docker"):
                stub = bin_dir / tool
                stub.write_text(f'#!/bin/sh\necho "{tool} $*" >> "$STUB_LOG"\n'
                                + ('shift; exec "$@"\n' if tool == "flock" else "exit 0\n"))
                stub.chmod(0o755)
            lib = ci_local.ROOT / "deploy" / "tests" / "lib.sh"

            def ensure_image(**extra: str) -> list[str]:
                log.write_text("")
                env = {"PATH": f"{bin_dir}:/usr/bin:/bin", "HOME": tmp, "STUB_LOG": str(log), **extra}
                subprocess.run(["bash", "-c", f'. "{lib}" && ensure_image'], env=env, check=True, timeout=30)
                return log.read_text().splitlines()

            calls = ensure_image()
            self.assertTrue(calls[0].startswith("flock "), calls)
            self.assertTrue(any(c.startswith("docker build") for c in calls), calls)
            calls = ensure_image(KANBANR_HEAVY_LOCK_HELD="1")
            self.assertFalse(any(c.startswith("flock") for c in calls), calls)
            self.assertTrue(any(c.startswith("docker build") for c in calls), calls)


class VarsAreVariables(unittest.TestCase):
    """A gh 403 is "cannot verify", other gh failures still fail (MNT-007 T10)."""

    def run_check(self, listing: tuple[bool, str]):
        calls = []

        def fake_sh(cmd, cwd, *a, **k):
            calls.append(cmd)
            if cmd[:2] == ["gh", "api"]:
                return True, "someone\n"
            return listing

        said = []
        with mock.patch.object(ci_local, "sh", fake_sh), \
                mock.patch.object(ci_local, "say", lambda *a, **k: said.append(a)), \
                mock.patch.object(ci_local, "not_verified", []) as nv:
            ci_local.vars_are_variables()
            return said, list(nv)

    def test_a_403_is_not_verified_not_a_failure(self):
        said, nv = self.run_check((False, "HTTP 403: Resource not accessible"))
        self.assertEqual(said, [])
        self.assertEqual(len(nv), 1)
        self.assertIn("someone", nv[0])
        self.assertIn("admin", nv[0])

    def test_another_error_still_fails(self):
        said, nv = self.run_check((False, "gh: network unreachable"))
        self.assertEqual(nv, [])
        self.assertEqual(len(said), 1)
        self.assertFalse(said[0][1])


if __name__ == "__main__":
    unittest.main()

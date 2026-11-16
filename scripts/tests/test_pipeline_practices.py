"""The pipeline follows the shared Rust CI practices (MNT-010; MNT-007 adds its classes here).

These read the workflow, the Makefile, make ci's plan and the nextest config themselves, not a
copy of them (ADR-0008): the property is asserted where it lives.
"""

from __future__ import annotations

import re
import sys
import tomllib
import unittest
from pathlib import Path

import yaml

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))
import ci_local  # noqa: E402

CI = ROOT / ".github" / "workflows" / "ci.yml"
MAKEFILE = ROOT / "Makefile"
NEXTEST = ROOT / "api" / ".config" / "nextest.toml"
JUNIT = "api/target/nextest/default/junit.xml"


def rust_steps() -> list[dict]:
    return yaml.safe_load(CI.read_text())["jobs"]["rust"]["steps"]


def step(steps: list[dict], name: str) -> dict:
    found = [s for s in steps if s.get("name") == name]
    if len(found) != 1:
        raise AssertionError(f"ci.yml rust: expected one step named {name!r}, found {len(found)}")
    return found[0]


def recipe(target: str) -> list[str]:
    """The Makefile recipe lines of `target`, without their leading tab."""
    lines = MAKEFILE.read_text().splitlines()
    head = re.compile(rf"^{re.escape(target)}:(\s|$)")
    for i, line in enumerate(lines):
        if head.match(line):
            body = []
            for rest in lines[i + 1:]:
                if not rest.startswith("\t"):
                    break
                body.append(rest[1:].strip())
            return body
    raise AssertionError(f"Makefile has no target {target!r}")


class Tests(unittest.TestCase):
    """MNT-010: one test script, a nextest config that times every test, and CI that keeps it."""

    def test_ci_make_test_and_make_ci_run_the_one_test_script(self) -> None:
        """R-1: ci.yml's Test step, `make test` and make ci all run scripts/test.sh."""
        test = step(rust_steps(), "Test")
        self.assertEqual(test.get("run", "").strip(), "../scripts/test.sh")
        self.assertEqual(test.get("shell"), "bash")
        self.assertEqual(recipe("test"), ["scripts/test.sh"])
        self.assertEqual(ci_local.PLAN["ci.yml"]["rust"]["Test"], ci_local.LOCAL)
        stamp = step(rust_steps(), "Build stamp follows HEAD in a worktree")
        self.assertEqual(stamp.get("run", "").strip(),
                         "../scripts/test.sh --ignored -p kanbanr-cli --test build_stamp")
        self.assertIn("scripts/test.sh --ignored -p kanbanr-cli", recipe("itest"))

    def test_nextest_config_writes_junit_names_slow_tests_and_never_fails_fast(self) -> None:
        """R-4: the default profile writes a JUnit report, names a test past 60 s, and never stops early."""
        profile = tomllib.loads(NEXTEST.read_text())["profile"]["default"]
        self.assertEqual(profile["junit"]["path"], "junit.xml")
        slow = profile["slow-timeout"]
        self.assertEqual(slow["period"], "60s")
        self.assertNotIn("terminate-after", slow)
        self.assertIs(profile["fail-fast"], False)

    def test_ci_installs_a_pinned_nextest_checks_and_uploads_each_systems_junit_report(self) -> None:
        """R-5: a pinned nextest is installed before Test, and each system's report is checked and kept."""
        steps = rust_steps()
        names = [s.get("name") for s in steps]
        test_at = names.index("Test")
        installs = [i for i, s in enumerate(steps)
                    if "taiki-e/install-action@" in str(s.get("uses", ""))
                    and (s.get("with") or {}).get("tool") == "cargo-nextest@0.9.151"]
        self.assertTrue(installs, "no install-action step installs cargo-nextest@0.9.151")
        self.assertLess(installs[0], test_at, "cargo-nextest is installed after the Test step")
        self.assertRegex(steps[installs[0]]["uses"], r"@[0-9a-f]{40}\b")

        check = step(steps, "The run left a JUnit report with a time for every test")
        self.assertGreater(names.index(check["name"]), test_at)
        cond = check.get("if", "")
        self.assertIn("!cancelled()", cond)
        self.assertIn("runner.os == 'Linux'", cond)
        self.assertIn("scripts/check_junit.py target/nextest/default/junit.xml", check.get("run", ""))

        uploads = [s for s in steps if "actions/upload-artifact@" in str(s.get("uses", ""))]
        self.assertEqual(len(uploads), 1, "the rust job should upload one artifact, the JUnit report")
        upload = uploads[0]
        self.assertGreater(steps.index(upload), test_at)
        self.assertRegex(upload["uses"], r"@[0-9a-f]{40}\b")
        self.assertEqual(upload.get("if"), "${{ !cancelled() }}")
        self.assertEqual(upload["with"]["path"], JUNIT)
        self.assertIn("matrix.os", upload["with"]["name"])


if __name__ == "__main__":
    unittest.main()

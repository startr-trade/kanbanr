"""Unit tests for scripts/check_junit.py (MNT-010 R-4). Run by the workflows job, or:

    python3 -m unittest discover -s scripts/tests -p 'test_check_junit.py' -v
"""

from __future__ import annotations

import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import check_junit  # noqa: E402

# Modelled on the report cargo-nextest 0.9.151 writes with [profile.default.junit].
REPORT = """<?xml version="1.0" encoding="UTF-8"?>
<testsuites name="nextest-run" tests="3" failures="1" errors="0" uuid="0b7c" timestamp="2026-11-16T10:00:00.000+05:30" time="1.512">
    <testsuite name="nxprobe" tests="2" disabled="0" errors="0" failures="1">
        <testcase name="tests::fast" classname="nxprobe" timestamp="2026-11-16T10:00:00.001+05:30" time="0.004">
        </testcase>
        <testcase name="tests::slowish" classname="nxprobe" timestamp="2026-11-16T10:00:00.001+05:30" time="1.502">
            <failure type="test failure">thread 'tests::slowish' panicked</failure>
        </testcase>
    </testsuite>
    <testsuite name="nxprobe::cli_flow" tests="1" disabled="0" errors="0" failures="0">
        <testcase name="integration_ok" classname="nxprobe::cli_flow" timestamp="2026-11-16T10:00:00.002+05:30" time="0.006">
        </testcase>
    </testsuite>
</testsuites>
"""

NO_TIME = """<?xml version="1.0" encoding="UTF-8"?>
<testsuites name="nextest-run">
    <testsuite name="nxprobe">
        <testcase name="tests::fast" classname="nxprobe" time="0.004"></testcase>
        <testcase name="tests::untimed" classname="nxprobe"></testcase>
    </testsuite>
</testsuites>
"""

NO_TEST = """<?xml version="1.0" encoding="UTF-8"?>
<testsuites name="nextest-run" tests="0"></testsuites>
"""


class JunitReport(unittest.TestCase):
    def write(self, text: str) -> Path:
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        path = Path(tmp.name) / "junit.xml"
        path.write_text(text, encoding="utf-8")
        return path

    def test_a_report_with_a_time_on_every_test_passes(self) -> None:
        ok, message = check_junit.check(self.write(REPORT))
        self.assertTrue(ok, message)
        self.assertEqual(message, "junit: 3 tests, 1.512s, slowest nxprobe tests::slowish 1.502s")
        self.assertEqual(check_junit.main([str(self.write(REPORT))]), 0)

    def test_a_report_missing_a_time_or_with_no_test_fails(self) -> None:
        ok, message = check_junit.check(self.write(NO_TIME))
        self.assertFalse(ok)
        self.assertIn("nxprobe tests::untimed", message)
        ok, message = check_junit.check(self.write(NO_TEST))
        self.assertFalse(ok)
        self.assertIn("no <testcase>", message)
        self.assertFalse(check_junit.check(self.write(REPORT.replace('time="0.006"', 'time="-1"')))[0])
        self.assertEqual(check_junit.main([str(self.write(NO_TEST))]), 1)


if __name__ == "__main__":
    unittest.main()

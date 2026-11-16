#!/usr/bin/env python3
"""Check that a cargo-nextest JUnit report has a time for every test (MNT-010).

    python3 scripts/check_junit.py api/target/nextest/default/junit.xml

Exit 0 and print `junit: <n> tests, <total>s, slowest <name> <t>s`, or exit 1
naming the first offender: no report, no <testcase>, or a <testcase> whose
`time` is missing or not a non-negative number. Standard library only.
"""

from __future__ import annotations

import sys
import xml.etree.ElementTree as ET
from pathlib import Path


def check(path: Path) -> tuple[bool, str]:
    """Return (ok, message) for the JUnit report at `path`."""
    try:
        root = ET.parse(path).getroot()
    except (OSError, ET.ParseError) as e:
        return False, f"junit: cannot read {path}: {e}"
    count, total, slowest = 0, 0.0, ("", -1.0)
    for case in root.iter("testcase"):
        name = " ".join(p for p in (case.get("classname"), case.get("name")) if p)
        raw = case.get("time")
        try:
            t = float(raw) if raw is not None else None
        except ValueError:
            t = None
        if t is None or not t >= 0.0:
            return False, f"junit: {name or '<unnamed testcase>'} has no time (time={raw!r})"
        count += 1
        total += t
        if t > slowest[1]:
            slowest = (name, t)
    if count == 0:
        return False, f"junit: {path} holds no <testcase>"
    return True, f"junit: {count} tests, {total:.3f}s, slowest {slowest[0]} {slowest[1]:.3f}s"


def main(argv: list[str]) -> int:
    if len(argv) != 1:
        print("usage: check_junit.py <report>", file=sys.stderr)
        return 2
    ok, message = check(Path(argv[0]))
    print(message, file=sys.stdout if ok else sys.stderr)
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))

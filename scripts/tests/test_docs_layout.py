"""The docs layout holds (MNT-002): the product guide is `docs/guide`, the system representation is
`docs/system`, and the move from the old source folder left nothing behind. Run by CI, `make ci`, or:

    python3 -m unittest -v scripts/tests/test_docs_layout.py
"""

from __future__ import annotations

import re
import subprocess
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
DOCS = ROOT / "docs"
GUIDE = DOCS / "guide"
SYSTEM = DOCS / "system"
SNAPSHOT = Path(__file__).resolve().parent / "fixtures" / "summary-before-mnt-002.txt"

# Built from two parts so this file does not name the old folder itself.
OLD = "docs/" + "src"

# Records, kept as written: the changelog (and the book's mirror of it) and past release notes.
RECORDS = re.compile(r"^(CHANGELOG\.md|docs/guide/project/changelog\.md|docs/guide/releases/[^/]+\.md)$")

LINK = re.compile(r"\]\(([^)#]+\.md)")


def read(path: Path) -> str:
    return path.read_text(encoding="utf-8")


def summary_paths() -> list[str]:
    return LINK.findall(read(GUIDE / "SUMMARY.md"))


class TestLayout(unittest.TestCase):
    def test_summary_paths_match_the_pre_rename_summary(self) -> None:
        """R-1: every chapter the book had before the move is still listed at the same path relative
        to the book's source, and still exists, so its published URL is unchanged."""
        before = [line.strip() for line in read(SNAPSHOT).splitlines() if line.strip()]
        self.assertTrue(before, "the snapshot of the pre-move SUMMARY.md is empty")
        now = set(summary_paths())
        missing = [p for p in before if p not in now]
        self.assertEqual(missing, [], "chapters no longer listed at their old path")
        gone = [p for p in before if not (GUIDE / p).is_file()]
        self.assertEqual(gone, [], "chapters listed but missing from docs/guide")

    def test_book_source_is_docs_guide(self) -> None:
        """R-1: book.toml builds from docs/guide, and the old source folder is gone."""
        book = read(DOCS / "book.toml")
        self.assertRegex(book, r'(?m)^src\s*=\s*"guide"\s*$')
        self.assertTrue((GUIDE / "SUMMARY.md").is_file())
        self.assertFalse((ROOT / OLD).exists(), f"{OLD} still exists")

    def test_no_live_reference_to_docs_src_remains(self) -> None:
        """R-2: only past changelog entries and past release notes still name the old folder."""
        out = subprocess.run(
            ["git", "grep", "-l", "-F", OLD],
            cwd=ROOT, capture_output=True, text=True, check=False,
        )
        self.assertIn(out.returncode, (0, 1), out.stderr)
        live = [p for p in out.stdout.splitlines() if p and not RECORDS.match(p)]
        self.assertEqual(live, [], f"files that still name {OLD}")

    def test_docs_system_exists_and_is_not_in_the_book(self) -> None:
        """R-5: docs/system exists with its README, outside the book's source, and no chapter
        lists it."""
        self.assertTrue((SYSTEM / "README.md").is_file())
        self.assertFalse(SYSTEM.resolve().is_relative_to(GUIDE.resolve()))
        for p in summary_paths():
            target = (GUIDE / p).resolve()
            self.assertFalse(target.is_relative_to(SYSTEM.resolve()), f"SUMMARY.md lists {p}")
        self.assertNotIn("system/", read(GUIDE / "SUMMARY.md"))


if __name__ == "__main__":
    unittest.main()

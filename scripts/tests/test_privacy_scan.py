"""Unit tests for scripts/privacy_scan.py (MNT-012). Run by `make ci`, or:

    python3 -m unittest scripts/tests/test_privacy_scan.py

Every repository is built under `tempfile`, never under this checkout (L-41).
"""

from __future__ import annotations

import contextlib
import importlib.util
import io
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).resolve().parents[1] / "privacy_scan.py"
_spec = importlib.util.spec_from_file_location("privacy_scan", SCRIPT)
ps = importlib.util.module_from_spec(_spec)
sys.modules["privacy_scan"] = ps
_spec.loader.exec_module(ps)


def git(repo: Path, *args: str) -> str:
    return subprocess.run(["git", *args], cwd=repo, check=True, capture_output=True, text=True).stdout


class TempRepo(unittest.TestCase):
    """A board-like repository and a folder beside it, both under one temporary directory."""

    def setUp(self) -> None:
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name).resolve()
        self.repo = self.root / "board"
        self.repo.mkdir()
        git(self.repo, "init", "-q", "-b", "main")
        git(self.repo, "config", "user.name", "Pat Example")
        git(self.repo, "config", "user.email", "pat@kanbanr.invalid")
        git(self.repo, "config", "commit.gpgsign", "false")
        self.pattern_file = self.root / "private-patterns.txt"
        self.pattern_file.write_text("# the person's own list\nname: Alex\n\n")

    def tearDown(self) -> None:
        self.tmp.cleanup()

    def write(self, rel: str, content: str | bytes) -> None:
        path = self.repo / rel
        path.parent.mkdir(parents=True, exist_ok=True)
        if isinstance(content, bytes):
            path.write_bytes(content)
        else:
            path.write_text(content)

    def commit(self, message: str) -> str:
        git(self.repo, "add", "-A")
        git(self.repo, "commit", "-q", "-m", message)
        return git(self.repo, "rev-parse", "--short", "HEAD").strip()

    def run_main(self, *argv: str) -> int:
        with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
            return ps.main(list(argv))


class HeadScan(TempRepo):
    def test_every_match_in_a_tracked_file_is_listed_with_its_place(self) -> None:
        self.write("items/FTR-001/spec.md", "# Spec\n\nlogs under /home/someone/work\n")
        self.write("docs/notes.md", "reviewed with Alex on Monday\n")
        self.write("assets/logo.bin", b"\xff\xfe\x00Alex /home/someone/\x00\xff")
        self.commit("init")
        self.write("untracked.md", "/home/someone/ and Alex\n")
        pats = ps.load_patterns(self.pattern_file)

        hits = ps.scan_head("kanbanr", self.repo, pats)

        found = sorted((h.board, h.where, h.place, h.category, h.pattern, h.at_head) for h in hits)
        self.assertEqual(
            found,
            [
                ("kanbanr", "head", "docs/notes.md:1", "name", "Alex", True),
                ("kanbanr", "head", "items/FTR-001/spec.md:3", "home-path", r"/home/[^/\s]+/", True),
            ],
        )
        self.assertFalse(any("untracked" in h.place or "logo" in h.place for h in hits))
        self.assertEqual(
            {h.text for h in hits}, {"logs under /home/someone/work", "reviewed with Alex on Monday"}
        )


class Patterns(TempRepo):
    def test_built_in_patterns_match_and_allow_lines_suppress(self) -> None:
        built_in = ps.load_patterns(None)

        def categories(pats, line: str) -> list[str]:
            return [c for c, _ in ps._matches(line, pats)]

        self.assertEqual(categories(built_in, "at /home/alex/src"), ["home-path"])
        self.assertEqual(categories(built_in, "at /Users/alex/src"), ["home-path"])
        self.assertEqual(categories(built_in, r"at C:\Users\alex\src"), ["home-path"])
        self.assertEqual(categories(built_in, "write to alex.b@corp.io"), ["email"])
        self.assertEqual(categories(built_in, "under /tmp/claude-1001/x"), ["scratch-path"])
        self.assertEqual(categories(built_in, 'The person said: "keep it"'), ["quotation"])
        self.assertEqual(categories(built_in, "the user wrote: “ship it”"), ["quotation"])
        self.assertEqual(categories(built_in, "mail someone@example.com"), [])
        self.assertEqual(categories(built_in, "mail a@b.test or noreply@github.com"), [])
        self.assertEqual(categories(built_in, "Alex was here"), [])

        own = self.root / "own.txt"
        own.write_text("name: Alex\nrepository: kanban\nallow: kanbanr\n")
        pats = ps.load_patterns(own)
        self.assertEqual(categories(pats, "ALEX was here"), ["name"])
        self.assertEqual(categories(pats, "the kanban board"), ["repository"])
        self.assertEqual(categories(pats, "the kanbanr board"), [])

        bad = self.root / "bad.txt"
        bad.write_text("# fine\nname: Alex\nname: (unclosed\n")
        with self.assertRaises(ValueError) as ctx:
            ps.load_patterns(bad)
        self.assertTrue(str(ctx.exception).startswith("line 3:"), str(ctx.exception))


class HistoryScan(TempRepo):
    def test_a_history_only_match_is_reported_and_the_repository_is_unchanged(self) -> None:
        self.write("docs/a.md", "first line\nmet Alex today\n")
        first = self.commit("add a note")
        self.write("docs/a.md", "first line\n")
        second = self.commit("drop the name Alex from the note")
        head_before = git(self.repo, "rev-parse", "HEAD")
        status_before = git(self.repo, "status", "--porcelain")
        pats = ps.load_patterns(self.pattern_file)

        head_hits = ps.scan_head("kanbanr", self.repo, pats)
        hits = ps.scan_history("kanbanr", self.repo, pats, head_hits)

        self.assertEqual(head_hits, [])
        added = [h for h in hits if h.place.endswith("docs/a.md:2")]
        self.assertEqual(len(added), 1, hits)
        self.assertEqual(added[0].place, f"{first} docs/a.md:2")
        self.assertEqual((added[0].where, added[0].category, added[0].at_head), ("history", "name", False))
        messages = [h for h in hits if h.place == f"{second} message"]
        self.assertEqual([(h.category, h.text) for h in messages], [("name", "drop the name Alex from the note")])
        identities = [h for h in hits if h.category == "identity"]
        self.assertEqual(len(identities), 1, identities)
        self.assertEqual(identities[0].place, f"{first} identity (2 commits)")
        self.assertEqual(identities[0].text, "Pat Example <pat@kanbanr.invalid>")
        self.assertEqual(git(self.repo, "rev-parse", "HEAD"), head_before)
        self.assertEqual(git(self.repo, "status", "--porcelain"), status_before)


class Report(TempRepo):
    def test_the_summary_names_no_matched_text(self) -> None:
        self.write("docs/a.md", "met Alex at /home/someone/x\n")
        self.commit("note")
        out = self.root / "out"

        code = self.run_main(
            "--board", f"kanbanr={self.repo}", "--patterns", str(self.pattern_file), "--out", str(out)
        )

        self.assertEqual(code, 1)
        head = git(self.repo, "rev-parse", "HEAD").strip()
        summary = (out / "summary.md").read_text()
        self.assertIn(head, summary)
        self.assertIn("| kanbanr | name | 1 | 1 | 0 |", summary)
        self.assertIn("| kanbanr | home-path | 1 | 1 | 0 |", summary)
        rows = (out / "hits.tsv").read_text().splitlines()
        self.assertEqual(rows[0], "board\twhere\tplace\tcategory\tpattern\tat_head\ttext")
        texts = {row.split("\t")[6] for row in rows[1:]}
        self.assertEqual(len(rows) - 1, 5)  # two head hits, their two history hits, one identity
        for text in texts:
            self.assertNotIn(text, summary)
        self.assertNotIn("Alex", summary)
        self.assertNotIn("/home/someone", summary)
        self.assertNotIn(str(self.pattern_file), summary)
        self.assertNotIn(self.pattern_file.name, summary)

    def test_an_output_folder_inside_a_board_is_refused(self) -> None:
        self.write("docs/a.md", "nothing private\n")
        self.commit("note")
        script_repo = ps._script_repo()
        inside_board = self.repo / "x"
        inside_script_repo = script_repo / "x"
        self.assertFalse(inside_script_repo.exists())

        for out in (inside_board, inside_script_repo):
            code = self.run_main("--board", f"kanbanr={self.repo}", "--out", str(out))
            self.assertEqual(code, 2, out)
            self.assertFalse(out.exists(), out)
        self.assertEqual(git(self.repo, "status", "--porcelain"), "")

        beside = self.root / "out"
        self.assertEqual(self.run_main("--board", f"kanbanr={self.repo}", "--out", str(beside)), 0)
        self.assertTrue((beside / "summary.md").is_file())


if __name__ == "__main__":
    unittest.main()

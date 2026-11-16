"""Tests for deploy/compose/serve.sh (FTR-030 R-10).

serve.sh runs under bash with a stub `kanbanr` first on PATH that prints its
arguments, one per line, so each test reads the command the site would start.
SERVE_CONFIG and BOARDS_ROOT point the script at a temporary boards.conf and
boards folder. Run from the repository root:

    python3 -m unittest -q deploy/tests/test_serve_sh.py
"""

import os
import subprocess
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).resolve().parents[1] / "compose" / "serve.sh"

STUB = """\
#!/bin/sh
for a in "$@"; do printf '%s\\n' "$a"; done
"""


class ServeShTest(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.tmp = Path(self._tmp.name)
        self.bin = self.tmp / "bin"
        self.bin.mkdir()
        stub = self.bin / "kanbanr"
        stub.write_text(STUB)
        stub.chmod(0o755)
        self.root = self.tmp / "boards"
        self.root.mkdir()
        self.conf = self.tmp / "boards.conf"  # absent unless a test writes it

    def tearDown(self):
        self._tmp.cleanup()

    def board(self, name=None):
        """A board folder (with projects/) under the boards root, or the root itself."""
        path = self.root / name if name else self.root
        (path / "projects").mkdir(parents=True)
        return path

    def serve(self, local_boards=""):
        env = dict(
            os.environ,
            PATH=f"{self.bin}{os.pathsep}{os.environ['PATH']}",
            SERVE_CONFIG=str(self.conf),
            BOARDS_ROOT=str(self.root),
            KANBANR_LOCAL_BOARDS=local_boards,
        )
        return subprocess.run(["bash", str(SCRIPT)], env=env, capture_output=True, text=True, timeout=30)

    def assertServes(self, result, *paths):
        self.assertEqual(result.returncode, 0, result.stderr)
        expected = ["serve"]
        for p in paths:
            expected += ["--board", str(p)]
        self.assertEqual(result.stdout.splitlines(), expected)

    def test_one_board_passes_one_board_flag(self):
        # Mode (b) with KANBANR_LOCAL_BOARDS empty: the mounted folder is the one board.
        self.board()
        self.assertServes(self.serve(), self.root)
        # Mode (b) naming one subfolder.
        self.board("solo")
        self.assertServes(self.serve(" solo "), self.root / "solo")
        # Mode (a), a conf of one board.
        self.conf.write_text("# one board\n\nsolo https://git.example.invalid/solo.git 60\n")
        self.assertServes(self.serve(), self.root / "solo")

    def test_several_boards_pass_one_board_flag_each(self):
        for name in ("zeta", "alpha", "mid"):
            self.board(name)
        # Mode (b): in the order named, not sorted.
        self.assertServes(self.serve("zeta,alpha,mid"), *(self.root / n for n in ("zeta", "alpha", "mid")))
        # Mode (a): in conf order, comments and blank lines skipped, the last line unterminated.
        self.conf.write_text(
            "# the site's boards\n"
            "mid https://git.example.invalid/mid.git 60\n"
            "\n"
            "   # an indented comment\n"
            "zeta https://git.example.invalid/zeta.git 120 main\n"
            "alpha https://git.example.invalid/alpha.git"
        )
        self.assertServes(self.serve(), *(self.root / n for n in ("mid", "zeta", "alpha")))

    def test_a_folder_without_projects_is_refused(self):
        self.board("good")
        (self.root / "bad").mkdir()
        result = self.serve("good,bad")
        self.assertEqual(result.returncode, 2)
        self.assertEqual(result.stdout, "", "kanbanr must not start")
        self.assertIn(f"{self.root / 'bad'} is not a board folder (it has no projects/)", result.stderr)
        # The root itself, with no projects/, is refused too.
        result = self.serve("")
        self.assertEqual(result.returncode, 2)
        self.assertIn(f"{self.root} is not a board folder", result.stderr)
        # An empty list is refused.
        self.conf.write_text("# no boards yet\n")
        result = self.serve()
        self.assertEqual(result.returncode, 2)
        self.assertIn("no boards to serve", result.stderr)


if __name__ == "__main__":
    unittest.main()

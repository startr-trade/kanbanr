"""Tests for deploy/modules/kanbanr-site/files/sync.sh (FTR-030 R-2 to R-4).

Each test works in its own temporary folder with bare remotes by path, the
host's `git` and `sh`, and short intervals (PULL_INTERVAL=1, MAX_BACKOFF=4).
Run from the repository root:

    python3 -m unittest -q deploy/tests/test_sync_sh.py

SYNC_SH_SHELL picks the shell that runs the script (default `sh`), e.g.
`busybox sh` to try the alpine/git image's shell.
"""

import os
import re
import shlex
import subprocess
import tempfile
import time
import unittest
from pathlib import Path

SCRIPT = Path(__file__).resolve().parents[1] / "modules" / "kanbanr-site" / "files" / "sync.sh"
SHELL = shlex.split(os.environ.get("SYNC_SH_SHELL", "sh"))


def git(*args, cwd=None):
    """Run git with a fixed identity and no user or system config."""
    env = dict(os.environ)
    env.update(
        GIT_CONFIG_NOSYSTEM="1",
        GIT_CONFIG_GLOBAL=os.devnull,
        GIT_AUTHOR_NAME="Test",
        GIT_AUTHOR_EMAIL="test@example.invalid",
        GIT_COMMITTER_NAME="Test",
        GIT_COMMITTER_EMAIL="test@example.invalid",
    )
    out = subprocess.run(
        ["git", *args], cwd=cwd, env=env, check=True, capture_output=True, text=True
    )
    return out.stdout.strip()


class SyncShTest(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.tmp = Path(self._tmp.name)
        self.home = self.tmp / "home"
        self.boards = self.tmp / "boards"
        self.boards.mkdir()
        self._logs = []

    def tearDown(self):
        for f in self._logs:
            f.close()
        self._tmp.cleanup()

    # -- fixtures -----------------------------------------------------------

    def make_remote(self, name):
        """A bare remote `<name>.git` with one commit on main, and its work tree."""
        remote = self.tmp / f"{name}.git"
        work = self.tmp / f"{name}-work"
        git("init", "--quiet", "--bare", "-b", "main", str(remote))
        git("init", "--quiet", "-b", "main", str(work))
        (work / "projects").mkdir()
        (work / "projects" / "first.md").write_text("first\n")
        git("add", ".", cwd=work)
        git("commit", "--quiet", "-m", "first", cwd=work)
        git("remote", "add", "origin", str(remote), cwd=work)
        git("push", "--quiet", "origin", "main", cwd=work)
        return remote, work

    def push_file(self, work, rel, text="x\n"):
        path = work / rel
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text)
        git("add", ".", cwd=work)
        git("commit", "--quiet", "-m", f"add {rel}", cwd=work)
        git("push", "--quiet", "origin", "main", cwd=work)
        return git("rev-parse", "HEAD", cwd=work)

    def env(self, **extra):
        env = {
            "PATH": os.environ["PATH"],
            "HOME": str(self.home),
            "TMPDIR": str(self.tmp),
            "GIT_TERMINAL_PROMPT": "0",
            "GIT_CONFIG_NOSYSTEM": "1",
            "PULL_INTERVAL": "1",
            "MAX_BACKOFF": "4",
            "FETCH_TIMEOUT": "10",
            "CLONE_TIMEOUT": "30",
            "CREDENTIAL_KIND": "none",
            "BOARDS_ROOT": str(self.boards),
        }
        env.update({k: str(v) for k, v in extra.items()})
        return env

    def board_env(self, name, remote, **extra):
        return self.env(
            BOARD_NAME=name,
            BOARD_URL=remote,
            BOARD_DIR=self.boards / name,
            HEARTBEAT=self.tmp / f"heartbeat-{name}",
            **extra,
        )

    def sync(self, *args, env, timeout=60):
        return subprocess.run(
            [*SHELL, str(SCRIPT), *args],
            env=env,
            capture_output=True,
            text=True,
            timeout=timeout,
        )

    def popen(self, *args, env, log):
        self._logs.append(log.open("w"))
        return subprocess.Popen(
            [*SHELL, str(SCRIPT), *args],
            env=env,
            stdout=subprocess.DEVNULL,
            stderr=self._logs[-1],
            text=True,
        )

    def clone(self, name, remote):
        out = self.sync("clone", env=self.board_env(name, remote))
        self.assertEqual(out.returncode, 0, out.stderr)
        return self.boards / name

    def wait_until(self, cond, seconds, what):
        deadline = time.monotonic() + seconds
        while time.monotonic() < deadline:
            if cond():
                return
            time.sleep(0.2)
        self.fail(f"timed out after {seconds}s waiting for {what}")

    # -- R-2 clone first ----------------------------------------------------

    def test_clone_makes_a_full_checkout_owned_by_the_runner(self):
        remote, work = self.make_remote("board-a")
        self.push_file(work, "projects/second.md")
        out = self.sync("clone", env=self.board_env("board-a", remote))
        self.assertEqual(out.returncode, 0, out.stderr)
        repo = self.boards / "board-a"
        self.assertEqual(git("rev-parse", "--is-shallow-repository", cwd=repo), "false")
        self.assertEqual(git("rev-list", "--count", "HEAD", cwd=repo), "2")
        for path in (repo, repo / ".git", repo / "projects" / "first.md"):
            self.assertEqual(path.stat().st_uid, os.getuid(), path)
        self.assertFalse((self.boards / "board-a.partial").exists())
        sha = git("rev-parse", "--short", "HEAD", cwd=repo)
        self.assertRegex(
            out.stderr,
            r"(?m)^\d{4}-\d\d-\d\dT\d\d:\d\d:\d\dZ sync: \[board-a\] cloned main at " + sha + "$",
        )

    def test_a_failed_clone_leaves_no_partial_checkout(self):
        missing = self.tmp / "no-such-remote.git"
        out = self.sync("clone", env=self.board_env("board-a", missing))
        self.assertEqual(out.returncode, 1, out.stderr)
        self.assertFalse((self.boards / "board-a").exists())
        self.assertFalse((self.boards / "board-a.partial").exists())
        self.assertRegex(
            out.stderr, r"sync: \[board-a\] clone failed \(exit \d+\); retrying is the caller's"
        )

    def test_clone_twice_keeps_the_first_checkout(self):
        remote, work = self.make_remote("board-a")
        repo = self.clone("board-a", remote)
        first = git("rev-parse", "HEAD", cwd=repo)
        (repo / "kept.txt").write_text("kept\n")
        self.push_file(work, "projects/second.md")
        out = self.sync("clone", env=self.board_env("board-a", remote))
        self.assertEqual(out.returncode, 0, out.stderr)
        self.assertEqual(git("rev-parse", "HEAD", cwd=repo), first)
        self.assertTrue((repo / "kept.txt").exists())
        short = git("rev-parse", "--short", "HEAD", cwd=repo)
        self.assertIn(f"checkout present at {short}; not cloning", out.stderr)

    def test_clone_all_clones_each_board_and_stops_at_a_failure(self):
        remote_a, _ = self.make_remote("board-a")
        remote_b, _ = self.make_remote("board-b")
        conf = self.tmp / "boards.conf"
        conf.write_text(
            "# name url interval branch\n"
            f"board-a {remote_a} 60\n"
            "\n"
            f"board-b {remote_b} 30 main\n"
        )
        out = self.sync("clone-all", str(conf), env=self.env())
        self.assertEqual(out.returncode, 0, out.stderr)
        for name in ("board-a", "board-b"):
            self.assertTrue((self.boards / name / "projects" / "first.md").exists(), name)
        self.assertIn("[board-a] cloned main at", out.stderr)
        self.assertIn("[board-b] cloned main at", out.stderr)

        remote_c, _ = self.make_remote("board-c")
        conf.write_text(
            f"board-c {remote_c}\n"
            f"board-bad {self.tmp / 'no-such-remote.git'}\n"
            f"board-d {remote_c}\n"
        )
        out = self.sync("clone-all", str(conf), env=self.env())
        self.assertEqual(out.returncode, 1, out.stderr)
        self.assertTrue((self.boards / "board-c" / ".git").exists())
        self.assertFalse((self.boards / "board-bad").exists())
        self.assertFalse((self.boards / "board-d").exists())
        self.assertIn("[board-bad] clone failed", out.stderr)

        conf.write_text(f"board-a {remote_a}\nboard-a {remote_b}\n")
        out = self.sync("clone-all", str(conf), env=self.env())
        self.assertNotEqual(out.returncode, 0)
        self.assertIn("board board-a is listed twice", out.stderr)

    # -- R-3 follow in place ------------------------------------------------

    def test_follow_fast_forwards_in_place(self):
        remote, work = self.make_remote("board-a")
        repo = self.clone("board-a", remote)
        old = git("rev-parse", "--short", "HEAD", cwd=repo)
        inode = (repo / "projects").stat().st_ino
        self.push_file(work, "projects/second.md")
        new = git("rev-parse", "--short", "HEAD", cwd=work)
        out = self.sync("follow", env=self.board_env("board-a", remote, SYNC_ROUNDS=1))
        self.assertEqual(out.returncode, 0, out.stderr)
        self.assertTrue((repo / "projects" / "second.md").exists())
        self.assertEqual((repo / "projects").stat().st_ino, inode)
        self.assertEqual(git("rev-parse", "--short", "HEAD", cwd=repo), new)
        self.assertIn(f"sync: [board-a] updated {old}..{new}", out.stderr)
        self.assertTrue((self.tmp / "heartbeat-board-a").exists())

    def test_follow_all_runs_one_loop_per_board(self):
        remote_a, work_a = self.make_remote("board-a")
        remote_b, work_b = self.make_remote("board-b")
        conf = self.tmp / "boards.conf"
        conf.write_text(f"board-a {remote_a} 1\nboard-b {remote_b} 1\n")
        out = self.sync("clone-all", str(conf), env=self.env())
        self.assertEqual(out.returncode, 0, out.stderr)

        log = self.tmp / "follow-all.log"
        proc = self.popen("follow-all", str(conf), env=self.env(), log=log)
        try:
            self.push_file(work_a, "projects/only-a.md")
            self.wait_until(
                lambda: (self.boards / "board-a" / "projects" / "only-a.md").exists(),
                20,
                "board-a to update",
            )
            self.assertFalse((self.boards / "board-b" / "projects" / "only-a.md").exists())
            self.push_file(work_b, "projects/only-b.md")
            self.wait_until(
                lambda: (self.boards / "board-b" / "projects" / "only-b.md").exists(),
                20,
                "board-b to update",
            )
            self.assertFalse((self.boards / "board-a" / "projects" / "only-b.md").exists())
            for name in ("board-a", "board-b"):
                self.assertTrue((self.tmp / f"sync-heartbeat-{name}").exists(), name)
            self.assertIsNone(proc.poll())
        finally:
            proc.terminate()
            proc.wait(timeout=10)
        text = log.read_text()
        self.assertIn("sync: [board-a] updated", text)
        self.assertIn("sync: [board-b] updated", text)

        # A loop that exits stops the others, and follow-all exits 1.
        conf.write_text(f"board-a {remote_a} 1\nboard-gone {remote_b} 1\n")
        out = self.sync("follow-all", str(conf), env=self.env(), timeout=20)
        self.assertEqual(out.returncode, 1, out.stderr)
        self.assertIn("[board-gone] no checkout at", out.stderr)

    # -- R-4 failures keep the checkout ---------------------------------------

    def test_follow_keeps_the_checkout_when_the_remote_is_not_a_fast_forward(self):
        remote, work = self.make_remote("board-a")
        self.push_file(work, "projects/second.md")
        repo = self.clone("board-a", remote)
        head = git("rev-parse", "HEAD", cwd=repo)
        files = sorted(p.name for p in (repo / "projects").iterdir())
        git("reset", "--quiet", "--hard", "HEAD~1", cwd=work)
        (work / "projects" / "rewritten.md").write_text("rewritten\n")
        git("add", ".", cwd=work)
        git("commit", "--quiet", "-m", "rewrite", cwd=work)
        git("push", "--quiet", "--force", "origin", "main", cwd=work)

        out = self.sync("follow", env=self.board_env("board-a", remote, SYNC_ROUNDS=1))
        self.assertEqual(out.returncode, 0, out.stderr)
        self.assertEqual(git("rev-parse", "HEAD", cwd=repo), head)
        self.assertEqual(sorted(p.name for p in (repo / "projects").iterdir()), files)
        self.assertEqual(git("status", "--porcelain", cwd=repo), "")
        short = git("rev-parse", "--short", "HEAD", cwd=repo)
        self.assertIn(
            f"the remote's main is not a fast-forward of the checkout at {short}; "
            "keeping it (restart to clone afresh); next try in 2s",
            out.stderr,
        )

    def test_follow_backs_off_on_failed_fetches_and_recovers(self):
        remote, work = self.make_remote("board-a")
        repo = self.clone("board-a", remote)
        short = git("rev-parse", "--short", "HEAD", cwd=repo)
        away = self.tmp / "away.git"
        remote.rename(away)
        log = self.tmp / "follow.log"
        proc = self.popen(
            "follow", env=self.board_env("board-a", remote, SYNC_ROUNDS=4), log=log
        )
        try:
            # Rounds sleep 1, 2, 4 and 4 s; the remote is back before the fourth.
            self.wait_until(
                lambda: log.read_text().count("next try in 4s") >= 2, 20, "three failed fetches"
            )
            git("remote", "set-url", "origin", str(away), cwd=work)
            new = self.push_file(work, "projects/second.md")
            away.rename(remote)
            self.assertEqual(proc.wait(timeout=20), 0)
        finally:
            if proc.poll() is None:
                proc.kill()
        text = log.read_text()
        tries = re.findall(r"fetch failed \(exit \d+\); keeping (\w+); next try in (\d+)s", text)
        self.assertEqual(tries, [(short, "2"), (short, "4"), (short, "4")], text)
        self.assertIn(f"updated {short}..{new[:len(short)]}", text)
        self.assertLess(text.index("next try in 2s"), text.index("updated"))
        self.assertEqual(git("rev-parse", "HEAD", cwd=repo), new)

    def test_follow_reports_a_stalled_loop_to_its_probe(self):
        heartbeat = self.tmp / "heartbeat-board-a"
        env = self.board_env("board-a", self.tmp / "unused.git", FETCH_TIMEOUT=5)
        # The bound is max(1, 4) + 5 + 60 = 69 s.
        heartbeat.touch()
        out = self.sync("alive", env=env)
        self.assertEqual(out.returncode, 0, out.stdout + out.stderr)

        stale = time.time() - 300
        os.utime(heartbeat, (stale, stale))
        out = self.sync("alive", env=env)
        self.assertEqual(out.returncode, 1)
        self.assertRegex(out.stdout, r"^no sync round for (29\d|30\d)s$")

        heartbeat.unlink()
        out = self.sync("alive", env=env)
        self.assertEqual(out.returncode, 1)


if __name__ == "__main__":
    unittest.main()

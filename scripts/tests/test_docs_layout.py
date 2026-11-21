"""The docs layout holds (MNT-002): the product guide is `docs/guide`, the system representation is
`docs/system`, and the move from the old source folder left nothing behind. Run by CI, `make ci`, or:

    python3 -m unittest -v scripts/tests/test_docs_layout.py
"""

from __future__ import annotations

import fnmatch
import re
import subprocess
import tempfile
import unittest
from pathlib import Path
from typing import NamedTuple

ROOT = Path(__file__).resolve().parents[2]
DOCS = ROOT / "docs"
GUIDE = DOCS / "guide"
SYSTEM = DOCS / "system"
SNAPSHOT = Path(__file__).resolve().parent / "fixtures" / "summary-before-mnt-002.txt"
ALLOW_FILE = Path(__file__).resolve().parent / "fixtures" / "agent-neutral-allow.txt"

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


# --- MNT-019: product text names no agent vendor outside the adapter -------------------------------

VENDOR = re.compile(r"\bClaude\b(?! Code)")
FENCE = re.compile(r"^\s*(```|~~~)")
CODE_SPAN = re.compile(r"`[^`]*`")
RS_TOKENS = ("Taker::Claude", "ClaudeCmd", '"Claude"')
MANIFESTS = (
    "api/Cargo.toml",
    "api/crates/kanbanr-cli/Cargo.toml",
    "docs/book.toml",
    "assets/brand/kanbanr-social-preview.svg",
    ".claude-plugin/plugin.json",
    ".claude-plugin/marketplace.json",
    ".github/ISSUE_TEMPLATE/bug_report.yml",
)
RULE = (
    "Product text says 'agent', not 'Claude'; Claude Code is named only as the adapter. Fix the "
    "line, or (for the adapter's own files) add an entry with its reason to "
    "scripts/tests/fixtures/agent-neutral-allow.txt."
)


class Allow(NamedTuple):
    """One allowlist entry: a path glob, an optional substring that limits it to one line, why."""

    glob: str
    substring: str | None
    reason: str


def load_allow(text: str) -> list[Allow]:
    entries = []
    for raw in text.splitlines():
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        spec, _, reason = line.partition(" # ")
        glob, sep, sub = spec.partition(" :: ")
        entries.append(Allow(glob.strip(), sub.strip() if sep else None, reason.strip()))
    return entries


def _product_files(root: Path) -> list[Path]:
    files: list[Path] = []
    readme = root / "README.md"
    if readme.is_file():
        files.append(readme)
    guide = root / "docs" / "guide"
    files += sorted(
        p for p in guide.rglob("*.md")
        if not RECORDS.match(p.relative_to(root).as_posix())
    )
    files.append(guide / "reference" / "openapi.json")
    files.append(root / "skill" / "kanbanr" / "SKILL.md")
    web = root / "web" / "src"
    files += sorted(list(web.rglob("*.ts")) + list(web.rglob("*.tsx")))
    files += sorted((root / "api" / "crates" / "kanbanr-core" / "src").rglob("*.rs"))
    files.append(root / "api" / "crates" / "kanbanr-cli" / "src" / "main.rs")
    files += [root / m for m in MANIFESTS]
    return [p for p in files if p.is_file()]


def _hits(root: Path, path: Path) -> list[tuple[int, str]]:
    """Every (line number, line) in `path` that names the vendor as product text."""
    rel = path.relative_to(root).as_posix()
    lines = read(path).splitlines()
    suffix = path.suffix
    if rel == "skill/kanbanr/SKILL.md":  # the front matter only
        end = next((i for i, l in enumerate(lines[1:], 1) if l.strip() == "---"), len(lines))
        lines = lines[:end]
    found = []
    in_fence = False
    for n, line in enumerate(lines, 1):
        text = line
        if suffix == ".md":
            if FENCE.match(line):
                in_fence = not in_fence
                continue
            if in_fence:
                continue
            text = CODE_SPAN.sub("", line)
        elif suffix == ".json":
            text = CODE_SPAN.sub("", line)
        elif suffix in (".ts", ".tsx", ".rs"):
            if suffix == ".rs" and "#[cfg(test)]" in line:
                break  # unit tests are not product text
            stripped = line.strip()
            is_clap_help = rel.endswith("kanbanr-cli/src/main.rs") and stripped.startswith("///")
            if stripped.startswith(("//", "/*", "*", "{/*")) and not is_clap_help:
                continue
            if suffix == ".rs":
                for token in RS_TOKENS:
                    text = text.replace(token, "")
            else:
                text = text.replace('"Claude"', "")
        elif suffix in (".toml", ".yml") and line.strip().startswith("#"):
            continue  # comments are not product text
        if VENDOR.search(text):
            found.append((n, line.strip()))
    return found


def scan(root: Path, allow: list[Allow]) -> list[str]:
    """Every `path:line: text` in product text that names Claude outside the adapter, then every
    allowlist entry that exempted nothing (so the list cannot rot)."""
    used = [False] * len(allow)
    out = []
    for path in _product_files(root):
        rel = path.relative_to(root).as_posix()
        for n, text in _hits(root, path):
            exempt = False
            for i, a in enumerate(allow):
                if fnmatch.fnmatch(rel, a.glob) and (a.substring is None or a.substring in text):
                    used[i] = exempt = True
            if not exempt:
                out.append(f"{rel}:{n}: {text}")
    for i, a in enumerate(allow):
        if not used[i]:
            sub = f" :: {a.substring}" if a.substring else ""
            out.append(f"stale allow entry (matches nothing): {a.glob}{sub}")
    return out


class TestAgentNeutral(unittest.TestCase):
    def test_product_text_names_no_agent_vendor_outside_the_adapter(self) -> None:
        """R-1, R-3, R-4: no bare "Claude" is left in the monitor, the README, the guide, the API
        description, the skill's description or the CLI's help and messages."""
        hits = scan(ROOT, load_allow(read(ALLOW_FILE)))
        self.assertEqual(hits, [], RULE + "\n" + "\n".join(hits))

    def test_the_scan_flags_a_planted_mention_and_spares_the_adapter(self) -> None:
        """R-4: the scan's rules, its allowlist, and that a stale entry fails."""
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)

            def put(rel: str, text: str) -> None:
                f = root / rel
                f.parent.mkdir(parents=True, exist_ok=True)
                f.write_text(text, encoding="utf-8")

            put("README.md", "# Title\n\nask Claude to plan\n\nthe Claude Code skill\n\n"
                "```\nby: Claude\n```\n\nstored as `by: Claude` here\n")
            put("web/src/a.ts", '// Claude says hi\nconst by = "Claude";\nconst s = "Ask Claude";\n')
            core = "api/crates/kanbanr-core/src/"
            put(core + "claude.rs", 'let s = "Claude wrote this";\n')
            put(core + "other.rs", 'let a = Taker::Claude;\n/// Claude doc\n'
                "#[cfg(test)]\nmod t { const X: &str = \"Claude\"; fn f() { \"Claude says\"; } }\n")
            put("CHANGELOG.md", "Claude, as a record\n")
            allow = load_allow(
                "api/crates/kanbanr-core/src/claude.rs  # the adapter's own file\n"
                "docs/guide/gone.md :: no such line  # a stale entry\n"
            )
            hits = scan(root, allow)
            self.assertEqual(
                [h.split(": ", 1)[0] for h in hits[:-1]], ["README.md:3", "web/src/a.ts:3"], hits
            )
            self.assertEqual(len(hits), 3, hits)
            self.assertTrue(hits[-1].startswith("stale allow entry"), hits)
            # With the stale entry gone and the adapter file listed, only the two real hits remain.
            clean = scan(root, allow[:1])
            self.assertEqual(len(clean), 2, clean)


if __name__ == "__main__":
    unittest.main()

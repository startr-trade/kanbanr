#!/usr/bin/env python3
"""List what a board would show the public that it should not (MNT-012).

    python3 scripts/privacy_scan.py --board kanbanr=<board> [--board katalogr=<board>] \
        [--patterns <file kept outside every repository>] --out <folder outside every repository>

Scans each board's tracked files at HEAD (read from `git archive`) and its history (every added
line and commit message of `git log --all`, and each distinct commit identity) for the built-in
rules (home paths, e-mail addresses, scratch paths, quotations attributed to the person) and the
person's own `<category>: <regex>` lines. `allow: <regex>` lines leave out a match that an allow
match on the same line covers. Matching is case-insensitive.

Writes `<out>/hits.tsv`, the raw record with the matched text, and `<out>/summary.md`, counts only:
no matched text, no pattern text, no path of the pattern file. Only reads the boards.

Exit 2 when `--out` is inside a board or inside this repository, a board is not a git repository,
or a pattern does not compile (nothing is written); 1 when a `head` hit outside the `quotation`
and `identity` categories remains; 0 otherwise. Standard library and `git` only.
"""

from __future__ import annotations

import argparse
import io
import re
import subprocess
import sys
import tarfile
import time
from dataclasses import dataclass
from pathlib import Path

EXCERPT = 160

# Categories listed for the person to read; they never fail the scan by themselves.
READ_ONLY = ("quotation", "identity")

_EMAIL = re.compile(r"[\w.+-]+@[\w-]+(\.[\w-]+)+", re.IGNORECASE)
_EXAMPLE_DOMAINS = ("example.com", "example.org", "example.net")
_RESERVED_TLDS = ("invalid", "example", "test")

BUILT_INS: list[tuple[str, re.Pattern]] = [
    ("home-path", re.compile(r"/home/[^/\s]+/", re.IGNORECASE)),
    ("home-path", re.compile(r"/Users/[^/\s]+/", re.IGNORECASE)),
    ("home-path", re.compile(r"[A-Za-z]:\\Users\\[^\\\s]+", re.IGNORECASE)),
    ("email", _EMAIL),
    ("scratch-path", re.compile(r"/tmp/claude-\d+/", re.IGNORECASE)),
    ("quotation", re.compile(r"\b(the person|the user)\b[^.\n]{0,60}:\s*[\"“]", re.IGNORECASE)),
]

IDENTITY_PATTERN = "commit identity"


@dataclass(frozen=True)
class Hit:
    board: str
    where: str  # "head" or "history"
    place: str  # "path:line", or "<short commit> path:line" / "message" / "identity"
    category: str
    pattern: str
    text: str
    at_head: bool


@dataclass
class Patterns:
    rules: list[tuple[str, re.Pattern]]
    allow: list[re.Pattern]


def load_patterns(path: Path | None) -> Patterns:
    """The built-in rules plus the lines of the person's file, if one is given.

    Each non-blank line not starting with `#` is `<category>: <regex>` or `allow: <regex>`.
    A line that is not of that form, or whose regex does not compile, raises
    `ValueError("line N: …")`."""
    pats = Patterns(rules=list(BUILT_INS), allow=[])
    if path is None:
        return pats
    for n, raw in enumerate(Path(path).read_text(encoding="utf-8").splitlines(), start=1):
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        category, sep, regex = line.partition(":")
        category, regex = category.strip(), regex.strip()
        if not sep or not category or not regex:
            raise ValueError(f"line {n}: expected '<category>: <regex>' or 'allow: <regex>'")
        try:
            compiled = re.compile(regex, re.IGNORECASE)
        except re.error as e:
            raise ValueError(f"line {n}: the regex does not compile: {e}") from None
        if category.lower() == "allow":
            pats.allow.append(compiled)
        else:
            pats.rules.append((category, compiled))
    return pats


def _email_is_safe(address: str) -> bool:
    """A documentation or no-reply address (RFC 2606/6761), which may stand."""
    low = address.lower()
    if "noreply" in low or "no-reply" in low:
        return True
    domain = low.rsplit("@", 1)[-1]
    if any(domain == d or domain.endswith("." + d) for d in _EXAMPLE_DOMAINS):
        return True
    return domain.rsplit(".", 1)[-1] in _RESERVED_TLDS


def _matches(line: str, pats: Patterns) -> list[tuple[str, re.Pattern]]:
    """(category, rule) for every match in `line` that no allow match covers."""
    allowed = [m.span() for a in pats.allow for m in a.finditer(line)]
    found: list[tuple[str, re.Pattern]] = []
    for category, rule in pats.rules:
        for m in rule.finditer(line):
            if rule is _EMAIL and _email_is_safe(m.group(0)):
                continue
            start, end = m.span()
            if any(s <= start and end <= e for s, e in allowed):
                continue
            found.append((category, rule))
    return found


def _excerpt(line: str) -> str:
    return line.strip()[:EXCERPT]


def _git(repo: Path, *args: str) -> bytes:
    return subprocess.run(
        ["git", *args], cwd=repo, check=True, capture_output=True, stdin=subprocess.DEVNULL
    ).stdout


def scan_head(name: str, repo: Path, pats: Patterns) -> list[Hit]:
    """Every match in a tracked file at HEAD. A member that is not UTF-8 (binary) is skipped."""
    data = _git(repo, "archive", "--format=tar", "HEAD")
    hits: list[Hit] = []
    with tarfile.open(fileobj=io.BytesIO(data), mode="r:") as tar:
        for member in tar.getmembers():
            if not member.isfile():
                continue
            fh = tar.extractfile(member)
            if fh is None:
                continue
            try:
                content = fh.read().decode("utf-8")
            except UnicodeDecodeError:
                continue
            for n, line in enumerate(content.split("\n"), start=1):
                line = line.rstrip("\r")
                for category, rule in _matches(line, pats):
                    hits.append(
                        Hit(name, "head", f"{member.name}:{n}", category, rule.pattern, _excerpt(line), True)
                    )
    return hits


def _split_place(place: str) -> tuple[str, str]:
    """(path, line) of a `path:line` place."""
    path, _, line = place.rpartition(":")
    return path, line


def _diff_path(header: str) -> str | None:
    """The new path from a `+++ b/<path>` line, or None for a deletion."""
    target = header[4:].strip()
    if target.startswith('"') and target.endswith('"'):
        target = target[1:-1]
    if target == "/dev/null":
        return None
    return target[2:] if target.startswith("b/") else target


_HUNK = re.compile(r"^@@ -\d+(?:,\d+)? \+(\d+)(?:,\d+)? @@")


def scan_history(name: str, repo: Path, pats: Patterns, head_hits: list[Hit]) -> list[Hit]:
    """Every match in an added line or a commit message of any commit, and each distinct commit
    identity once. `at_head` is true for an added-line hit whose text is still in the same path
    at HEAD (taken from `head_hits`)."""
    out = _git(
        repo,
        "log",
        "--all",
        "--format=%x00%h%x00%an <%ae>%x00%cn <%ce>%x00%B%x00",
        "-p",
        "--unified=0",
        "--no-color",
        "--no-ext-diff",
    )
    at_head_keys = {
        (_split_place(h.place)[0], h.category, h.pattern, h.text) for h in head_hits if h.board == name
    }
    hits: list[Hit] = []
    # identity -> [first commit seen so far (oldest, as the log runs newest first), count]
    identities: dict[str, list] = {}
    parts = out.split(b"\0")
    for i in range(1, len(parts) - 4, 5):
        short, author, committer, body, patch = (p.decode("utf-8", errors="replace") for p in parts[i : i + 5])
        for ident in dict.fromkeys((author, committer)):
            entry = identities.setdefault(ident, [short, 0])
            entry[0] = short
            entry[1] += 1
        for line in body.split("\n"):
            for category, rule in _matches(line, pats):
                hits.append(Hit(name, "history", f"{short} message", category, rule.pattern, _excerpt(line), False))
        path: str | None = None
        lineno = 0
        for line in patch.split("\n"):
            if line.startswith("diff --git "):
                path, lineno = None, 0
            elif line.startswith("+++ "):
                path = _diff_path(line)
            elif line.startswith("@@"):
                m = _HUNK.match(line)
                lineno = int(m.group(1)) if m else 0
            elif line.startswith("+") and path is not None:
                text = line[1:].rstrip("\r")
                for category, rule in _matches(text, pats):
                    excerpt = _excerpt(text)
                    hits.append(
                        Hit(
                            name,
                            "history",
                            f"{short} {path}:{lineno}",
                            category,
                            rule.pattern,
                            excerpt,
                            (path, category, rule.pattern, excerpt) in at_head_keys,
                        )
                    )
                lineno += 1
    for ident, (first, count) in identities.items():
        noun = "commit" if count == 1 else "commits"
        hits.append(
            Hit(name, "history", f"{first} identity ({count} {noun})", "identity", IDENTITY_PATTERN, _excerpt(ident), False)
        )
    return hits


def _sort_key(hit: Hit) -> tuple:
    head, sep, tail = hit.place.rpartition(":")
    if sep and tail.isdigit():
        return (hit.board, hit.where, head, int(tail))
    return (hit.board, hit.where, hit.place, -1)


def _column(hit: Hit) -> str:
    if hit.where == "head":
        return "head"
    return "history (also at head)" if hit.at_head else "history only"


COLUMNS = ("head", "history (also at head)", "history only")


def _table(heads: dict[str, str], hits: list[Hit]) -> str:
    counts: dict[tuple[str, str], dict[str, int]] = {}
    for h in hits:
        if h.category == "identity":
            continue
        row = counts.setdefault((h.board, h.category), dict.fromkeys(COLUMNS, 0))
        row[_column(h)] += 1
    lines = ["| board | category | " + " | ".join(COLUMNS) + " |", "|---|---|" + "---:|" * len(COLUMNS)]
    for board in heads:
        cats = sorted(c for b, c in counts if b == board)
        if not cats:
            lines.append(f"| {board} | (none) | " + " | ".join("0" for _ in COLUMNS) + " |")
        for c in cats:
            row = counts[(board, c)]
            lines.append(f"| {board} | {c} | " + " | ".join(str(row[k]) for k in COLUMNS) + " |")
    return "\n".join(lines)


def write_report(out: Path, heads: dict[str, str], hits: list[Hit]) -> None:
    """`out/hits.tsv` (the raw record, with the matched text) and `out/summary.md` (counts only)."""
    out.mkdir(parents=True, exist_ok=True)
    rows = ["\t".join(("board", "where", "place", "category", "pattern", "at_head", "text"))]
    for h in sorted(hits, key=_sort_key):
        fields = (h.board, h.where, h.place, h.category, h.pattern, "true" if h.at_head else "false", h.text)
        rows.append("\t".join(f.replace("\t", " ").replace("\n", " ") for f in fields))
    (out / "hits.tsv").write_text("\n".join(rows) + "\n", encoding="utf-8")

    summary = ["# Privacy scan summary", "", "## Boards", ""]
    for board, head in heads.items():
        summary.append(f"- {board}: HEAD `{head}`")
    summary += ["", "## Hits", "", _table(heads, hits), "", "## Commit identities", ""]
    for board in heads:
        n = sum(1 for h in hits if h.board == board and h.category == "identity")
        summary.append(f"- {board}: {n}")
    summary.append("")
    (out / "summary.md").write_text("\n".join(summary), encoding="utf-8")


def _script_repo() -> Path:
    here = Path(__file__).resolve().parent
    try:
        top = _git(here, "rev-parse", "--show-toplevel").decode().strip()
        return Path(top).resolve()
    except (subprocess.CalledProcessError, OSError):
        return here.parent


def _inside(path: Path, root: Path) -> bool:
    return path == root or path.is_relative_to(root)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--board", action="append", required=True, metavar="NAME=PATH")
    parser.add_argument("--patterns", type=Path)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args(argv)

    boards: dict[str, Path] = {}
    for spec in args.board:
        name, sep, raw = spec.partition("=")
        if not sep or not name or not raw:
            print(f"privacy scan: --board expects NAME=PATH, got {spec!r}", file=sys.stderr)
            return 2
        boards[name] = Path(raw).resolve()

    out = args.out.resolve()
    heads: dict[str, str] = {}
    for name, repo in boards.items():
        try:
            heads[name] = _git(repo, "rev-parse", "--verify", "HEAD").decode().strip()
        except (subprocess.CalledProcessError, OSError):
            print(f"privacy scan: board {name} is not a git repository with a commit", file=sys.stderr)
            return 2
        if _inside(out, repo):
            print(f"privacy scan: --out is inside board {name}; choose a folder outside it", file=sys.stderr)
            return 2
    if _inside(out, _script_repo()):
        print("privacy scan: --out is inside the repository holding this script", file=sys.stderr)
        return 2
    try:
        pats = load_patterns(args.patterns)
    except ValueError as e:
        print(f"privacy scan: the pattern file: {e}", file=sys.stderr)
        return 2
    except OSError:
        print("privacy scan: the pattern file cannot be read", file=sys.stderr)
        return 2

    started = time.monotonic()
    hits: list[Hit] = []
    for name, repo in boards.items():
        head_hits = scan_head(name, repo, pats)
        hits += head_hits
        hits += scan_history(name, repo, pats, head_hits)
    write_report(out, heads, hits)
    elapsed = time.monotonic() - started
    with (out / "summary.md").open("a", encoding="utf-8") as fh:
        fh.write(f"\nScan time: {elapsed:.1f}s\n")
    print(_table(heads, hits))
    fixable = [h for h in hits if h.where == "head" and h.category not in READ_ONLY]
    return 1 if fixable else 0


if __name__ == "__main__":
    sys.exit(main())

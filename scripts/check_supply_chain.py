#!/usr/bin/env python3
"""Supply-chain checks for kanbanr (MNT-004).

A build that makes a thing people install or trust reads only the lockfile, and fails when the
lockfile is wrong. This script enforces that in CI and in `make ci`:

    python3 scripts/check_supply_chain.py [--root DIR]    scan the repository
    python3 scripts/check_supply_chain.py --sbom FILE...  check CycloneDX SBOM files

It prints one problem per line on stderr, as `path:line: message`, and exits 1 if there are any.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]

CARGO_VERBS = r"(?:build|install|test|run|clippy|check|tree|doc|metadata|publish|package|bench)"
CARGO_RE = re.compile(rf"\bcargo\s+(?:{CARGO_VERBS}\b|chef\s+cook\b)")
CARGO_PY_RE = re.compile(rf'"cargo"\s*,\s*"(?:{CARGO_VERBS}|chef)"')
LOCKED_RE = re.compile(r"--locked\b|--frozen\b")
NPM_INSTALL_RE = re.compile(r"\bnpm\s+install\b")
NPM_GLOBAL_RE = re.compile(r"(?:^|\s)(?:-g|--global)(?:\s|$)")
NPM_FALLBACK_RE = re.compile(r"\bnpm\s+ci\s*\|\|")
UNLOCKED_OK_RE = re.compile(r"#\s*unlocked-ok\b:?(.*)$")
ATTEST_ACTION = "actions/attest-build-provenance"
SHA40_RE = re.compile(r"^[0-9a-f]{40}$")


def logical_lines(text: str) -> list[tuple[int, str]]:
    """(first line number, text) for each line, a trailing backslash joining the next."""
    out: list[tuple[int, str]] = []
    pending: str | None = None
    start = 0
    for number, raw in enumerate(text.splitlines(), 1):
        if pending is None:
            start = number
            pending = ""
        if raw.rstrip().endswith("\\"):
            pending += raw.rstrip()[:-1] + " "
            continue
        out.append((start, pending + raw))
        pending = None
    if pending is not None:
        out.append((start, pending))
    return out


def strip_comment(line: str) -> str:
    return re.sub(r"(?:^|\s)#.*$", "", line)


def is_echo(line: str) -> bool:
    return bool(re.match(r"\s*(?:-?@?\s*)?(?:echo|printf)\b", line))


def check_cargo(path: Path, text: str) -> list[str]:
    """Report cargo commands that read dependencies without --locked or --frozen."""
    problems: list[str] = []
    python = path.suffix == ".py"
    for number, line in logical_lines(text):
        ok = UNLOCKED_OK_RE.search(line)
        if ok and not ok.group(1).strip():
            problems.append(f"{path}:{number}: 'unlocked-ok' needs a reason after the colon")
            continue
        code = line if python else strip_comment(line)
        if not python and is_echo(code):
            continue
        hit = CARGO_PY_RE.search(code) if python else CARGO_RE.search(code)
        if not hit or ok or LOCKED_RE.search(code):
            continue
        command = " ".join(code[hit.start():].split())
        problems.append(f"{path}:{number}: cargo without --locked: {command}")
    return problems


def check_npm(path: Path, text: str) -> list[str]:
    """Report `npm install` (not global) and `npm ci || ...` fallbacks."""
    problems: list[str] = []
    for number, line in logical_lines(text):
        code = strip_comment(line)
        if is_echo(code):
            continue
        if NPM_INSTALL_RE.search(code) and not NPM_GLOBAL_RE.search(code):
            problems.append(f"{path}:{number}: npm install, use npm ci: {' '.join(code.split())}")
        if NPM_FALLBACK_RE.search(code):
            problems.append(f"{path}:{number}: npm ci must not fall back: {' '.join(code.split())}")
    return problems


def check_dockerfile(text: str) -> list[str]:
    """Report a COPY that globs the lockfile, which would make a missing lockfile no error."""
    return [
        f"Dockerfile:{number}: the lockfile is copied with a glob: {' '.join(line.split())}"
        for number, line in logical_lines(text)
        if re.match(r"\s*COPY\b", line) and re.search(r"package-lock\.json\*", line)
    ]


def job_blocks(workflow: str) -> dict[str, str]:
    """The text of each job under `jobs:`, by name (a job is a two-space-indented key)."""
    jobs: dict[str, list[str]] = {}
    name = None
    in_jobs = False
    for line in workflow.splitlines():
        if re.match(r"^jobs:\s*$", line):
            in_jobs = True
            continue
        if not in_jobs:
            continue
        if re.match(r"^\S", line):
            in_jobs = False
            continue
        m = re.match(r"^  ([A-Za-z0-9_-]+):\s*$", line)
        if m:
            name = m.group(1)
            jobs[name] = []
        elif name:
            jobs[name].append(line)
    return {k: "\n".join(v) for k, v in jobs.items()}


def check_release(workflow: str) -> list[str]:
    """The release workflow builds and attaches the SBOMs, and attests assets and image."""
    problems: list[str] = []
    jobs = job_blocks(workflow)
    if "scripts/make-sbom.sh" not in workflow:
        problems.append("release.yml: no step runs scripts/make-sbom.sh")
    if "ATTEST_RELEASE" not in workflow:
        problems.append("release.yml: ATTEST_RELEASE is not read, so attestation cannot be switched off")
    for job in ("release", "image"):
        block = jobs.get(job)
        if block is None:
            problems.append(f"release.yml: no '{job}' job")
            continue
        uses = re.findall(rf"uses:\s*{re.escape(ATTEST_ACTION)}@(\S+)", block)
        if not uses:
            problems.append(f"release.yml: the '{job}' job has no {ATTEST_ACTION} step")
        for ref in uses:
            if not SHA40_RE.match(ref):
                problems.append(f"release.yml: {ATTEST_ACTION} in '{job}' is not pinned to a commit: {ref}")
        if uses and "vars.ATTEST_RELEASE != 'false'" not in block:
            problems.append(f"release.yml: the '{job}' attestation is not switched by vars.ATTEST_RELEASE")
    image = jobs.get("image", "")
    if not re.search(r"^\s*sbom:\s*true\s*$", image, re.M):
        problems.append("release.yml: the image push does not set 'sbom: true'")
    if not re.search(r"^\s*provenance:\s*mode=max\s*$", image, re.M):
        problems.append("release.yml: the image push does not set 'provenance: mode=max'")
    return problems


def check_sbom(path: Path) -> list[str]:
    """A CycloneDX JSON file with a spec version and at least one component."""
    try:
        data = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, ValueError) as err:
        return [f"{path}: not readable JSON: {err}"]
    if not isinstance(data, dict) or data.get("bomFormat") != "CycloneDX":
        return [f"{path}: bomFormat is not CycloneDX"]
    problems = []
    if not data.get("specVersion"):
        problems.append(f"{path}: no specVersion")
    components = data.get("components")
    if not isinstance(components, list) or not components:
        problems.append(f"{path}: no components")
    return problems


def fenced_blocks(text: str) -> str:
    """Only the lines inside ``` fences, other lines blanked so line numbers stay true."""
    out, inside = [], False
    for line in text.splitlines():
        if line.lstrip().startswith("```"):
            inside = not inside
            out.append("")
        else:
            out.append(line if inside else "")
    return "\n".join(out)


def scan_files(root: Path) -> list[Path]:
    """Files whose build commands are checked. History (changelog, release notes), this
    script and its fixtures are not."""
    found: list[Path] = []
    found += sorted((root / ".github/workflows").glob("*.yml"))
    for name in ("Makefile", "docker/Dockerfile"):
        found.append(root / name)
    for pattern in ("*.sh", "*.ps1", "*.py"):
        found += sorted((root / "scripts").glob(pattern))
    found += sorted((root / "tools").glob("**/*.sh"))
    skip = {Path(__file__).resolve()}
    return [p for p in found if p.is_file() and p.resolve() not in skip]


def check(root: Path) -> list[str]:
    """All problems in the tree at root, each `path:line: message`."""
    problems: list[str] = []
    for path in scan_files(root):
        rel = path.relative_to(root)
        text = path.read_text(encoding="utf-8")
        problems += check_cargo(rel, text)
        if path.suffix != ".py":
            problems += check_npm(rel, text)
        if path.name == "Dockerfile":
            problems += [f"{rel}:{p.split(':', 1)[1]}" for p in check_dockerfile(text)]
    for doc in ("docs/src/getting-started/installation.md", "docs/src/project/contributing.md"):
        path = root / doc
        if path.is_file():
            text = fenced_blocks(path.read_text(encoding="utf-8"))
            problems += check_cargo(Path(doc), text) + check_npm(Path(doc), text)
    release = root / ".github/workflows/release.yml"
    if release.is_file():
        problems += check_release(release.read_text(encoding="utf-8"))
    return problems


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--root", type=Path, default=ROOT)
    parser.add_argument("--sbom", nargs="+", type=Path, metavar="FILE")
    args = parser.parse_args(argv)
    problems = [p for f in args.sbom for p in check_sbom(f)] if args.sbom else check(args.root)
    for problem in problems:
        print(problem, file=sys.stderr)
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))

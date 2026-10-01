#!/usr/bin/env python3
"""Run locally every CI check that can run off GitHub (FEAT-131). Entry point: `make ci`.

WHY THIS EXISTS: the first hour on GitHub turned up six CI failures, and every one had passed on
the maintainer's machine — a test against a binary built without its monitor, a symlinked temp
folder on macOS, two Windows failures, a mirrored changelog chapter only docs.yml compared, and a
release job naming a retired runner. Once the repository is public, each red run is public too.

HOW IT STAYS HONEST:
  * It works on a clean copy of the TRACKED tree (what `actions/checkout` gives a runner), at a
    fixed cache path with its own build cache, never on the working directory.
  * A step that is a script runs FROM THE WORKFLOW FILE: its `run:` block, working directory,
    shell and env are read from the YAML, so editing a workflow changes what runs here too.
  * EVERY `run:` step of every workflow must be classified below, as LOCAL or as needing GitHub
    (with the reason). A step nobody classified fails the run: a new CI check cannot quietly go
    unverified here.
  * It ends by listing what it could not verify, so "all passed" never overstates.

Needs: bash, git, python3 with PyYAML, cargo, node/npm, docker (actionlint, PowerShell), gh
(authenticated, for resolving action references), mdbook + mdbook-mermaid, shellcheck.
"""

from __future__ import annotations

import os
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

import yaml

ROOT = Path(subprocess.check_output(["git", "rev-parse", "--show-toplevel"], text=True).strip())
TREE = Path(os.environ.get("XDG_CACHE_HOME", Path.home() / ".cache")) / "kanbanr-ci" / "tree"
ACTIONLINT = "rhysd/actionlint:1.7.12"
PWSH = "mcr.microsoft.com/powershell:lts-7.4-ubuntu-22.04"

LOCAL = "local"

# workflow -> job -> step name -> LOCAL, or the reason it can only run on GitHub.
PLAN: dict[str, dict[str, dict[str, str]]] = {
    "ci.yml": {
        "rust": {
            "Install build deps (Linux)": "installs system packages on the runner; the local toolchain builds the same code",
            "Install build deps (macOS)": "macOS runner",
            "Verify build deps (Windows)": "Windows runner",
            "Build the monitor (web/dist)": LOCAL,
            "Check formatting (ubuntu only)": LOCAL,
            "Clippy (ubuntu only)": LOCAL,
            "Test": LOCAL,
        },
        "definition": {
            "Extract the definition from the PR body": "reads a pull request's body",
            "Build the CLI": LOCAL,
            "Check it against the bar": "checks the definition a pull request carries",
        },
        "web": {
            "Install dependencies": LOCAL,
            "Build": LOCAL,
            "Docs diagrams parse": LOCAL,
            "The monitor presents itself correctly": LOCAL,
        },
        "workflows": {
            "Every third-party action is pinned to a commit": LOCAL,
            "Every scanner suppression has a reason and an expiry": LOCAL,
            "actionlint": LOCAL,
        },
        "supply-chain": {
            "cargo deny (licences, bans, sources, advisories)": LOCAL,
            "cargo audit (RustSec advisories, yanked crates)": LOCAL,
            "npm audit (the monitor's production dependencies)": LOCAL,
        },
        "installers": {
            "The CLI links a TLS backend": LOCAL,
            "Install shellcheck": "installs a system package on the runner",
            "shellcheck install.sh": LOCAL,
            "install.ps1 parses": LOCAL,
            "The installers pin https on request and redirect": LOCAL,
            "The installer's targets are the targets we publish": LOCAL,
        },
    },
    "codeql.yml": {
        "analyze": {
            "Code scanning is available": "asks GitHub whether the repository can take code-scanning results",
        },
    },
    "trivy.yml": {
        "fs": {
            "Code scanning is available": "asks GitHub whether the repository can take code-scanning results",
        },
    },
    "docs.yml": {
        "build": {
            "Install mdBook + mermaid preprocessor": "downloads the pinned mdBook; checked here by version instead",
            "Every page is in the table of contents": LOCAL,
            "The mirrored chapters are in sync": LOCAL,
            "Build": LOCAL,
        },
    },
    "release.yml": {
        "tag": {
            "the tagged commit is on main": "judges a pushed tag against origin/main",
            "The tag is a semver that names the version being built": LOCAL,
        },
        "binaries": {
            "Install build deps (Linux)": "installs system packages on the runner",
            "Install build deps (macOS)": "macOS runner",
            "Build the web monitor": LOCAL,
            "Build CLI": LOCAL,
            "The binary names the commit it was built from": LOCAL,
            "Check the monitor is embedded, and compressed": LOCAL,
            "Package (Unix)": LOCAL,
            "Package (Windows)": "Windows runner",
            "Publish the binary's own checksum": LOCAL,
        },
        "release": {
            "Stage the installers beside the binaries": "needs every leg's artifacts",
            "Compute SHA256SUMS": "needs every leg's artifacts",
        },
        "verify-install": {
            "Minimal prerequisites (as a stranger would have)": "installs from a published release",
            "Run the published installer": "installs from a published release",
            "It installed, and it names both the version and the commit": "installs from a published release",
            "The monitor is served with no --ui-dir and no build step": "installs from a published release",
        },
        "image": {
            "Lowercase image name": "names the GHCR image; the image build itself is checked below",
        },
        "crates": {
            "Publish ears-classifier": "publishes to crates.io",
        },
    },
}

results: list[tuple[str, bool, str]] = []
not_verified: list[str] = []


def say(name: str, ok: bool, detail: str = "") -> None:
    results.append((name, ok, detail))
    print(f"{name:<72} {'ok' if ok else 'FAIL'}", flush=True)
    if not ok and detail:
        print("\n".join("    " + line for line in detail.rstrip().splitlines()[-30:]), flush=True)


def sh(cmd: list[str] | str, cwd: Path, env: dict[str, str] | None = None) -> tuple[bool, str]:
    proc = subprocess.run(
        cmd,
        cwd=cwd,
        env=env,
        shell=isinstance(cmd, str),
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
    )
    return proc.returncode == 0, proc.stdout


def fresh_tree() -> None:
    """The tracked tree, copied to TREE; only its build caches (api/target, web/node_modules) kept."""
    TREE.mkdir(parents=True, exist_ok=True)
    keep = {("api", "target"), ("web", "node_modules")}

    def remove(path: Path) -> None:
        if path.is_dir() and not path.is_symlink():
            shutil.rmtree(path)
        else:
            path.unlink()

    for child in TREE.iterdir():
        if child.name in {"api", "web"} and child.is_dir():
            for grandchild in child.iterdir():
                if (child.name, grandchild.name) not in keep:
                    remove(grandchild)
        else:
            remove(child)
    files = subprocess.check_output(["git", "ls-files", "-z"], cwd=ROOT).split(b"\0")
    for rel in filter(None, files):
        src, dst = ROOT / rel.decode(), TREE / rel.decode()
        if src.is_file():
            dst.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(src, dst)
    # A repository of its own, so steps that ask git (diffs, rev-parse) behave as on a runner.
    shutil.rmtree(TREE / ".git", ignore_errors=True)
    for cmd in (["git", "init", "-q"], ["git", "add", "-A"],
                ["git", "-c", "user.name=ci", "-c", "user.email=ci@local", "commit", "-qm", "snapshot"]):
        subprocess.run(cmd, cwd=TREE, check=True, stdout=subprocess.DEVNULL)


def workspace_version() -> str:
    meta = subprocess.check_output(
        ["cargo", "metadata", "--no-deps", "--format-version", "1"], cwd=TREE / "api", text=True
    )
    import json

    return next(p["version"] for p in json.loads(meta)["packages"] if p["name"] == "kanbanr-cli")


def expand(text: str, context: dict[str, str], where: str) -> str:
    """Replace `${{ … }}` with local values; an expression we cannot answer is an error."""

    def one(match: re.Match[str]) -> str:
        key = match.group(1).strip()
        if key in context:
            return context[key]
        raise KeyError(f"{where}: no local value for ${{{{ {key} }}}}")

    return re.sub(r"\$\{\{\s*([^}]+?)\s*\}\}", one, text)


def linux_matrix(job: dict) -> dict[str, str]:
    include = (job.get("strategy") or {}).get("matrix", {}).get("include") or []
    for leg in include:
        if str(leg.get("os", "")).startswith("ubuntu") and "x86_64" in str(leg.get("target", "x86_64")):
            return {f"matrix.{k}": str(v) for k, v in leg.items()}
    return {"matrix.os": "ubuntu-latest"}


def other_legs(job: dict) -> list[str]:
    matrix = (job.get("strategy") or {}).get("matrix") or {}
    legs = [str(v) for v in matrix.get("os", [])]
    legs += [f"{leg.get('os')} ({leg.get('target')})" for leg in matrix.get("include", [])]
    return [leg for leg in legs if not leg.startswith("ubuntu-latest") and "x86_64-unknown-linux" not in leg]


def run_workflow_steps() -> None:
    version = workspace_version()
    head = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip()
    for wf_name, jobs_plan in PLAN.items():
        wf = yaml.safe_load((TREE / ".github/workflows" / wf_name).read_text())
        wf_env = {k: str(v) for k, v in (wf.get("env") or {}).items()}
        for job_id, job in wf["jobs"].items():
            plan = jobs_plan.get(job_id, {})
            for leg in other_legs(job):
                not_verified.append(f"{wf_name} {job_id}: the {leg} leg")
            # Each job starts from a fresh checkout on GitHub, so it does here too: one job's
            # outputs (a regenerated chapter, a built web/dist) must not leak into the next —
            # a release build then reported itself "-dirty" because a docs step had synced a file.
            # The build caches survive, as the runner's caches do.
            subprocess.run(["git", "checkout", "-q", "--", "."], cwd=TREE, check=True)
            subprocess.run(["git", "clean", "-fdqx", "-e", "api/target", "-e", "web/node_modules"],
                           cwd=TREE, check=True)
            job_env = {**wf_env, **{k: str(v) for k, v in (job.get("env") or {}).items()}}
            job_wd = ((job.get("defaults") or {}).get("run") or {}).get("working-directory")
            for step in job.get("steps", []):
                if "run" not in step:
                    continue
                name = step.get("name")
                label = f"{wf_name} {job_id}: {name}"
                if name is None:
                    say(f"{wf_name} {job_id}: an unnamed run step", False,
                        "Name it: make ci classifies steps by name.")
                    continue
                decision = plan.get(name)
                if decision is None:
                    say(label, False, "This step is not classified in scripts/ci_local.py PLAN: "
                        "mark it local, or give the reason it can only run on GitHub.")
                    continue
                if decision != LOCAL:
                    not_verified.append(f"{label} — {decision}")
                    continue
                context = {
                    **linux_matrix(job),
                    "github.sha": head,
                    "github.ref_name": f"v{version}",
                    "github.repository": "startr-trade/kanbanr",
                    "runner.os": "Linux",
                    **{f"env.{k}": v for k, v in job_env.items()},
                }
                try:
                    script = expand(step["run"], context, label)
                    step_env = {k: expand(str(v), context, label) for k, v in (step.get("env") or {}).items()}
                except KeyError as err:
                    say(label, False, str(err))
                    continue
                wd = TREE / (step.get("working-directory") or job_wd or ".")
                with tempfile.TemporaryDirectory() as scratch:
                    env = {
                        **os.environ, **job_env, **step_env,
                        "CI": "true", "GITHUB_SHA": head, "GITHUB_REF_NAME": f"v{version}",
                        "GITHUB_REPOSITORY": "startr-trade/kanbanr",
                        "GITHUB_OUTPUT": f"{scratch}/output", "GITHUB_ENV": f"{scratch}/env",
                        "GITHUB_PATH": f"{scratch}/path", "GITHUB_STEP_SUMMARY": f"{scratch}/summary",
                    }
                    if step.get("shell") == "pwsh":
                        rel = wd.relative_to(TREE)
                        ok, out = sh(["docker", "run", "--rm", "-v", f"{TREE}:/repo", "-w", f"/repo/{rel}",
                                      PWSH, "pwsh", "-NoProfile", "-Command", script], TREE)
                    else:
                        ok, out = sh(["bash", "--noprofile", "--norc", "-eo", "pipefail", "-c", script], wd, env)
                say(label, ok, out)
            for name in plan:
                if not any(s.get("name") == name for s in job.get("steps", [])):
                    say(f"{wf_name} {job_id}: {name}", False,
                        "PLAN names a step the workflow no longer has — update scripts/ci_local.py.")


def lint_workflows() -> None:
    for wf in sorted((TREE / ".github/workflows").glob("*.yml")):
        try:
            yaml.safe_load(wf.read_text())
            say(f"workflows: {wf.name} parses", True)
        except yaml.YAMLError as err:
            say(f"workflows: {wf.name} parses", False, str(err))
    ok, out = sh(["docker", "run", "--rm", "-v", f"{TREE}:/repo", "-w", "/repo", "--entrypoint", "sh",
                  ACTIONLINT, "-c", "actionlint -no-color .github/workflows/*.yml"], TREE)
    say("workflows: actionlint (expressions, runner labels, shellcheck of run blocks)", ok, out)
    refs = sorted(set(re.findall(r"uses:\s+([A-Za-z0-9._/-]+@[A-Za-z0-9._-]+)",
                                 "\n".join(p.read_text() for p in (TREE / ".github/workflows").glob("*.yml")))))
    missing = []
    for ref in refs:
        repo = "/".join(ref.split("@")[0].split("/")[:2])
        tag = ref.split("@")[1]
        exact = [f"/repos/{repo}/git/ref/tags/{tag}", f"/repos/{repo}/git/ref/heads/{tag}"]
        if re.fullmatch(r"[0-9a-f]{40}", tag):
            exact = [f"/repos/{repo}/commits/{tag}"]
        # Three tries: a single network hiccup reported one of eighteen references as missing.
        found = any(sh(["gh", "api", path, "--silent"], TREE)[0] for _ in range(3) for path in exact)
        if not found:
            missing.append(ref)
    say(f"workflows: all {len(refs)} action references resolve upstream", not missing,
        "unresolvable: " + ", ".join(missing))


def uses_equivalents() -> None:
    """What workflows do through actions rather than scripts, done the same way here."""
    want = yaml.safe_load((TREE / ".github/workflows/docs.yml").read_text())["env"]
    ok, out = sh(["mdbook", "--version"], TREE)
    say(f"docs: mdbook is the pinned {want['MDBOOK_VERSION']}", ok and want["MDBOOK_VERSION"] in out, out)
    ok, out = sh(["mdbook-mermaid", "--version"], TREE)
    say(f"docs: mdbook-mermaid is the pinned {want['MDBOOK_MERMAID_VERSION']}",
        ok and want["MDBOOK_MERMAID_VERSION"] in out, out)
    ok, out = sh(["docker", "build", "-q", "-f", "docker/Dockerfile", "-t", "kanbanr:ci-local", "."], TREE)
    say("release image: docker/Dockerfile builds (docker/build-push-action, without the push)", ok, out)
    ok, out = sh([str(TREE / "scripts/security-scan.sh"), "image", "kanbanr:ci-local"], TREE)
    say("release image: no fixable HIGH/CRITICAL finding (release.yml's scan before the push)", ok, out)
    ok, out = sh([str(TREE / "scripts/security-scan.sh"), "deps", str(TREE)], TREE)
    say("trivy.yml: no finding in dependencies, secrets or the Dockerfile (stricter than CI)", ok, out)
    not_verified.append("codeql.yml: CodeQL (run `make codeql`; several minutes, so not part of make ci)")
    not_verified.append("codeql.yml / trivy.yml: uploads to code scanning")
    not_verified.append("release.yml image: pushing to GHCR")
    not_verified.append("release.yml release: creating the GitHub release")
    not_verified.append("docs.yml deploy: publishing to GitHub Pages")


def main() -> int:
    print(f"make ci: a clean copy of the tracked tree at {TREE}\n", flush=True)
    fresh_tree()
    lint_workflows()
    run_workflow_steps()
    uses_equivalents()
    failed = [name for name, ok, _ in results if not ok]
    print("\nNot verified locally (runs only on GitHub):")
    for item in not_verified:
        print(f"  - {item}")
    print()
    if failed:
        print(f"{len(failed)} of {len(results)} checks FAILED:")
        for name in failed:
            print(f"  - {name}")
        return 1
    print(f"all {len(results)} checks passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())

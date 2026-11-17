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
(authenticated, for resolving action references), mdbook, shellcheck.
"""

from __future__ import annotations

import argparse
import os
import re
import shutil
import subprocess
import sys
import tempfile
from datetime import datetime, timezone
from pathlib import Path

import yaml

ROOT = Path(subprocess.check_output(["git", "rev-parse", "--show-toplevel"], text=True).strip())
TREE = Path(os.environ.get("XDG_CACHE_HOME", Path.home() / ".cache")) / "kanbanr-ci" / "tree"
ACTIONLINT = "rhysd/actionlint:1.7.12"
PWSH = "mcr.microsoft.com/powershell:lts-7.4-ubuntu-22.04"

LOCAL = "local"

# Each run of a step in KEEP leaves its full output and JUnit report here, one folder a run, never
# pruned: the raw data a test's durations are read from (MNT-010; FTR-058 captures them).
RUNS = TREE.parent / "runs"
KEEP = {("ci.yml", "rust", "Test")}

# What each repository variable (`vars.NAME`) reads as when make ci runs a step locally. An unset
# variable reads as empty on GitHub too. ATTEST_RELEASE is on unless it is `false`, so the notes
# step runs as it does by default; the publish switches stay off, as nothing is published here.
LOCAL_VARS: dict[str, str] = {
    "vars.ATTEST_RELEASE": "",
    "vars.PUBLISH_CRATES": "",
    "vars.PUBLISH_OPENVSX": "",
}

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
            "The published OpenAPI matches the code": LOCAL,
            "Test": LOCAL,
            "The run left a JUnit report with a time for every test": LOCAL,
            "Build stamp follows HEAD in a worktree": LOCAL,
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
            "The diagram lint agrees with Pele": LOCAL,
            "The review page checks diagrams before Approve": LOCAL,
            "The monitor presents itself correctly": LOCAL,
            "The monitor's markdown cannot run script": LOCAL,
            "Every request goes to the configured API base": LOCAL,
            "The standalone monitor is read-only": LOCAL,
            "Board cards say what is next in plain words": LOCAL,
            "Each release's button says what it does": LOCAL,
            "A candidate whose preparation changed offers Promote to candidate again": LOCAL,
            "A release button names the release and asks first": LOCAL,
            "A release's notes show and are edited from the Releases page": LOCAL,
            "An item says which release it shipped in": LOCAL,
            "The board can show the items in no sprint": LOCAL,
            "Milestones are listed newest first": LOCAL,
            "The header shows the running kanbanr and warns when the board is ahead": LOCAL,
            "A prepared record shows filled in beside its button": LOCAL,
            "A disabled Approve or Ratify button says what is missing and who provides it": LOCAL,
            "The docs-folded sign-off asks for no form": LOCAL,
            "The Review page lists manual checks waiting for you": LOCAL,
            "The Review page offers one click for a batch": LOCAL,
            "The Review page shows a result's check id": LOCAL,
            "A prepared answer shows its outcome once": LOCAL,
            "The Workflow page shows the cadence, the git model and the guard rails": LOCAL,
            "The monitor shows agent scopes and who made each change": LOCAL,
            "Live updates settle before the monitor refetches": LOCAL,
            "The monitor's data cache shares reads and re-reads only changed slices": LOCAL,
            "The UI kit is built on the chosen design tokens": LOCAL,
            "The UI kit's renderer draws a component": LOCAL,
            "The monitor's navigation lists every page once, grouped into areas": LOCAL,
            "The monitor labels background drift with its revision and age": LOCAL,
            "The UI kit's tabs pick, render and underline correctly": LOCAL,
            "The monitor's colours meet WCAG 2.1 AA contrast": LOCAL,
            "The monitor draws no text under 13 px": LOCAL,
            "The monitor always shows the keyboard focus ring": LOCAL,
            "The Review page says a grant cannot be checked and why": LOCAL,
            "The UI kit's accordion rows open on demand under a heading": LOCAL,
            "The UI kit's table sorts, filters and exports correctly": LOCAL,
            "The UI kit's matrix, timeline and stepper draw their data correctly": LOCAL,
            "The monitor's types match the published API": LOCAL,
        },
        "vscode": {
            "Install the extension's dependencies": LOCAL,
            "The extension's version is the workspace's": LOCAL,
            "The extension compiles and packages": LOCAL,
        },
        "workflows": {
            "No tracked merge leftovers (*.orig, *.rej)": LOCAL,
            "The version being built has release notes": LOCAL,
            "The guides cover the pipeline and a product people use": LOCAL,
            "The privacy scan's own tests pass": LOCAL,
            "The pipeline follows the shared Rust CI practices": LOCAL,
            "The docs layout holds": LOCAL,
            "Every gate check is described in Working with Claude": LOCAL,
            "Every third-party action is pinned to a commit": LOCAL,
            "Every scanner suppression has a reason and an expiry": LOCAL,
            "actionlint": LOCAL,
        },
        "installers": {
            "Install shellcheck": "installs a system package on the runner",
            "shellcheck the installer and its verifier": LOCAL,
            "The PowerShell installer and its verifier parse": LOCAL,
            "The installers pin https on request and redirect": LOCAL,
            "The installer's targets are the targets we publish": LOCAL,
            "The installer's skill step does what it says": LOCAL,
            "The installer's editor step does what it says": LOCAL,
        },
    },
    # The dependency tier (MNT-007): what ci.yml's supply-chain job and TLS step ran, and the MSRV build.
    "deps.yml": {
        "supply-chain": {
            "cargo deny (licences, bans, sources, advisories)": LOCAL,
            "cargo audit (RustSec advisories, yanked crates)": LOCAL,
            "npm audit (the monitor's production dependencies)": LOCAL,
            "The supply-chain check's own tests pass": LOCAL,
            "Builds are locked and installs are clean": LOCAL,
            "The SBOMs build and are valid CycloneDX": LOCAL,
        },
        "tls-backend": {
            "The CLI links a TLS backend": LOCAL,
        },
        "msrv": {
            "Install build deps (Linux)": "installs system packages on the runner; the local toolchain builds the same code",
            "The workspace builds on its declared rust-version": LOCAL,
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
            "Install mdBook": "downloads the pinned mdBook; checked here by version instead",
            "Every page is in the table of contents": LOCAL,
            "The mirrored chapters are in sync": LOCAL,
            "Build": LOCAL,
        },
    },
    "release.yml": {
        "tag": {
            "the tagged commit is on main": "judges a pushed tag against origin/main",
            "The tagged commit passed CI": "asks GitHub about CI on a pushed tag's commit; the script's paths are checked against real runs and a stand-in",
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
            "The release notes, for the release page": LOCAL,
            "Build the SBOMs": LOCAL,
            "Compute SHA256SUMS": "needs every leg's artifacts",
            "Attest the release assets": "asks the host to sign; the SBOM it covers is built above",
        },
        "verify-install": {
            "Minimal prerequisites (as a stranger would have)": "installs from a published release",
            "Run the published installer": "installs from a published release",
            "It names the release and its commit, and serves the monitor": "checks a just-published release; the script is run below against the latest one",
        },
        "verify-install-native": {
            "Run the published installer (macOS)": "macOS runner, installs from a published release",
            "Run the published installer (Windows)": "Windows runner, installs from a published release",
            "It names the release and its commit, and serves the monitor (macOS)": "macOS runner; the same script is run below on Linux",
            "It names the release and its commit, and serves the monitor (Windows)": "Windows runner; the script is parsed by the installers job",
        },
        "monitor": {
            "Build the monitor": LOCAL,
            "Pack the monitor bundle": LOCAL,
            "Check the monitor bundle": LOCAL,
        },
        "vscode": {
            "Install the extension's dependencies": "the same install ci.yml's vscode job runs locally",
            "Package the extension": "packages from the tag; ci.yml's vscode job packages the same way locally",
        },
        "openvsx": {
            "Install the extension's dependencies": "publishes to Open VSX",
            "Publish to Open VSX": "publishes to Open VSX",
        },
        "image": {
            "Lowercase image name": "names the GHCR image; the image build itself is checked below",
            "The commit and its date, for the image's --version": "feeds the image build; checked below with the same values",
            "The image's kanbanr is the released program (its commit, date and skill)": "checks the image built for scanning; run below on the image make ci builds",
            "The image reports a serving monitor healthy": "checks the image built for scanning; run below on the image make ci builds",
            "Attest the image": "asks the host to sign a pushed digest",
        },
        "crates": {
            "Publish ears-classifier": "publishes to crates.io",
            "Publish mermaid-lint": "publishes to crates.io",
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


# The machine's build lock and a memory cap for make ci's heavy steps (MNT-007): another heavy
# build on the machine queues behind this one rather than racing it, and a runaway build is killed
# before the desktop is. systemd-run sits outside flock, so the lock is held only while the capped
# process runs. Without the tools (no flock, no user session manager: a container, macOS) a heavy
# step runs plainly, and make ci says so once at the start.
LOCK = Path.home() / ".cache/heavy-build.lock"
MEMINFO = Path("/proc/meminfo")
# scripts/test.sh runs cargo nextest or cargo test (MNT-010); the Test steps name only the script.
HEAVY = re.compile(r"\bcargo\b|\bdocker build\b|\bnpm (ci|run build)\b|\bscripts/test\.sh\b")


def memory_cap() -> str:
    """KANBANR_CI_MEMORY_MAX, else 60% of MemTotal in bytes, else "max" (no cap; systemd accepts it)."""
    if os.environ.get("KANBANR_CI_MEMORY_MAX"):
        return os.environ["KANBANR_CI_MEMORY_MAX"]
    try:
        found = re.search(r"^MemTotal:\s+(\d+)\s+kB", MEMINFO.read_text(), re.M)
    except OSError:
        found = None
    return str(int(int(found.group(1)) * 1024 * 0.6)) if found else "max"


def has_user_manager(env: dict[str, str]) -> bool:
    return bool(shutil.which("systemd-run")) and bool(env.get("XDG_RUNTIME_DIR"))


def heavy(cmd: list[str], env: dict[str, str]) -> list[str]:
    """`cmd` under the machine's build lock and a memory cap, as far as this machine has the tools."""
    if shutil.which("flock"):
        cmd = ["flock", str(LOCK), *cmd]
    if has_user_manager(env):
        cmd = ["systemd-run", "--user", "--scope", "-q", "-p", f"MemoryMax={memory_cap()}", *cmd]
    return cmd


_wrap_heavy = heavy  # sh()'s parameter is named heavy too


def is_heavy(script: str) -> bool:
    """A step that builds or tests Rust, builds the monitor or builds the image."""
    return HEAVY.search(script) is not None


def build_jobs() -> str:
    """CARGO_BUILD_JOBS, else the machine's CPUs minus two (two are left for the desktop)."""
    return os.environ.get("CARGO_BUILD_JOBS") or str(max(1, (os.cpu_count() or 1) - 2))


def heavy_notice(env: dict[str, str]) -> str | None:
    """The one line make ci prints when a heavy step will run without the lock or the cap."""
    missing = []
    if not shutil.which("flock"):
        missing.append("the machine's build lock (no flock)")
    if not has_user_manager(env):
        missing.append("a memory cap (no systemd-run user session)")
    if not missing:
        return None
    return "make ci: heavy steps run without " + " and without ".join(missing)


def sh(cmd: list[str] | str, cwd: Path, env: dict[str, str] | None = None,
       heavy: bool = False) -> tuple[bool, str]:
    capped = False
    if heavy and isinstance(cmd, list):
        wrap_env = env if env is not None else dict(os.environ)
        cmd = _wrap_heavy(cmd, wrap_env)
        capped = cmd[0] == "systemd-run"
    proc = subprocess.run(
        cmd,
        cwd=cwd,
        env=env,
        shell=isinstance(cmd, str),
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
    )
    out = proc.stdout
    if capped and proc.returncode in (-9, 137):
        out += (f"\nkilled: the step reached make ci's memory cap ({cmd[5]}; "
                "set KANBANR_CI_MEMORY_MAX to change it)\n")
    return proc.returncode == 0, out


def keep_run(output: str, junit: Path, head: str, now: datetime, runs: Path = RUNS) -> Path:
    """Keep a test step's full output and its JUnit report in a new folder under `runs`.

    The folder is `<YYYYMMDDTHHMMSSZ>-<sha7>`, with `-2`, `-3` … when that exists, so names sort
    by time and an earlier run is never overwritten. Without a report it says why in
    `junit.missing`.
    """
    base = f"{now:%Y%m%dT%H%M%SZ}-{head[:7]}"
    runs.mkdir(parents=True, exist_ok=True)
    folder, n = runs / base, 1
    while True:
        try:
            folder.mkdir()
            break
        except FileExistsError:
            n += 1
            folder = runs / f"{base}-{n}"
    (folder / "test.log").write_text(output, encoding="utf-8")
    if junit.is_file():
        shutil.copyfile(junit, folder / "junit.xml")
    else:
        (folder / "junit.missing").write_text("no JUnit report: the step ran cargo test, which writes none")
    return folder


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
        ["cargo", "metadata", "--locked", "--no-deps", "--format-version", "1"], cwd=TREE / "api", text=True
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
                    **LOCAL_VARS,
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
                        **os.environ, "CARGO_BUILD_JOBS": build_jobs(), **job_env, **step_env,
                        "CI": "true", "GITHUB_SHA": head, "GITHUB_REF_NAME": f"v{version}",
                        "GITHUB_REPOSITORY": "startr-trade/kanbanr",
                        "GITHUB_OUTPUT": f"{scratch}/output", "GITHUB_ENV": f"{scratch}/env",
                        "GITHUB_PATH": f"{scratch}/path", "GITHUB_STEP_SUMMARY": f"{scratch}/summary",
                        "RUNNER_TEMP": scratch,
                    }
                    if step.get("shell") == "pwsh":
                        rel = wd.relative_to(TREE)
                        ok, out = sh(["docker", "run", "--rm", "-v", f"{TREE}:/repo", "-w", f"/repo/{rel}",
                                      PWSH, "pwsh", "-NoProfile", "-Command", script], TREE)
                    else:
                        ok, out = sh(["bash", "--noprofile", "--norc", "-eo", "pipefail", "-c", script], wd, env,
                                     heavy=is_heavy(script))
                say(label, ok, out)
                if (wf_name, job_id, name) in KEEP:
                    # Keeping a run never fails it.
                    try:
                        kept = keep_run(out, TREE / "api/target/nextest/default/junit.xml", head,
                                        datetime.now(timezone.utc))
                        print(f"    kept: {kept}", flush=True)
                    except OSError as err:
                        print(f"    not kept: {err}", flush=True)
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
    container_steps_are_posix()
    vars_are_variables()


def container_steps_are_posix() -> None:
    """A job in a `container:` runs its steps with `sh`, not bash, unless it says otherwise.

    On Debian and Ubuntu that is dash, which stops on a bashism such as `${GITHUB_SHA:0:12}` —
    how v0.1.0's installer check failed in public (FEAT-143). actionlint shellchecks every run
    block as bash, so it cannot see this; here those blocks are shellchecked as POSIX sh.
    """
    problems = []
    for wf in sorted((TREE / ".github/workflows").glob("*.yml")):
        doc = yaml.safe_load(wf.read_text())
        wf_shell = ((doc.get("defaults") or {}).get("run") or {}).get("shell")
        for job_id, job in (doc.get("jobs") or {}).items():
            if not job.get("container"):
                continue
            job_shell = ((job.get("defaults") or {}).get("run") or {}).get("shell") or wf_shell
            for step in job.get("steps", []):
                if "run" not in step or (step.get("shell") or job_shell or "sh").split()[0] != "sh":
                    continue
                script = re.sub(r"\$\{\{.*?\}\}", "x", step["run"])
                with tempfile.NamedTemporaryFile("w", suffix=".sh") as f:
                    f.write(script)
                    f.flush()
                    ok, out = sh(["shellcheck", "-s", "sh", "-S", "warning", f.name], TREE)
                if not ok:
                    problems.append(f"{wf.name} {job_id} / {step.get('name')}:\n{out}")
    say("workflows: container-job steps are POSIX sh (their default shell)", not problems,
        "\n".join(problems))


def verify_install_script() -> None:
    """release.yml's installer check, run as the release runs it — install.sh in a bare container,
    then scripts/verify-install.sh under its sh — against the latest published release (FEAT-149).

    The release can only run it on a release it has just published; this is the one place the
    script itself is exercised before a tag depends on it.
    """
    ok, tag = sh(["gh", "release", "view", "-R", "startr-trade/kanbanr", "--json", "tagName", "-q", ".tagName"], TREE)
    ok2, commit = sh(["gh", "api", f"repos/startr-trade/kanbanr/commits/{tag.strip()}", "-q", ".sha"], TREE)
    if not (ok and ok2):
        say("release verify-install: scripts/verify-install.sh against the latest release", False,
            "could not resolve the latest release with gh:\n" + tag + commit)
        return
    tag, commit = tag.strip(), commit.strip()
    script = (
        "set -eu; apt-get update -qq >/dev/null; "
        "apt-get install -y -qq --no-install-recommends curl ca-certificates >/dev/null; "
        f"curl -fsSL https://github.com/startr-trade/kanbanr/releases/download/{tag}/install.sh "
        f"| sh -s -- --version {tag} --dir /usr/local/bin --no-skill; "
        "sh scripts/verify-install.sh"
    )
    # Started from the repository root, as release.yml starts it — the checkout's .kanbanr marker
    # is part of what the script has to cope with (FEAT-155).
    ok, out = sh(["docker", "run", "--rm", "-v", f"{TREE}:/repo:ro", "-w", "/repo", "-e", f"GITHUB_REF_NAME={tag}",
                  "-e", f"GITHUB_SHA={commit}", "debian:bookworm-slim", "sh", "-c", script], TREE)
    say(f"release verify-install: scripts/verify-install.sh against the latest release ({tag}, debian:bookworm-slim)",
        ok, out)


def vars_are_variables() -> None:
    """`vars.NAME` reads a repository *variable*; a secret of the same name is invisible to it.

    PUBLISH_CRATES created as a secret silently skipped the crates.io publish of v0.1.0
    (FEAT-143). Only a name held as a secret and not as a variable fails: an unset variable is
    a switch left off on purpose.
    """
    names = sorted(set(re.findall(r"\bvars\.([A-Za-z_][A-Za-z0-9_]*)",
                                  "\n".join(p.read_text() for p in (TREE / ".github/workflows").glob("*.yml")))))
    ok_v, variables = sh(["gh", "variable", "list", "-R", "startr-trade/kanbanr", "--json", "name", "-q", ".[].name"], TREE)
    ok_s, secrets = sh(["gh", "secret", "list", "-R", "startr-trade/kanbanr", "--json", "name", "-q", ".[].name"], TREE)
    if not (ok_v and ok_s):
        say("workflows: vars.* are repository variables, not secrets", False,
            "could not list the repository's variables and secrets with gh:\n" + variables + secrets)
        return
    wrong = [n for n in names if n in secrets.split() and n not in variables.split()]
    say(f"workflows: vars.* are repository variables, not secrets ({', '.join(names) or 'none'})", not wrong,
        "held as a secret, so `vars.` reads it as empty: " + ", ".join(wrong)
        + " — create it with `gh variable set NAME --body …` and delete the secret")


def image_build_command(sha: str, date: str, refresh: str) -> list[str]:
    """The local release image build. --pull takes the registry's current base image, and the
    refresh argument (declared before the Dockerfile's apt-get upgrade layer) changes every run, so
    that layer is never taken from the cache and hides a fixable finding (MNT-006)."""
    return ["docker", "build", "--pull", "-q", "-f", "docker/Dockerfile", "-t", "kanbanr:ci-local",
            "--build-arg", f"KANBANR_GIT_SHA={sha}",
            "--build-arg", f"KANBANR_BUILD_DATE={date}",
            "--build-arg", f"KANBANR_APT_REFRESH={refresh}", "."]


def refresh_token() -> str:
    """A value that differs on every call, down to the microsecond."""
    return datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%S%f")


def dockerfile_refreshes_packages(text: str | None = None) -> str | None:
    """A problem sentence when the Dockerfile's final stage would let its apt-get upgrade layer come
    from the cache, else None: the last ARG KANBANR_APT_REFRESH must follow the last FROM and come
    before the first RUN that upgrades, and that RUN must use it."""
    source = text if text is not None else (TREE / "docker/Dockerfile").read_text()
    # One instruction per entry: a trailing backslash continues onto the next line.
    steps = [re.sub(r"\\\n\s*", " ", step).strip() for step in re.split(r"(?<!\\)\n", source)]
    steps = [step for step in steps if step and not step.startswith("#")]
    froms = [i for i, s in enumerate(steps) if s.startswith("FROM ")]
    args = [i for i, s in enumerate(steps) if s.startswith("ARG KANBANR_APT_REFRESH")]
    runs = [i for i, s in enumerate(steps) if s.startswith("RUN ") and "apt-get upgrade" in s]
    if not froms or not runs:
        return "docker/Dockerfile has no final stage with an apt-get upgrade layer"
    run = next((r for r in runs if r > froms[-1]), None)
    if run is None:
        return "the final stage has no apt-get upgrade layer"
    if not args or args[-1] < froms[-1]:
        return "the final stage does not declare ARG KANBANR_APT_REFRESH"
    if args[-1] > run:
        return "ARG KANBANR_APT_REFRESH is declared after the apt-get upgrade layer, so it cannot change it"
    if "KANBANR_APT_REFRESH" not in steps[run]:
        return "the apt-get upgrade layer does not use KANBANR_APT_REFRESH"
    return None


def uses_equivalents() -> None:
    """What workflows do through actions rather than scripts, done the same way here."""
    want = yaml.safe_load((TREE / ".github/workflows/docs.yml").read_text())["env"]
    ok, out = sh(["mdbook", "--version"], TREE)
    say(f"docs: mdbook is the pinned {want['MDBOOK_VERSION']}", ok and want["MDBOOK_VERSION"] in out, out)
    verify_install_script()
    _, sha = sh(["git", "rev-parse", "HEAD"], TREE)
    _, date = sh(["git", "log", "-1", "--format=%cd", "--date=short"], TREE)
    problem = dockerfile_refreshes_packages()
    say("release image: the package layer is refreshed on every local build (MNT-006)", problem is None,
        problem or "")
    ok, out = sh(image_build_command(sha.strip(), date.strip(), refresh_token()), TREE, heavy=True)
    say("release image: docker/Dockerfile builds, package layer refreshed (docker/build-push-action, without the push)", ok, out)
    ok, out = sh([str(TREE / "scripts/check-image.sh"), "kanbanr:ci-local"], TREE)
    say("release image: its kanbanr names its commit and date, and carries the repository's skill", ok, out)
    ok, out = sh([str(TREE / "scripts/check-image-health.sh"), "kanbanr:ci-local"], TREE)
    say("release image: a serving monitor reports healthy (its own HEALTHCHECK)", ok, out)
    ok, out = sh([str(TREE / "scripts/security-scan.sh"), "image", "kanbanr:ci-local"], TREE)
    say("release image: no fixable HIGH/CRITICAL finding (release.yml's scan before the push)", ok, out)
    ok, out = sh([str(TREE / "scripts/security-scan.sh"), "deps", str(TREE)], TREE)
    say("trivy.yml: no finding in dependencies, secrets or the Dockerfile (stricter than CI)", ok, out)
    not_verified.append("codeql.yml: CodeQL (run `make codeql`; several minutes, so not part of make ci)")
    not_verified.append("codeql.yml / trivy.yml: uploads to code scanning")
    not_verified.append("ci.yml rust: uploading each system's JUnit report")
    # The plugin manifests (FEAT-139): nothing checked them, and a skills path Claude Code could not
    # resolve shipped. Validated wherever the claude command is installed; said so where it is not.
    if shutil.which("claude"):
        # Fails when the validator fails; its warnings are advisory (it says they do not change the
        # verdict), so they are printed with the result and never fail it (MNT-005).
        ok, out = sh(["claude", "plugin", "validate", "."], TREE)
        say("plugin: claude plugin validate . (marketplace and plugin manifests)", ok, out)
        if ok:
            for line in out.splitlines():
                if line.strip().startswith("❯") or "warning" in line.lower():
                    print("    " + line.strip(), flush=True)
    else:
        not_verified.append("plugin: claude plugin validate . (no claude command here)")
    not_verified.append("release.yml image: pushing to GHCR")
    not_verified.append("release.yml release: creating the GitHub release")
    not_verified.append("docs.yml deploy: publishing to GitHub Pages")


# A person's verdict on the board (FEAT-198): what the monitor or the CLI commits when someone
# approves, ratifies, signs off or withdraws one, or issues, claims or revokes an agent's scope
# (FEAT-218). Tests cannot make these — the board fence stops them reaching the board — so one made
# during a run is someone at work, not a leak.
VERDICT_SUBJECTS = (
    "approve definition of ",
    "ratify ",
    "withdraw the approval of ",
    "sign off ",
    "withdraw sign-off ",
)
SCOPE_SUBJECTS = ("issue agent scope ", "claim agent scope ", "revoke agent scope ")
VERDICT_PATHS = ("features/", "activity/", "events/")
AGENT_SCOPE = re.compile(r"^Agent-Scope: (SC-\d+)\b", re.M)


def is_verdict_path(path: str) -> bool:
    """`projects/<id>/features/…`, `…/activity/…` or `…/events/…`, and nothing else."""
    parts = path.split("/", 2)
    return len(parts) == 3 and parts[0] == "projects" and parts[2].startswith(VERDICT_PATHS)


def is_verdict(subject: str, files: list[str]) -> bool:
    """A person's verdict, or their act on an agent's scope, touching only what that writes."""
    if subject.startswith(VERDICT_SUBJECTS):
        return all(is_verdict_path(f) for f in files)
    if subject.startswith(SCOPE_SUBJECTS):
        return all(
            is_verdict_path(f) or (f.startswith("projects/") and f.endswith("/scopes.yaml") and f.count("/") == 2)
            for f in files
        )
    return False


def when(value: object) -> datetime | None:
    """An RFC 3339 time as kanbanr writes it (nanoseconds, `Z`), or one YAML already parsed."""
    if isinstance(value, datetime):
        return value if value.tzinfo else value.replace(tzinfo=timezone.utc)
    if not isinstance(value, str) or not value:
        return None
    text = re.sub(r"(\.\d{6})\d+", r"\1", value.strip()).replace("Z", "+00:00")
    try:
        parsed = datetime.fromisoformat(text)
    except ValueError:
        return None
    return parsed if parsed.tzinfo else None


def scoped_write(board: Path, sha: str, files: list[str], subject: str, body: str) -> str | None:
    """The scope id when commit `sha` is a build agent's own progress record (FEAT-218): an
    `Agent-Scope:` trailer naming a build scope, read from the board at that commit, that was active
    at its time; a task or test update of the scope's item; touching only that item's file, the
    activity, the events and the index. None for anything else, or anything unreadable."""
    found = AGENT_SCOPE.search(body)
    if not found or not files or not all(f.startswith("projects/") and f.count("/") >= 2 for f in files):
        return None
    projects = {f.split("/")[1] for f in files}
    if len(projects) != 1:
        return None
    scope_id, project = found.group(1), projects.pop()
    ok, text = sh(["git", "show", f"{sha}:projects/{project}/scopes.yaml"], board)
    try:
        scopes = yaml.safe_load(text) if ok else None
    except yaml.YAMLError:
        return None
    scope = next((s for s in scopes or [] if isinstance(s, dict) and s.get("id") == scope_id), None)
    if scope is None or scope.get("role") != "build" or not scope.get("item"):
        return None
    item = str(scope["item"])
    ok, at = sh(["git", "log", "-1", "--format=%cI", sha], board)
    made, start = when(at.strip()) if ok else None, when(scope.get("claimed_at") or scope.get("issued_at"))
    ends = [t for t in (when(scope.get("expires_at")), when(scope.get("revoked_at"))) if t]
    # A commit's time has whole seconds; the claim's has nanoseconds.
    if made is None or start is None or made < start.replace(microsecond=0) or any(made >= t for t in ends):
        return None
    if not (
        (subject.startswith("update task ") and f"({item}/" in subject)
        or (subject.startswith("test ") and f" of {item}/R-" in subject)
        or re.fullmatch(r"record \d+ test result\(s\) from a run", subject)
    ):
        return None
    root = f"projects/{project}/"
    for f in files:
        rest = f[len(root):]
        own = rest.startswith("features/") and rest.count("/") == 2 and rest.endswith(f"/{item}.yaml")
        if not (own or rest.startswith(("activity/", "events/")) or rest == "index.yaml"):
            return None
    return scope_id


def allowed_during_run(board: Path, before: str, after: str) -> list[str] | None:
    """What the commits `after` has beyond `before` were, one line each, when every one is a
    person's verdict (`verdict: …`) or an agent's scoped write (`SC-4 for FEAT-1: …`); None when
    any is neither, or history was rewritten."""
    if not sh(["git", "merge-base", "--is-ancestor", before, after], board)[0]:
        return None
    ok, shas = sh(["git", "rev-list", "--reverse", f"{before}..{after}"], board)
    if not ok:
        return None
    lines = []
    for sha in shas.split():
        ok, subject = sh(["git", "log", "-1", "--format=%s", sha], board)
        ok2, body = sh(["git", "log", "-1", "--format=%b", sha], board)
        ok3, out = sh(["git", "diff-tree", "--no-commit-id", "--name-only", "-r", sha], board)
        if not (ok and ok2 and ok3):
            return None
        subject, files = subject.strip(), [f for f in out.split("\n") if f.strip()]
        if is_verdict(subject, files):
            lines.append(f"verdict: {subject}")
            continue
        scope = scoped_write(board, sha, files, subject, body)
        if scope is None:
            return None
        lines.append(f"{scope}: {subject}")
    return lines


def board_state() -> tuple[Path, str, str] | None:
    """The real board this checkout uses, its HEAD and its uncommitted changes (FEAT-195).

    None when there is no board to protect: no `kanbanr` on PATH, no marker, or a board that is not
    a git repository (a runner, a fresh clone)."""
    exe = shutil.which("kanbanr")
    if not exe:
        return None
    env = {k: v for k, v in os.environ.items() if k not in ("KANBANR_BOARD_FENCE", "KANBANR_DATA_DIR")}
    ok, out = sh([exe, "where", "--json"], ROOT, env)
    if not ok:
        return None
    try:
        import json

        board = Path(json.loads(out)["data_dir"])
    except (ValueError, KeyError, TypeError):
        return None
    head_ok, head = sh(["git", "rev-parse", "HEAD"], board)
    status_ok, status = sh(["git", "status", "--porcelain"], board)
    if not (board.is_dir() and head_ok and status_ok):
        return None
    return board, head.strip(), status


def gate_record_args(ok: bool, head: str, dirty: bool, failed_steps: list[str]) -> list[str] | None:
    """The `kanbanr` arguments that record this run as the gate `local` for `head` (FTR-002).

    A failure is always recorded; a pass only when the working tree is clean, because a pass for a
    tree with uncommitted changes is not about `head`. None means record nothing."""
    base = ["gate", "record", "local", "--source", "local", "--commit", head]
    if not ok:
        return [*base, "--failed", "--note", "failed: " + ", ".join(failed_steps)]
    if dirty:
        return None
    return [*base, "--passed"]


def record_gate(ok: bool, failed_steps: list[str]) -> None:
    """Record the run's verdict on the board, after every check. It never changes the exit code."""
    exe = shutil.which("kanbanr")
    if not exe:
        print("gate local not recorded: kanbanr is not on PATH")
        return
    head_ok, head = sh(["git", "rev-parse", "HEAD"], ROOT)
    status_ok, status = sh(["git", "status", "--porcelain"], ROOT)
    if not (head_ok and status_ok):
        print("gate local not recorded: could not read the repository's HEAD and status")
        return
    args = gate_record_args(ok, head.strip(), bool(status.strip()), failed_steps)
    if args is None:
        print("gate local not recorded: uncommitted changes")
        return
    env = {k: v for k, v in os.environ.items() if k not in ("KANBANR_BOARD_FENCE", "KANBANR_DATA_DIR")}
    rec_ok, out = sh([exe, *args], ROOT, env)
    last = out.strip().splitlines()[-1] if out.strip() else ""
    print(f"gate local {'recorded' if rec_ok else 'not recorded'}" + (f": {last}" if last else ""))


def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    """Read the command line before any work starts: `--help` and a mistyped flag run no check."""
    parser = argparse.ArgumentParser(
        prog="scripts/ci_local.py",
        description=(__doc__ or "").split("\n\n")[0],
        epilog="Takes no other arguments; it always runs every check.",
    )
    return parser.parse_args(argv)


def unittest_by_name(file: str) -> list[str]:
    """Run one test file so unittest prints each test's line under the bare file name as its module
    (`test_x (test_ci_local.Class.test_x) ... ok`), the names the board tracks (MNT-007, DEF-068)."""
    return [sys.executable, "-m", "unittest", "discover", "-s", "scripts/tests", "-p", file, "-v"]


def main(argv: list[str] | None = None) -> int:
    parse_args(argv)
    print(f"make ci: a clean copy of the tracked tree at {TREE}", flush=True)
    notice = heavy_notice(dict(os.environ))
    if notice:
        print(notice, flush=True)
    if shutil.which("flock"):
        LOCK.parent.mkdir(parents=True, exist_ok=True)
    print(flush=True)
    # A test that reaches the real board writes to someone's data, and its push publishes it.
    # The fence (api/.cargo/config.toml) stops that; this proves it held for the whole run.
    board_before = board_state()
    ok, out = sh(unittest_by_name("test_ci_local.py"), ROOT)
    say("make ci: verdicts and agents' scoped writes on the board are told apart from other changes", ok, out)
    ok, out = sh(unittest_by_name("test_check_supply_chain.py"), ROOT)
    say("make ci: unlocked builds, npm install and an unattested release are caught (MNT-004)", ok, out)
    ok, out = sh(unittest_by_name("test_guides.py"), ROOT)
    say("make ci: the guides cover the pipeline and a product people use (DOC-007)", ok, out)
    ok, out = sh(unittest_by_name("test_privacy_scan.py"), ROOT)
    say("make ci: the privacy scan lists what a public board would show (MNT-012)", ok, out)
    fresh_tree()
    lint_workflows()
    run_workflow_steps()
    uses_equivalents()
    if board_before is None:
        not_verified.append("the real board stayed untouched: no board found for this checkout")
    else:
        board, head, status = board_before
        after = board_state()
        verdicts: list[str] | None = []
        if after is None or after[2] != status:
            verdicts = None
        elif after[1] != head:
            verdicts = allowed_during_run(board, head, after[1])
        say(
            "make ci: the real board did not move during the run",
            verdicts is not None,
            f"{board} changed while make ci ran: a test reached it, or it was written to meanwhile "
            "(then rerun).\nbefore:\n" + head + "\n" + status + "\nafter:\n"
            + ("\n".join(after[1:]) if after else "unreadable"),
        )
        if verdicts:
            people = sum(1 for v in verdicts if v.startswith("verdict: "))
            print(
                f"    the real board took {people} verdict(s) and {len(verdicts) - people} agent "
                "write(s) during the run: " + "; ".join(verdicts)
            )
    failed = [name for name, ok, _ in results if not ok]
    record_gate(not failed, failed)
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

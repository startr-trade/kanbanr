"""The pipeline follows the shared Rust CI practices (MNT-010; MNT-007 adds its classes here).

These read the workflow, the Makefile, make ci's plan and the nextest config themselves, not a
copy of them (ADR-0008): the property is asserted where it lives.
"""

from __future__ import annotations

import re
import sys
import tomllib
import unittest
from pathlib import Path

import yaml

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))
import ci_local  # noqa: E402

WORKFLOWS = ROOT / ".github" / "workflows"
CI = WORKFLOWS / "ci.yml"
RELEASE = WORKFLOWS / "release.yml"
DEPS = WORKFLOWS / "deps.yml"
TRIVY = WORKFLOWS / "trivy.yml"
TOOLCHAIN_FILE = ROOT / "rust-toolchain.toml"
WORKSPACE_MANIFEST = ROOT / "api" / "Cargo.toml"
DOCKERFILE = ROOT / "docker" / "Dockerfile"
MAKEFILE = ROOT / "Makefile"
NEXTEST = ROOT / "api" / ".config" / "nextest.toml"
JUNIT = "api/target/nextest/default/junit.xml"


def rust_steps() -> list[dict]:
    return yaml.safe_load(CI.read_text())["jobs"]["rust"]["steps"]


def step(steps: list[dict], name: str) -> dict:
    found = [s for s in steps if s.get("name") == name]
    if len(found) != 1:
        raise AssertionError(f"ci.yml rust: expected one step named {name!r}, found {len(found)}")
    return found[0]


def recipe(target: str) -> list[str]:
    """The Makefile recipe lines of `target`, without their leading tab."""
    lines = MAKEFILE.read_text().splitlines()
    head = re.compile(rf"^{re.escape(target)}:(\s|$)")
    for i, line in enumerate(lines):
        if head.match(line):
            body = []
            for rest in lines[i + 1:]:
                if not rest.startswith("\t"):
                    break
                body.append(rest[1:].strip())
            return body
    raise AssertionError(f"Makefile has no target {target!r}")


class Tests(unittest.TestCase):
    """MNT-010: one test script, a nextest config that times every test, and CI that keeps it."""

    def test_ci_make_test_and_make_ci_run_the_one_test_script(self) -> None:
        """R-1: ci.yml's Test step, `make test` and make ci all run scripts/test.sh."""
        test = step(rust_steps(), "Test")
        self.assertEqual(test.get("run", "").strip(), "../scripts/test.sh")
        self.assertEqual(test.get("shell"), "bash")
        self.assertEqual(recipe("test"), ["scripts/test.sh"])
        self.assertEqual(ci_local.PLAN["ci.yml"]["rust"]["Test"], ci_local.LOCAL)
        stamp = step(rust_steps(), "Build stamp follows HEAD in a worktree")
        self.assertEqual(stamp.get("run", "").strip(),
                         "../scripts/test.sh --ignored -p kanbanr-cli --test build_stamp")
        self.assertIn("scripts/test.sh --ignored -p kanbanr-cli", recipe("itest"))

    def test_nextest_config_writes_junit_names_slow_tests_and_never_fails_fast(self) -> None:
        """R-4: the default profile writes a JUnit report, names a test past 60 s, and never stops early."""
        profile = tomllib.loads(NEXTEST.read_text())["profile"]["default"]
        self.assertEqual(profile["junit"]["path"], "junit.xml")
        slow = profile["slow-timeout"]
        self.assertEqual(slow["period"], "60s")
        self.assertNotIn("terminate-after", slow)
        self.assertIs(profile["fail-fast"], False)

    def test_ci_installs_a_pinned_nextest_checks_and_uploads_each_systems_junit_report(self) -> None:
        """R-5: a pinned nextest is installed before Test, and each system's report is checked and kept."""
        steps = rust_steps()
        names = [s.get("name") for s in steps]
        test_at = names.index("Test")
        installs = [i for i, s in enumerate(steps)
                    if "taiki-e/install-action@" in str(s.get("uses", ""))
                    and (s.get("with") or {}).get("tool") == "cargo-nextest@0.9.151"]
        self.assertTrue(installs, "no install-action step installs cargo-nextest@0.9.151")
        self.assertLess(installs[0], test_at, "cargo-nextest is installed after the Test step")
        self.assertRegex(steps[installs[0]]["uses"], r"@[0-9a-f]{40}\b")

        check = step(steps, "The run left a JUnit report with a time for every test")
        self.assertGreater(names.index(check["name"]), test_at)
        cond = check.get("if", "")
        self.assertIn("!cancelled()", cond)
        self.assertIn("runner.os == 'Linux'", cond)
        self.assertIn("scripts/check_junit.py target/nextest/default/junit.xml", check.get("run", ""))

        uploads = [s for s in steps if "actions/upload-artifact@" in str(s.get("uses", ""))]
        self.assertEqual(len(uploads), 1, "the rust job should upload one artifact, the JUnit report")
        upload = uploads[0]
        self.assertGreater(steps.index(upload), test_at)
        self.assertRegex(upload["uses"], r"@[0-9a-f]{40}\b")
        self.assertEqual(upload.get("if"), "${{ !cancelled() }}")
        self.assertEqual(upload["with"]["path"], JUNIT)
        self.assertIn("matrix.os", upload["with"]["name"])



# --- MNT-007: the shared Rust CI practices, each held where it lives (ADR-0008) ---------------

BUILD_ENV = {"CARGO_INCREMENTAL": "0", "CARGO_PROFILE_DEV_DEBUG": "0", "CARGO_PROFILE_TEST_DEBUG": "0"}
MAIN_ONLY = "${{ github.ref == 'refs/heads/main' }}"
# What deps.yml watches: a change to any of these is the only thing a dependency proof can catch.
DEPENDENCY_PATHS = {
    "api/Cargo.lock", "**/Cargo.toml", "rust-toolchain.toml", "api/.cargo/**", "api/deny.toml",
    "web/package.json", "web/package-lock.json", "editor/vscode/package.json",
    "editor/vscode/package-lock.json", "scripts/check_supply_chain.py", "scripts/make-sbom.sh",
    ".github/workflows/deps.yml",
}
# The proofs only a dependency or toolchain change can affect (R-6), as a step's run text shows them.
DEPENDENCY_PROOFS = {
    "cargo deny": re.compile(r"\bcargo deny\b"),
    "cargo audit": re.compile(r"\bcargo audit\b"),
    "npm audit": re.compile(r"\bnpm audit\b"),
    "the locked-build and clean-install check": re.compile(r"check_supply_chain\.py"),
    "the SBOM build": re.compile(r"make-sbom\.sh"),
    "the TLS-backend check": re.compile(r"\bcargo tree\b[^\n]*\brustls\b"),
}
# The release jobs that build or publish something; each waits for the dependency tier.
RELEASE_BUILD_JOBS = ("binaries", "vscode", "monitor", "image", "crates")


def workflow(path: Path) -> dict:
    return yaml.safe_load(path.read_text())


def triggers(doc: dict) -> dict:
    """The `on:` block; PyYAML reads the bare key `on` as the boolean True."""
    on = doc.get("on", doc.get(True))
    return on if isinstance(on, dict) else {name: None for name in (on or [])}


def jobs(doc: dict) -> dict[str, dict]:
    return doc.get("jobs") or {}


def steps_of(job: dict) -> list[dict]:
    return job.get("steps") or []


def uses(step_: dict, action: str) -> bool:
    return str(step_.get("uses", "")).split("@", 1)[0] == action


def is_compiling(job: dict) -> bool:
    """A job that compiles Rust: a step runs `cargo ` or sets up the toolchain (the Design's rule)."""
    return any("cargo " in str(s.get("run", "")) or uses(s, "dtolnay/rust-toolchain")
               for s in steps_of(job))


def channel() -> str:
    return tomllib.loads(TOOLCHAIN_FILE.read_text())["toolchain"]["channel"]


def rust_version() -> str:
    manifest = tomllib.loads(WORKSPACE_MANIFEST.read_text())
    return (manifest.get("workspace", {}).get("package", {}).get("rust-version")
            or manifest["package"]["rust-version"])


def compiling_workflows() -> list[Path]:
    return [CI, RELEASE, DEPS]


class Toolchain(unittest.TestCase):
    """R-7: one toolchain, named once in rust-toolchain.toml and held to it everywhere else."""

    def test_every_workflow_and_the_dockerfile_name_the_pinned_toolchain(self) -> None:
        pinned, msrv = channel(), rust_version()
        seen = 0
        for path in sorted(WORKFLOWS.glob("*.yml")):
            for name, job in jobs(workflow(path)).items():
                for s in steps_of(job):
                    if not uses(s, "dtolnay/rust-toolchain"):
                        continue
                    seen += 1
                    want = msrv if (path == DEPS and name == "msrv") else pinned
                    got = (s.get("with") or {}).get("toolchain")
                    self.assertEqual(None if got is None else str(got), want,
                                     f"{path.name} job {name!r}: dtolnay/rust-toolchain names "
                                     f"{got!r}, not {want!r}")
        self.assertGreater(seen, 0, "no workflow sets up the Rust toolchain")
        bases = re.findall(r"(?im)^FROM\s+rust:(\S+)", DOCKERFILE.read_text())
        self.assertTrue(bases, "docker/Dockerfile has no `FROM rust:` stage")
        for tag in bases:
            self.assertEqual(tag.split("-", 1)[0], pinned,
                             f"docker/Dockerfile builds on rust:{tag}, not the pinned {pinned}")


class BuildEnvironment(unittest.TestCase):
    """R-2: every compiling job runs with no incremental files and no debug info."""

    def test_every_compiling_job_sets_the_ci_build_environment(self) -> None:
        for path in compiling_workflows():
            self.assertTrue(path.exists(), f"{path.name} is missing")
            doc = workflow(path)
            for name, job in jobs(doc).items():
                if not is_compiling(job):
                    continue
                env = {**(doc.get("env") or {}), **(job.get("env") or {})}
                for var, value in BUILD_ENV.items():
                    self.assertEqual(str(env.get(var)), value,
                                     f"{path.name} job {name!r} compiles Rust without {var}={value}")


# A job that compiles or tests the workspace may take an hour (the cold rust job with vendored C
# takes about 25 minutes on Windows); every other job, at most half that.
COMPILING_MINUTES = 60
OTHER_MINUTES = 30
WORKSPACE_BUILD = re.compile(
    r"\bcargo (?:build|check|clippy|nextest|run|test)\b|\bscripts/test\.sh\b|\bdocker build\b")


def builds_the_workspace(job: dict) -> bool:
    """A job that compiles or tests the workspace, or builds the image (which compiles the binary)."""
    return any(WORKSPACE_BUILD.search(str(s.get("run", ""))) or uses(s, "docker/build-push-action")
               for s in steps_of(job))


class Timeouts(unittest.TestCase):
    """R-2: a hung job fails within the hour instead of holding a runner for six."""

    def test_every_job_has_a_timeout_and_compiling_jobs_get_sixty_minutes(self) -> None:
        for path in [*compiling_workflows(), TRIVY]:
            self.assertTrue(path.exists(), f"{path.name} is missing")
            for name, job in jobs(workflow(path)).items():
                # A job that calls a reusable workflow cannot set timeout-minutes (GitHub refuses
                # it); the called workflow's own jobs carry theirs, and deps.yml is checked here.
                if "uses" in job:
                    continue
                minutes = job.get("timeout-minutes")
                self.assertIsInstance(minutes, int,
                                      f"{path.name} job {name!r} has no timeout-minutes")
                self.assertGreater(minutes, 0, f"{path.name} job {name!r}: timeout-minutes {minutes}")
                limit = COMPILING_MINUTES if builds_the_workspace(job) else OTHER_MINUTES
                self.assertLessEqual(minutes, limit,
                                     f"{path.name} job {name!r}: timeout-minutes {minutes}, "
                                     f"over {limit}")


class Cache(unittest.TestCase):
    """R-3: a pull request restores main's cache and never saves its own; the release keeps its own."""

    def test_rust_cache_saves_only_from_main_under_a_shared_key(self) -> None:
        ci_keys = set()
        found = 0
        for name, job in jobs(workflow(CI)).items():
            for s in steps_of(job):
                if not uses(s, "Swatinem/rust-cache"):
                    continue
                found += 1
                with_ = s.get("with") or {}
                self.assertEqual(with_.get("save-if"), MAIN_ONLY,
                                 f"ci.yml job {name!r}: rust-cache saves from more than main")
                key = str(with_.get("shared-key", ""))
                self.assertIn("matrix.os", key,
                              f"ci.yml job {name!r}: rust-cache has no shared-key per runner image")
                ci_keys.add(key)
        self.assertGreater(found, 0, "ci.yml has no rust-cache step")
        found = 0
        for name, job in jobs(workflow(RELEASE)).items():
            for s in steps_of(job):
                if not uses(s, "Swatinem/rust-cache"):
                    continue
                found += 1
                key = str((s.get("with") or {}).get("shared-key", ""))
                self.assertIn("matrix.target", key,
                              f"release.yml job {name!r}: rust-cache has no shared-key per target")
                self.assertNotIn(key, ci_keys, f"release.yml job {name!r} shares CI's cache key")
        self.assertGreater(found, 0, "release.yml has no rust-cache step")


class Tiers(unittest.TestCase):
    """R-6: the dependency-only proofs run in deps.yml, on dependency changes, nightly, on demand and at release."""

    def test_dependency_proofs_run_on_dependency_changes_nightly_on_demand_and_at_release(self) -> None:
        self.assertTrue(DEPS.exists(), ".github/workflows/deps.yml is missing")
        doc = workflow(DEPS)
        on = triggers(doc)
        push = on.get("push") or {}
        self.assertEqual(push.get("branches"), ["main"])
        self.assertEqual(set(push.get("paths") or []), DEPENDENCY_PATHS)
        self.assertEqual(set((on.get("pull_request") or {}).get("paths") or []), DEPENDENCY_PATHS)
        self.assertTrue(any(isinstance(c, dict) and c.get("cron")
                            for c in (on.get("schedule") or [])), "deps.yml has no nightly schedule")
        self.assertIn("workflow_dispatch", on)
        self.assertIn("workflow_call", on)

        runs = [str(s.get("run", "")) for job in jobs(doc).values() for s in steps_of(job)]
        for proof, pattern in DEPENDENCY_PROOFS.items():
            self.assertTrue(any(pattern.search(r) for r in runs), f"deps.yml does not run {proof}")
        msrv = jobs(doc).get("msrv")
        self.assertIsNotNone(msrv, "deps.yml has no msrv job")
        self.assertTrue(any("cargo check --locked --workspace --all-targets" in str(s.get("run", ""))
                            for s in steps_of(msrv)), "the msrv job does not check the workspace")

        release = jobs(workflow(RELEASE))
        callers = [n for n, j in release.items() if j.get("uses") == "./.github/workflows/deps.yml"]
        self.assertEqual(len(callers), 1, "release.yml should call deps.yml from one job")
        for name in RELEASE_BUILD_JOBS:
            needs = release[name].get("needs") or []
            needs = [needs] if isinstance(needs, str) else needs
            self.assertIn(callers[0], needs, f"release.yml job {name!r} builds before the dependency tier")

    def test_ci_yml_holds_no_dependency_only_proof(self) -> None:
        for name, job in jobs(workflow(CI)).items():
            for s in steps_of(job):
                run = str(s.get("run", ""))
                for proof, pattern in DEPENDENCY_PROOFS.items():
                    self.assertIsNone(pattern.search(run),
                                      f"ci.yml job {name!r} step {s.get('name')!r} runs {proof}, "
                                      "which belongs in deps.yml")


# A unix-only API outside a test module must sit under `#[cfg(unix)]` (or `cfg(all(…, unix, …))`)
# or inside an `if cfg!(unix)` branch: the rule is textual, the nearest attribute or enclosing block.
UNIX_API = re.compile(r"std::os::unix|\bPermissionsExt\b|\bsymlink\(")
UNIX_ATTR = re.compile(r"#\[cfg\((?:unix\b|all\([^\]]*\bunix\b)")
UNIX_IF = re.compile(r"\bcfg!\(\s*unix\s*\)")
TEST_ATTR = re.compile(r"^#\[cfg\((?:test\b|all\([^\]]*\btest\b)")
MOD_OPEN = re.compile(r"^\s*(?:pub(?:\([\w:]+\))?\s+)?mod\s+\w+\s*\{")
# Accepted exceptions, as "<path from the repository root>:<the hit's trimmed line>" -> the reason.
UNIX_API_ALLOWED: dict[str, str] = {}


def rust_code(line: str) -> str:
    """The line without its string and char literals and its `//` comment, for counting braces."""
    line = re.sub(r'"(?:\\.|[^"\\])*"', '""', line)
    line = re.sub(r"'(?:\\.|[^'\\])'", "''", line)
    return line.split("//", 1)[0]


def attributes_above(lines: list[str], i: int) -> list[str]:
    """The attribute (and doc-comment) lines directly above line `i`."""
    found = []
    for k in range(i - 1, -1, -1):
        s = lines[k].strip()
        if not (s.startswith("#[") or s.startswith("//")):
            break
        found.append(s)
    return found


def test_module_lines(lines: list[str]) -> set[int]:
    """The lines of every `mod` gated on `cfg(test)`."""
    inside: set[int] = set()
    for i, line in enumerate(lines):
        if not TEST_ATTR.match(line.strip()):
            continue
        j = i + 1
        while j < len(lines) and lines[j].strip().startswith("#["):
            j += 1
        if j >= len(lines) or not MOD_OPEN.match(lines[j]):
            continue
        depth = 0
        for k in range(j, len(lines)):
            code = rust_code(lines[k])
            depth += code.count("{") - code.count("}")
            if depth == 0:
                inside.update(range(i, k + 1))
                break
    return inside


def guarded_by_unix(lines: list[str], i: int) -> bool:
    if any(UNIX_ATTR.search(a) for a in attributes_above(lines, i)):
        return True
    depth = 0
    for k in range(i - 1, -1, -1):
        code = rust_code(lines[k])
        for ch in reversed(code):
            if ch == "}":
                depth += 1
            elif ch == "{":
                if depth:
                    depth -= 1
                elif UNIX_IF.search(code) or any(UNIX_ATTR.search(a)
                                                 for a in attributes_above(lines, k)):
                    return True
    return False


def unguarded_unix_apis(text: str) -> list[int]:
    """1-based lines of `text` that use a unix-only API outside tests with no unix guard."""
    lines = text.splitlines()
    tests = test_module_lines(lines)
    return [i + 1 for i, line in enumerate(lines)
            if i not in tests and UNIX_API.search(rust_code(line)) and not guarded_by_unix(lines, i)]


PORTABILITY_FIXTURE = """\
use std::path::Path;

#[cfg(unix)]
fn make_executable(p: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = p;
}

fn link(a: &Path, b: &Path) -> std::io::Result<()> {
    if cfg!(unix) {
        #[cfg(unix)]
        std::os::unix::fs::symlink(a, b)?;
    }
    Ok(())
}

fn forgot(p: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = p;
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;
}
"""


class Portability(unittest.TestCase):
    """R-8: unix-only APIs stay behind a unix guard, so Windows never meets one first on a hosted runner."""

    def test_unix_only_apis_are_behind_cfg_unix(self) -> None:
        # The rule finds the one unguarded use in the fixture, and only that one.
        self.assertEqual(unguarded_unix_apis(PORTABILITY_FIXTURE), [18])
        found = []
        for path in sorted((ROOT / "api" / "crates").glob("*/src/**/*.rs")):
            text = path.read_text()
            rel = path.relative_to(ROOT).as_posix()
            lines = text.splitlines()
            for n in unguarded_unix_apis(text):
                if f"{rel}:{lines[n - 1].strip()}" not in UNIX_API_ALLOWED:
                    found.append(f"{rel}:{n}: {lines[n - 1].strip()}")
        self.assertEqual(found, [], "unix-only APIs outside #[cfg(unix)]:\n" + "\n".join(found))


class Release(unittest.TestCase):
    """R-9: a release is never cancelled by another, and its image reuses the cooked dependency layer."""

    def test_the_release_has_its_own_concurrency_group_and_a_layer_cache(self) -> None:
        doc = workflow(RELEASE)
        group = doc.get("concurrency") or {}
        self.assertIsInstance(group, dict, "release.yml's concurrency should name a group")
        self.assertTrue(str(group.get("group", "")).startswith("release-"),
                        "release.yml has no concurrency group of its own")
        self.assertIs(group.get("cancel-in-progress"), False, "a release run may be cancelled")
        builds = [s for job in jobs(doc).values() for s in steps_of(job)
                  if uses(s, "docker/build-push-action")]
        self.assertGreaterEqual(len(builds), 2, "release.yml should build the image twice (scan, push)")
        scopes = set()
        for s in builds:
            with_ = s.get("with") or {}
            src, dst = str(with_.get("cache-from", "")), str(with_.get("cache-to", ""))
            self.assertTrue(src.startswith("type=gha"), f"{s.get('name')!r} has no gha cache-from")
            self.assertTrue(dst.startswith("type=gha"), f"{s.get('name')!r} has no gha cache-to")
            self.assertIn("mode=max", dst, f"{s.get('name')!r} does not cache every layer")
            scopes |= set(re.findall(r"scope=([^,\s]+)", src + "," + dst))
        self.assertEqual(len(scopes), 1, f"the image builds should share one cache scope, not {scopes}")


if __name__ == "__main__":
    unittest.main()

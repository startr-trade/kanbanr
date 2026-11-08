"""Unit tests for scripts/check_supply_chain.py (MNT-004). Run by `make ci`, or:

    python3 -m unittest scripts/tests/test_check_supply_chain.py
"""

from __future__ import annotations

import json
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import check_supply_chain as csc  # noqa: E402

ROOT = Path(__file__).resolve().parents[2]
SHA = "a" * 40


def cargo(text: str, name: str = "Makefile") -> list[str]:
    return csc.check_cargo(Path(name), text)


class UnlockedCargoTests(unittest.TestCase):
    def test_an_unlocked_cargo_build_is_reported(self) -> None:
        for line in (
            "\tcd api && cargo build --release",
            "      run: cargo test --workspace",
            "RUN cargo chef cook --release --recipe-path recipe.json",
            "cargo install --path crates/kanbanr-cli",
            "cargo metadata --no-deps",
            "cargo build --release \\\n  -p kanbanr-cli",
        ):
            problems = cargo(line)
            self.assertEqual(len(problems), 1, line)
            self.assertIn("Makefile:1:", problems[0])
            self.assertIn("cargo", problems[0])
        py = csc.check_cargo(Path("scripts/x.py"), '    ["cargo", "metadata", "--no-deps"], cwd=x\n')
        self.assertEqual(len(py), 1)
        self.assertTrue(py[0].startswith("scripts/x.py:1:"))
        # the line number is the first line of a joined command
        self.assertIn("Makefile:2:", cargo("a\ncargo build \\\n  -p x\n")[0])

    def test_locked_and_exempt_commands_pass(self) -> None:
        for line in (
            "cargo build --release --locked -p kanbanr-cli",
            "cargo test --frozen",
            "cargo chef cook --release --locked --recipe-path r.json",
            "cargo build --release \\\n  --locked",
            "cargo fmt --all --check",
            "cargo deny check",
            "cargo audit",
            "cargo clean",
            '\t@echo "  install-cli   cargo install the one binary"',
            'echo "cargo install"',
            "# cargo build in a comment",
            "cargo install --path x  # unlocked-ok: the lockfile is regenerated here",
        ):
            self.assertEqual(cargo(line), [], line)
        self.assertEqual(
            csc.check_cargo(Path("a.py"), '["cargo", "build", "--locked"]\n'), []
        )

    def test_an_exemption_without_a_reason_is_reported(self) -> None:
        problems = cargo("cargo build  # unlocked-ok:")
        self.assertEqual(len(problems), 1)
        self.assertIn("needs a reason", problems[0])


class NpmTests(unittest.TestCase):
    def test_npm_install_is_reported(self) -> None:
        for line in (
            "cd web && npm install && npm run build",
            "RUN npm install",
            "run: npm ci || npm install",
        ):
            self.assertTrue(csc.check_npm(Path("Makefile"), line), line)
        self.assertTrue(csc.check_npm(Path("ci.yml"), "run: npm ci || true"))
        self.assertIn("Dockerfile:2:", csc.check_dockerfile("FROM x\nCOPY web/package.json web/package-lock.json* ./\n")[0])

    def test_npm_install_global_passes(self) -> None:
        for line in (
            "npm install -g @vscode/vsce",
            "npm install --global x",
            "cd web && npm ci && npm run build",
            "# npm install in a comment",
            'echo "npm install"',
        ):
            self.assertEqual(csc.check_npm(Path("Makefile"), line), [], line)
        self.assertEqual(csc.check_dockerfile("COPY web/package.json web/package-lock.json ./\n"), [])


def sbom(**over: object) -> str:
    doc: dict[str, object] = {
        "bomFormat": "CycloneDX",
        "specVersion": "1.5",
        "components": [{"type": "library", "name": "serde"}],
    }
    doc.update(over)
    return json.dumps(doc)


class SbomTests(unittest.TestCase):
    def setUp(self) -> None:
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)

    def write(self, text: str) -> Path:
        path = Path(self.tmp.name) / "x.cdx.json"
        path.write_text(text, encoding="utf-8")
        return path

    def test_a_valid_cyclonedx_file_passes(self) -> None:
        self.assertEqual(csc.check_sbom(self.write(sbom())), [])

    def test_an_empty_or_wrong_format_sbom_is_rejected(self) -> None:
        for text in (
            sbom(components=[]),
            sbom(components=None),
            sbom(bomFormat="SPDX"),
            sbom(specVersion=""),
            "not json",
            "[]",
        ):
            self.assertTrue(csc.check_sbom(self.write(text)), text)
        self.assertTrue(csc.check_sbom(Path(self.tmp.name) / "missing.json"))


RELEASE = f"""\
name: release
on:
  push:
jobs:
  release:
    runs-on: ubuntu-latest
    steps:
      - name: Build the SBOMs
        run: scripts/make-sbom.sh "$GITHUB_REF_NAME" staged
      - name: Attest the release assets
        if: vars.ATTEST_RELEASE != 'false'
        uses: actions/attest-build-provenance@{SHA} # v3.0.0
        with:
          subject-path: staged/*
  image:
    runs-on: ubuntu-latest
    steps:
      - name: Build & push
        id: push
        uses: docker/build-push-action@{SHA} # v6
        with:
          push: true
          sbom: true
          provenance: mode=max
      - name: Attest the image
        if: vars.ATTEST_RELEASE != 'false'
        uses: actions/attest-build-provenance@{SHA} # v3.0.0
"""


class WorkflowTests(unittest.TestCase):
    def without(self, text: str) -> list[str]:
        self.assertIn(text, RELEASE)
        return csc.check_release(RELEASE.replace(text, "", 1))

    def test_the_fixture_passes(self) -> None:
        self.assertEqual(csc.check_release(RELEASE), [])

    def test_release_workflow_builds_and_attaches_sboms(self) -> None:
        problems = self.without('        run: scripts/make-sbom.sh "$GITHUB_REF_NAME" staged\n')
        self.assertTrue(any("make-sbom.sh" in p for p in problems), problems)

    def test_attestation_steps_are_pinned_and_switchable(self) -> None:
        unpinned = csc.check_release(RELEASE.replace(f"@{SHA}", "@v3", 1).replace(
            "docker/build-push-action@v3", "docker/build-push-action@" + SHA))
        self.assertTrue(any("not pinned" in p for p in unpinned), unpinned)
        self.assertTrue(any("ATTEST_RELEASE" in p for p in csc.check_release(
            RELEASE.replace("vars.ATTEST_RELEASE != 'false'", "true"))))
        # a job without the attestation step
        bare = RELEASE.replace("uses: actions/attest-build-provenance", "uses: actions/other")
        self.assertTrue(any("'image' job has no" in p for p in csc.check_release(bare)))
        self.assertTrue(any("'release' job has no" in p for p in csc.check_release(bare)))

    def test_image_push_carries_sbom_and_provenance(self) -> None:
        self.assertTrue(any("sbom: true" in p for p in self.without("          sbom: true\n")))
        self.assertTrue(
            any("provenance: mode=max" in p for p in self.without("          provenance: mode=max\n"))
        )


class RepoTests(unittest.TestCase):
    def test_this_repository_passes(self) -> None:
        self.assertEqual(csc.check(ROOT), [])


if __name__ == "__main__":
    unittest.main()

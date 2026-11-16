"""Checks on the guides and the skill (DOC-007). Run by `make ci`, or:

    python3 -m unittest scripts/tests/test_guides.py

The tests are named for the requirement they prove, so the loader below takes every public method
of the five classes, not only those that start with `test`.
"""

from __future__ import annotations

import re
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
GUIDES = ROOT / "docs/src/guides"
SKILL = ROOT / "skill/kanbanr/SKILL.md"
PAGE = "delivery-pipeline.md"

PROCESSES = ["togaf", "scrum", "agile"]
WITH_CLAUDE = ["togaf", "scrum", "agile", "default", "scheduled", "pdca", "design-control"]
HOSTS = [
    "github", "gitlab", "bitbucket", "ghcr", "docker hub", "gitea", "azure", "jenkins",
    "circleci", "travis", "npm registry", "codeql",
]


def read(path: Path) -> str:
    return path.read_text(encoding="utf-8")


def flat(text: str) -> str:
    """The text with every run of whitespace, line breaks included, as one space."""
    return " ".join(text.split())


def section(text: str, heading: str) -> str:
    """The text under `heading`, up to the next `## ` heading."""
    m = re.search(rf"^{re.escape(heading)}\s*$(.*?)(?=^## |\Z)", text, re.M | re.S)
    return m.group(1) if m else ""


class PipelineTests(unittest.TestCase):
    def every_guide_with_releases_links_the_pipeline_page(self) -> None:
        for process in PROCESSES:
            for kind in ("with-claude", "cli"):
                name = f"{process}-{kind}.md"
                with self.subTest(guide=name):
                    self.assertTrue(PAGE in section(read(GUIDES / name), "## Releasing"))

    def the_pipeline_page_covers_each_topic(self) -> None:
        text = read(GUIDES / PAGE).lower()
        for topic in [
            "start small", "plan allows", "release description", "without a tag",
            "once per commit", "the way a person will", "record ci before you cut",
            "its own job", "rerun", "artifact", "lock file", "saved only from main",
            "timeout", "newer push", "cargo-chef", "sccache",
        ]:
            with self.subTest(topic=topic):
                self.assertTrue(topic in text)
        self.assertRegex(text, r"(?m)^> ", "the page needs an example prompt")

    def every_release_path_records_ci_before_the_cut(self) -> None:
        for process in PROCESSES:
            for kind, marker in (("with-claude", "Record ci passed"), ("cli", "gate record")):
                name = f"{process}-{kind}.md"
                with self.subTest(guide=name):
                    text = section(read(GUIDES / name), "## Releasing")
                    push = text.find("git push origin main")
                    record = text.find(marker)
                    cut = text.find("Cut", record)
                    self.assertTrue(0 <= push < record < cut, f"push {push}, record {record}, cut {cut}")

    def no_guide_prescribes_a_single_job(self) -> None:
        paths = sorted(GUIDES.glob("*.md")) + [SKILL]
        for path in paths:
            text = read(path).lower()
            for phrase in ("one job, one platform", "one job on one platform"):
                with self.subTest(file=path.name, phrase=phrase):
                    self.assertFalse(phrase in text)

    def the_index_and_summary_list_the_pipeline_page(self) -> None:
        self.assertTrue(PAGE in read(GUIDES / "index.md"))
        self.assertTrue(f"guides/{PAGE}" in read(ROOT / "docs/src/SUMMARY.md"))


class WordingTests(unittest.TestCase):
    def no_guide_names_a_host(self) -> None:
        found = []
        for path in sorted(GUIDES.glob("*.md")):
            for n, line in enumerate(read(path).splitlines(), 1):
                for word in HOSTS:
                    if word in line.lower():
                        found.append(f"{path.name}:{n}: {word}")
        self.assertEqual(found, [], "a guide names a host; say 'remote server', 'CI service' or 'registry'")


class ExampleTests(unittest.TestCase):
    def first_item_prompts_describe_a_person(self) -> None:
        paths = [GUIDES / f"{p}-with-claude.md" for p in WITH_CLAUDE]
        paths.append(ROOT / "docs/src/using/with-claude.md")
        for path in paths:
            text = read(path)
            # "What it looked like in practice" is the record of a real walkthrough.
            text = text.split("What it looked like in practice")[0]
            for phrase in ("saved carts", "cart restore", "restore a shopper's cart"):
                with self.subTest(guide=path.name, phrase=phrase):
                    self.assertFalse(phrase in text.lower())

    def every_with_claude_guide_explains_the_interface_question(self) -> None:
        for process in WITH_CLAUDE:
            name = f"{process}-with-claude.md"
            with self.subTest(guide=name):
                self.assertTrue("Claude asks whether it has a user interface" in read(GUIDES / name))


    def every_with_claude_guide_explains_the_style_question(self) -> None:
        for process in WITH_CLAUDE:
            name = f"{process}-with-claude.md"
            with self.subTest(guide=name):
                self.assertTrue("Claude also asks how the screens should look" in read(GUIDES / name))

    def every_guide_index_names_agile_first(self) -> None:
        """DOC-009 R-1: agile is the first row, marked recommended, and the report is linked."""
        text = read(GUIDES / "index.md")
        rows = [l for l in text.splitlines() if l.startswith("|") and not l.startswith("| Process") and not l.startswith("|---")]
        self.assertTrue(rows[0].startswith("| Agile"), rows[0])
        self.assertTrue("recommended" in rows[0])
        self.assertTrue("notes/developer-path-togaf-vs-agile.md" in text)

    def every_release_guide_says_a_patch_starts_as_a_candidate(self) -> None:
        """DOC-009 R-2: each release guide says a patch starts at candidate."""
        for process in PROCESSES:
            for kind in ("with-claude", "cli"):
                name = f"{process}-{kind}.md"
                with self.subTest(guide=name):
                    self.assertTrue("A patch release starts at **candidate**" in read(GUIDES / name))
        processes = read(ROOT / "docs/src/using/processes.md")
        self.assertFalse("the patch starts at the first stage" in processes)


class SkillTests(unittest.TestCase):
    def the_skill_recommends_agile_first(self) -> None:
        """DOC-009 R-1: the setup interview recommends agile and links the report."""
        text = section(read(SKILL), "## Activation: a setup interview in plan mode, then one approved setup")
        self.assertTrue("Recommend **agile**" in text)
        self.assertTrue("developer-path-togaf-vs-agile.md" in text)
        self.assertTrue(0 <= text.index("**agile**") < text.index("**togaf**"))

    def the_skill_asks_about_style_before_a_screen(self) -> None:
        self.assertTrue("Ask how a screen should look before designing it" in read(SKILL))

    def the_skill_asks_the_interface_question(self) -> None:
        self.assertTrue("Ask whether the product has a user interface" in read(SKILL))

    def the_skill_hands_no_write_command_for_a_decision(self) -> None:
        text = read(SKILL)
        self.assertTrue("Never hand the user a kanbanr command that writes" in text)
        self.assertFalse("(or `kanbanr gate record" in text)
        self.assertFalse(re.search(r"adr accept[^\n]*\n?[^\n]*in their own terminal", text))

    def the_skill_keeps_remote_ci_on_the_release(self) -> None:
        self.assertTrue("Remote CI belongs to the release" in read(SKILL))

    def the_skill_proposes_a_staged_pipeline(self) -> None:
        text = read(SKILL)
        start = text.index("**Designing CI or a release workflow.**")
        bullet = text[start:text.index("\n- ", start) if "\n- " in text[start:] else len(text)]
        bullet = bullet[:bullet.index("\n\n")] if "\n\n" in bullet else bullet
        for phrase in ("separate stages", "rerun alone", "lock files"):
            with self.subTest(phrase=phrase):
                self.assertTrue(phrase in bullet)

    def the_skill_designs_a_pipeline_in_the_guides_order(self) -> None:
        text = read(SKILL)
        self.assertTrue("Designing CI or a release workflow" in text)
        self.assertTrue("Designing CI and the release workflow" in text)

    def the_skill_recommends_a_sprint_starting_today(self) -> None:
        """DEF-041 R-3: the sprint's first day is recommended as today, and the guides say so."""
        text = section(read(SKILL), "## Starting the first sprint or release")
        self.assertTrue("--start today" in text)
        self.assertFalse("next Monday" in text)
        for process in PROCESSES:
            with self.subTest(guide=process):
                planning = section(read(GUIDES / f"{process}-with-claude.md"), "## Planning a sprint")
                self.assertTrue("starting today" in planning)


class ProcessTests(unittest.TestCase):
    """DOC-010: the guides state the process their preset defines."""

    def every_guide_states_the_chore_path_its_preset_has(self) -> None:
        """R-1: each guide's chore sentence equals its preset's paths.chore.stages."""
        checked = set()
        for name in WITH_CLAUDE:
            text = read(GUIDES / f"{name}-with-claude.md")
            if "### Chores" not in text:
                continue
            with self.subTest(guide=name):
                chores = flat(text.split("### Chores")[1].split("\n## ")[0])
                m = re.search(r"It goes (.+?), skipping", chores)
                self.assertIsNotNone(m, "the Chores section has no 'It goes ..., skipping' sentence")
                guide = m.group(1).split(" → ")
                preset = read(ROOT / "api/crates/kanbanr-core/presets" / f"{name}.yaml")
                p = re.search(r"(?m)^  chore:\n    stages: \[(.*?)\]", preset)
                self.assertIsNotNone(p, "the preset has no paths.chore.stages")
                stages = [s.strip() for s in p.group(1).split(",")]
                self.assertEqual(guide, stages)
            checked.add(name)
        self.assertTrue({"togaf", "scrum", "agile", "design-control"} <= checked, f"checked only {checked}")

    def the_agile_chore_merges_at_review_after_docs_folded(self) -> None:
        """R-1: the agile chore merges at Review after the docs-folded sign-off."""
        text = read(GUIDES / "agile-with-claude.md")
        chores = flat(text.split("### Chores")[1].split("\n## ")[0])
        self.assertTrue("Review is still where its branch merges into the sprint" in chores)
        self.assertTrue("**Sign off docs-folded**" in chores)
        self.assertFalse("skipping Design and Review" in text)

    def every_sprint_guide_says_a_manual_check_comes_before_docs_folded(self) -> None:
        """R-2: docs-folded waits for a Passed manual check, in a paragraph and in the stage table."""
        for name, row in (("agile", "| **Review**"), ("scrum", "| **Done**")):
            with self.subTest(guide=name):
                text = read(GUIDES / f"{name}-with-claude.md")
                self.assertTrue("**A manual check comes before docs-folded.**" in flat(text))
                lines = [line for line in text.splitlines() if line.startswith(row)]
                self.assertEqual(len(lines), 1, f"expected one table line starting {row!r}")
                self.assertTrue("**Passed**" in lines[0])
                self.assertTrue("**Manual checks**" in lines[0])

    def every_sprint_guide_says_a_granted_delegation_waits_for_the_mode(self) -> None:
        """R-3: a granted delegation does nothing until the mode is delegated."""
        for name in ("agile", "scrum", "togaf"):
            with self.subTest(guide=name):
                text = flat(read(GUIDES / f"{name}-with-claude.md"))
                self.assertTrue("A granted delegation does nothing until you set **Mode** to **delegated**" in text)


def load_tests(loader: unittest.TestLoader, tests: unittest.TestSuite, pattern: str | None) -> unittest.TestSuite:
    suite = unittest.TestSuite()
    for cls in (PipelineTests, WordingTests, ExampleTests, SkillTests, ProcessTests):
        for name in sorted(vars(cls)):
            if not name.startswith("_") and callable(getattr(cls, name)):
                suite.addTest(cls(name))
    return suite


if __name__ == "__main__":
    unittest.main()

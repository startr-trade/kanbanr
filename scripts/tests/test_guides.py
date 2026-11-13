"""Checks on the guides and the skill (DOC-007). Run by `make ci`, or:

    python3 -m unittest scripts/tests/test_guides.py

The tests are named for the requirement they prove, so the loader below takes every public method
of the four classes, not only those that start with `test`.
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
            "once per commit", "the way a person will", "check ci before you cut",
        ]:
            with self.subTest(topic=topic):
                self.assertTrue(topic in text)
        self.assertRegex(text, r"(?m)^> ", "the page needs an example prompt")

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


class SkillTests(unittest.TestCase):
    def the_skill_asks_the_interface_question(self) -> None:
        self.assertTrue("Ask whether the product has a user interface" in read(SKILL))

    def the_skill_designs_a_pipeline_in_the_guides_order(self) -> None:
        text = read(SKILL)
        self.assertTrue("Designing CI or a release workflow" in text)
        self.assertTrue(PAGE in text)

    def the_skill_recommends_a_sprint_starting_today(self) -> None:
        """DEF-041 R-3: the sprint's first day is recommended as today, and the guides say so."""
        text = section(read(SKILL), "## Starting the first sprint or release")
        self.assertTrue("--start today" in text)
        self.assertFalse("next Monday" in text)
        for process in PROCESSES:
            with self.subTest(guide=process):
                planning = section(read(GUIDES / f"{process}-with-claude.md"), "## Planning a sprint")
                self.assertTrue("starting today" in planning)


def load_tests(loader: unittest.TestLoader, tests: unittest.TestSuite, pattern: str | None) -> unittest.TestSuite:
    suite = unittest.TestSuite()
    for cls in (PipelineTests, WordingTests, ExampleTests, SkillTests):
        for name in sorted(vars(cls)):
            if not name.startswith("_") and callable(getattr(cls, name)):
                suite.addTest(cls(name))
    return suite


if __name__ == "__main__":
    unittest.main()

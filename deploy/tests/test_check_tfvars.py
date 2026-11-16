"""Tests for deploy/tests/check_tfvars.py (FTR-030 R-8).

Each finding (a) to (d) is made to fire on a scratch git repository under the
temp dir, which passes until the test breaks it; the last test runs the check
on this repository. Run from the repository root:

    python3 -m unittest -q deploy/tests/test_check_tfvars.py
"""

import os
import subprocess
import sys
import tempfile
import textwrap
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[1]
sys.path.insert(0, str(HERE))

import check_tfvars  # noqa: E402

GITIGNORE = """\
*.sensitive.auto.tfvars
.terraform/
.terraform.lock.hcl
*.tfstate
*.tfstate.*
"""

VARIABLES_TF = """\
variable "namespace" {
  type = string
}

variable "board_credentials" {
  description = "a { brace } and sensitive = true inside a string"
  type        = map(string)
  sensitive   = true
  default     = {}
}

variable "labels" {
  type = map(string)
  validation {
    condition     = true
    error_message = "sensitive = true"
  }
}
"""

TFVARS = """\
namespace = "site"
labels    = { sensitive = "no" }
"""

EXAMPLE = """\
# board_credentials = { board-a = "a real token in a comment is not a value" }
board_credentials = {
  board-a = "<read-only token or deploy key>"
  board-b = ""
}
"""


class CheckTfvarsTest(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.root = Path(self._tmp.name)
        self.git("init", "-q")
        self.write(".gitignore", GITIGNORE)
        self.write("site/variables.tf", VARIABLES_TF)
        self.write("site/terraform.tfvars", TFVARS)
        self.write("site/terraform.sensitive.auto.tfvars.example", EXAMPLE)
        self.git("add", "-A")
        self.assertEqual(check_tfvars.findings(self.root), [], "the scratch repository starts clean")

    def tearDown(self):
        self._tmp.cleanup()

    def git(self, *args):
        env = dict(os.environ, GIT_CONFIG_NOSYSTEM="1", GIT_CONFIG_GLOBAL=os.devnull)
        subprocess.run(["git", *args], cwd=self.root, env=env, check=True, capture_output=True)

    def write(self, rel, text):
        path = self.root / rel
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(textwrap.dedent(text), encoding="utf-8")

    def track(self, rel, text):
        """Write rel and track it even though .gitignore covers it."""
        self.write(rel, text)
        self.git("add", "-f", rel)

    def assertFinding(self, *words):
        found = check_tfvars.findings(self.root)
        hits = [line for line in found if all(w in line for w in words)]
        self.assertTrue(hits, f"no finding naming {words} in {found}")
        return found

    def test_a_tracked_sensitive_tfvars_file_fails(self):
        self.track("site/terraform.sensitive.auto.tfvars", 'board_credentials = { board-a = "t" }\n')
        self.assertFinding("site/terraform.sensitive.auto.tfvars", "is tracked")

    def test_a_tracked_state_file_fails(self):
        self.track("site/terraform.tfstate", "{}\n")
        self.track("site/terraform.tfstate.backup", "{}\n")
        self.track("site/.terraform/providers/x", "x\n")
        found = self.assertFinding("site/terraform.tfstate:", "is tracked")
        self.assertFinding("site/terraform.tfstate.backup", "is tracked")
        self.assertFinding("site/.terraform/providers/x", "is tracked")
        self.assertEqual(len(found), 3, found)

    def test_a_committed_tfvars_setting_a_sensitive_variable_fails(self):
        self.track("site/terraform.tfvars", TFVARS + 'board_credentials = { board-a = "t" }\n')
        self.track("site/extra.auto.tfvars", "board_credentials = {}\n")
        found = self.assertFinding("site/terraform.tfvars", "board_credentials")
        self.assertFinding("site/extra.auto.tfvars", "board_credentials")
        # Only the sensitive variable is named: namespace and labels are not.
        self.assertEqual(len(found), 2, found)
        self.assertEqual(check_tfvars.sensitive_variables([self.root / "site/variables.tf"]), {"board_credentials"})

    def test_the_example_holds_only_placeholders(self):
        self.track(
            "site/terraform.sensitive.auto.tfvars.example",
            EXAMPLE.replace('board-b = ""', 'board-b = "ghp_realtoken"'),
        )
        found = self.assertFinding("site/terraform.sensitive.auto.tfvars.example", "ghp_realtoken")
        self.assertEqual(len(found), 1, found)

    def test_gitignore_covers_secrets_and_state(self):
        self.track(".gitignore", "*.tfstate\n")
        found = check_tfvars.findings(self.root)
        self.assertEqual(
            sorted(found),
            sorted(
                f".gitignore: does not list {p}"
                for p in ("*.sensitive.auto.tfvars", ".terraform/", "*.tfstate.*")
            ),
        )

    def test_the_repository_passes(self):
        self.assertEqual(check_tfvars.findings(REPO), [])
        result = subprocess.run(
            [sys.executable, str(HERE / "check_tfvars.py")], cwd=REPO, capture_output=True, text=True
        )
        self.assertEqual((result.returncode, result.stdout), (0, ""))


if __name__ == "__main__":
    unittest.main()

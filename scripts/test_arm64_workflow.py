"""Regression contract for the trusted-main ARM lane (stdlib only)."""
import pathlib
import re
import subprocess
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[1]


def arm_job(text):
    return text.split("  arm64-verify:\n", 1)[1].split("\n  db-tests:", 1)[0]


class ArmWorkflowTests(unittest.TestCase):
    def setUp(self):
        self.job = arm_job((ROOT / ".github/workflows/ci.yml").read_text())

    def test_only_trusted_main_push(self):
        condition = re.search(r"^    if: (.+)$", self.job, re.M)[1]
        self.assertEqual(condition, "github.event_name == 'push' && github.ref == 'refs/heads/main'")
        expression = condition.replace("&&", "and")
        for event, ref, expected in [
            ("push", "refs/heads/main", True),
            ("push", "refs/heads/feature", False),
            ("push", "refs/tags/main", False),
            ("pull_request", "refs/pull/24/merge", False),
            ("pull_request", "refs/heads/main", False),
            ("workflow_dispatch", "refs/heads/main", False),
        ]:
            with self.subTest(event=event, ref=ref):
                rendered = expression.replace("github.event_name", repr(event)).replace("github.ref", repr(ref))
                self.assertEqual(eval(rendered, {"__builtins__": {}}), expected)

    def test_actions_are_immutable(self):
        actions = re.findall(r"uses: (\S+)", self.job)
        self.assertEqual(len(actions), 2)
        for action in actions:
            self.assertRegex(action, r"^[\w/-]+@[0-9a-f]{40}$")
        self.assertIn("persist-credentials: false", self.job)

    def test_locked_commands_and_scoped_cancellation(self):
        self.assertIn("cargo check --workspace --all-targets --locked", self.job)
        self.assertIn("cargo test --workspace --no-run --locked", self.job)
        self.assertIn("group: arm64-trusted-main-${{ github.repository }}", self.job)
        self.assertIn("cancel-in-progress: true", self.job)

    def test_original_candidate_rejected(self):
        baseline = subprocess.check_output(
            ["git", "show", "d17a7ee7db0896ed10bec10c18ecd48e6b57d8ca:.github/workflows/ci.yml"],
            cwd=ROOT, text=True,
        )
        old = arm_job(baseline)
        self.assertNotIn("github.event_name == 'push' && github.ref == 'refs/heads/main'", old)
        self.assertIn("@stable", old)
        self.assertNotIn("--locked", old)


if __name__ == "__main__":
    unittest.main()

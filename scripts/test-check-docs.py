#!/usr/bin/env python3
"""Regression checks for documentation guardrails using disposable repositories."""
import datetime as dt
import importlib.util
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location("check_docs", Path(__file__).with_name("check-docs.py"))
checker = importlib.util.module_from_spec(spec)
spec.loader.exec_module(checker)
TODAY = dt.date(2026, 9, 28)
HEADER = "<!-- owner: maintainers; reviewed: 2026-09-28 -->\n"


class DocumentationChecks(unittest.TestCase):
    def run_check(self, extra=None):
        files = {name: HEADER + "# Guide\n" for name in checker.ENTRYPOINTS}
        files.update(extra or {})
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for name, content in files.items():
                path = root / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text(content)
            return checker.check(root, set(files), TODAY)

    def test_valid_links_and_code_examples(self):
        errors = self.run_check({
            "docs/index.md": HEADER + "[Guide](guide.md#some-heading)\n```md\n[Example](absent.md)\n```\n",
            "docs/guide.md": HEADER + "## Some `heading`\n[Usage](../README.md#guide)\n",
        })
        self.assertEqual(errors, [])

    def test_broken_links_anchors_and_repository_escape(self):
        errors = self.run_check({"docs/index.md": HEADER + "[Absent](absent.md) [Bad](../README.md#absent) [Outside](../../secret.md)"})
        for phrase in ("missing or ignored target", "missing heading", "escapes repository"):
            self.assertTrue(any(phrase in error for error in errors), errors)

    def test_orphan_and_missing_owner(self):
        errors = self.run_check({"docs/orphan.md": "# Unowned\n"})
        self.assertTrue(any("orphan document" in error for error in errors))
        self.assertTrue(any("owner:" in error for error in errors))

    def test_stale_invalid_and_future_reviews(self):
        for date in ("2026-01-01", "2026-02-30", "2027-01-01"):
            with self.subTest(date=date):
                errors = self.run_check({"README.md": HEADER.replace("2026-09-28", date)})
                self.assertTrue(errors)

    def test_completed_plan_keeps_historical_date(self):
        errors = self.run_check({
            "docs/index.md": HEADER + "[Plan](exec-plans/completed/change.md)",
            "docs/exec-plans/completed/change.md": HEADER.replace("2026-09-28", "2026-01-01"),
        })
        self.assertEqual(errors, [])

    def test_instruction_map_stays_small(self):
        errors = self.run_check({"AGENTS.md": HEADER + "Guidance\n" * 100})
        self.assertTrue(any("exceeds 100 lines" in error for error in errors))


if __name__ == "__main__":
    unittest.main()

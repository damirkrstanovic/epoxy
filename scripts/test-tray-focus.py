#!/usr/bin/env python3
"""Offline focus-helper regressions; never connects to the compositor."""
import importlib.util
import io
import json
from pathlib import Path
import unittest
spec = importlib.util.spec_from_file_location("focus_trace", Path(__file__).with_name("trace-tray-focus.py"))
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)

class FocusTraceTests(unittest.TestCase):
    def run_trace(self, events):
        output = io.StringIO()
        module.trace((json.dumps(event) + "\n" for event in events), output)
        self.assertNotIn("SECRET", output.getvalue())
        return [line.split(" ", 1)[1] for line in output.getvalue().splitlines()]

    def test_focus_transitions_omit_titles_and_raw_metadata(self):
        result = self.run_trace([
            {"WindowsChanged": {"windows": [{"id": 1, "app_id": "terminal", "title": "SECRET", "is_focused": True}, {"id": 2, "app_id": "browser", "title": "SECRET", "is_focused": False}]}},
            {"WindowFocusChanged": {"id": 2}},
            {"WindowFocusChanged": {"id": None}},
            {"WindowOpenedOrChanged": {"window": {"id": 3, "app_id": {"unexpected": "SECRET"}, "is_focused": True}}},
        ])
        self.assertEqual(result, ["window 1: terminal", "window 2: browser", "no focused application window", "window 3: unknown application"])

    def test_late_metadata_identifies_already_focused_window(self):
        result = self.run_trace([
            {"WindowFocusChanged": {"id": 20}},
            {"WindowOpenedOrChanged": {"window": {"id": 20, "app_id": "terminal", "title": "SECRET", "is_focused": False}}},
        ])
        self.assertEqual(result, ["window 20: unknown application", "window 20: terminal"])

if __name__ == "__main__":
    unittest.main()

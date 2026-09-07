"""Synthetic crash fixtures; no real diagnostic reports or user data."""

import contextlib
import copy
import importlib.util
import io
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest


SCRIPT = Path(__file__).resolve().parents[1] / "analyze-macos-crash.py"
FIXTURES = Path(__file__).parent / "fixtures" / "macos-crash"
SPEC = importlib.util.spec_from_file_location("analyze_macos_crash", SCRIPT)
ANALYZER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(ANALYZER)


def fixture(name):
    return ANALYZER.parse_report((FIXTURES / name).read_text())


class CrashAnalysisTests(unittest.TestCase):
    def test_rust_panic_is_distinct_from_foreign_exception(self):
        result = ANALYZER.summarize(*fixture("rust-panic.ips"))
        self.assertEqual(result["status"]["classification"], "rust-panic-at-non-unwind-boundary")
        self.assertEqual(result["version"], "0.5.1")
        self.assertEqual(result["timestamp"], "2026-09-06 16:36:49 +0530")
        self.assertIn("/Downloads/", result["executable"])
        self.assertEqual(result["signing"]["status"], "team-id-absent")
        self.assertNotIn("ad-hoc", json.dumps(result))
        self.assertIn("Not established", result["status"]["jit_cause"])

    def test_weak_reference_abort_does_not_name_an_unproven_view(self):
        result = ANALYZER.summarize(*fixture("weak-reference.ips"))
        self.assertEqual(result["status"]["classification"], "objc-weak-reference-abort")
        self.assertNotIn("NSVisualEffectView", json.dumps(result))
        self.assertEqual(result["signing"]["status"], "team-id-recorded")
        self.assertIn("not verified", result["signing"]["verification"])

    def test_metadata_fallback_and_foreign_exception(self):
        result = ANALYZER.summarize(*fixture("foreign-exception.ips"))
        self.assertEqual(result["status"]["classification"], "foreign-exception-at-rust-boundary")
        self.assertEqual(result["version"], "0.5.0")
        self.assertEqual(result["signing"]["status"], "unknown")

    def test_uses_faulting_worker_and_ignores_main_thread_signature(self):
        result = ANALYZER.summarize(*fixture("worker-abort.ips"))
        self.assertEqual(result["crash_thread"]["index"], 1)
        self.assertFalse(result["crash_thread"]["main_thread_reported"])
        self.assertEqual(result["status"]["classification"], "abort-cause-unknown")
        self.assertNotIn("__rust_foreign_exception", json.dumps(result))

    def test_missing_faulting_thread_does_not_diagnose_idle_stacks(self):
        metadata, report = fixture("worker-abort.ips")
        report.pop("faultingThread")
        result = ANALYZER.summarize(metadata, report)
        self.assertIsNone(result["crash_thread"]["index"])
        self.assertEqual(result["crash_thread"]["frames"], [])
        self.assertEqual(result["status"]["classification"], "abort-cause-unknown")

    def test_output_contains_only_the_allowlisted_fields(self):
        result = ANALYZER.summarize(*fixture("rust-panic.ips"))
        self.assertEqual(set(result), {"timestamp", "version", "executable", "signing", "status", "crash_thread"})
        self.assertNotIn("DO_NOT_EXPORT", json.dumps(result))

    def test_frame_output_is_bounded_but_deep_signature_is_recognized(self):
        metadata, report = fixture("foreign-exception.ips")
        frames = report["threads"][0]["frames"]
        frames[:0] = [{"symbol": "ordinary_frame"}] * (ANALYZER.MAX_FRAMES + 4)
        result = ANALYZER.summarize(metadata, report)
        self.assertEqual(len(result["crash_thread"]["frames"]), ANALYZER.MAX_FRAMES)
        self.assertEqual(result["crash_thread"]["omitted_frames"], 7)
        self.assertEqual(result["status"]["classification"], "foreign-exception-at-rust-boundary")

    def test_absent_or_malformed_optional_fields_are_not_exported(self):
        _, base = fixture("weak-reference.ips")
        for value in (None, [], "bad", True):
            with self.subTest(value=value):
                report = copy.deepcopy(base)
                for field in ("bundleInfo", "exception", "termination", "usedImages"):
                    report[field] = value
                report["codeSigningTeamID"] = {"secret": "DO_NOT_EXPORT"}
                report["threads"][0]["frames"].insert(0, {"imageIndex": -1, "symbol": {"secret": "DO_NOT_EXPORT"}})
                self.assertNotIn("DO_NOT_EXPORT", json.dumps(ANALYZER.summarize({}, report)))

    def test_exception_throw_and_plain_rust_panics_have_separate_statuses(self):
        self.assertEqual(ANALYZER.classify(["objc_exception_throw"], {}, {}), "objective-c-exception")
        self.assertEqual(ANALYZER.classify(["rust_begin_unwind"], {}, {}), "rust-panic")
        self.assertEqual(ANALYZER.classify(["objc_initWeak"], {}, {}), "unknown")

    def test_malformed_input_does_not_echo_report_data(self):
        for body in ("PRIVATE_LOG", '{"secret":"PRIVATE_LOG",}', "{}", "[]", '{} {} {"threads":[]}'):
            with self.subTest(body=body), self.assertRaises(ValueError) as error:
                ANALYZER.parse_report(body)
            self.assertNotIn("PRIVATE_LOG", str(error.exception))

    def test_cli_requires_explicit_report_and_prints_json(self):
        missing = subprocess.run([sys.executable, str(SCRIPT)], capture_output=True, text=True)
        self.assertEqual(missing.returncode, 2)
        run = subprocess.run([sys.executable, str(SCRIPT), str(FIXTURES / "rust-panic.ips")], capture_output=True, text=True)
        self.assertEqual(run.returncode, 0, run.stderr)
        self.assertEqual(json.loads(run.stdout)["version"], "0.5.1")
        self.assertNotIn("DO_NOT_EXPORT", run.stdout + run.stderr)

    def test_cli_errors_are_bounded_and_do_not_echo_file_content(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "invalid.ips"
            for content in (b"PRIVATE_LOG", b"\xffPRIVATE_LOG"):
                path.write_bytes(content)
                error = io.StringIO()
                with contextlib.redirect_stderr(error):
                    self.assertEqual(ANALYZER.main([str(path)]), 2)
                self.assertNotIn("PRIVATE_LOG", error.getvalue())
            with path.open("wb") as report:
                report.truncate(ANALYZER.MAX_REPORT_BYTES + 1)
            error = io.StringIO()
            with contextlib.redirect_stderr(error):
                self.assertEqual(ANALYZER.main([str(path)]), 2)
            self.assertIn("32 MiB", error.getvalue())


if __name__ == "__main__":
    unittest.main()

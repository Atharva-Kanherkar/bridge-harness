#!/usr/bin/env python3
"""Targeted mutation checks. Every source edit is restored, including on failure.

Run from a checkout with the baseline feature tests passing:
  python3 scripts/test-chat-defaults-mutations.py --report /tmp/chat-defaults-mutations.json
This is a finite set of behavioral mutants, not coverage-guided fuzzing or an
exhaustive mutation score. The Rust and TypeScript suites contain seeded fuzzing.
"""
import argparse
import hashlib
import json
import os
import signal
from pathlib import Path
import subprocess
import tempfile
import time

ROOT = Path(__file__).resolve().parents[1]
FRONT = "src/chatDefaults.ts"
CORE = "src-tauri/bridge-core/src/chat_defaults.rs"
MUTANTS = [
    ("explicit-false-is-lost", FRONT, "if (value != null)", "if (value)", "frontend"),
    ("lower-layer-wins", FRONT, "Object.assign(result, { [key]: value });", "if (!(key in result)) Object.assign(result, { [key]: value });", "frontend"),
    ("project-revision-ignored", FRONT, "a.application === b.application && a.project === b.project", "a.application === b.application", "frontend"),
    ("transient-context-exported", FRONT, "return { target: settings.target,", "return { ...settings, target: settings.target,", "frontend"),
    ("host-save-revision-ignored", CORE, "let current = record(&transaction, &key)?;\n    if current.revision != params.expected_revision {", "let current = record(&transaction, &key)?;\n    if false {", "rust"),
    ("host-access-overlay-omitted", CORE, 'field!(access, "access");', '// access overlay removed', "rust"),
    ("scoped-access-becomes-full", CORE, "return Ok(Some(stored.settings.access));", "return Ok(Some(ChatAccess::Full));", "rust"),
    ("unsupported-sandbox-accepted", CORE, "if !descriptor.sandbox_modes.contains(&mode) {", "if false {", "rust"),
]


def command(kind):
    if kind == "frontend":
        return ["bunx", "vitest", "run", "src/chatDefaults.test.ts"]
    return ["cargo", "test", "--manifest-path", "src-tauri/Cargo.toml", "-p", "bridge-core", "chat_defaults::tests", "--locked"]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--report", default="/tmp/chat-defaults-mutations.json")
    parser.add_argument("--only", choices=["frontend", "rust"])
    args = parser.parse_args()
    logs = Path(tempfile.mkdtemp(prefix="chat-defaults-mutations-"))
    report = {"method": "targeted mutation testing", "logs": str(logs), "mutants": []}
    env = {**os.environ, "CARGO_TERM_PROGRESS_WHEN": "never"}
    def interrupted(_signum, _frame):
        raise KeyboardInterrupt("mutation run interrupted; source will be restored")
    signal.signal(signal.SIGTERM, interrupted)
    Path(args.report).write_text(json.dumps(report, indent=2) + "\n")
    try:
        for name, filename, before, after, kind in MUTANTS:
            if args.only and kind != args.only:
                continue
            path = ROOT / filename
            original = path.read_bytes()
            source = original.decode()
            if source.count(before) != 1:
                raise RuntimeError(f"{name}: expected exactly one mutation site, got {source.count(before)}")
            started = time.monotonic()
            result = {"name": name, "file": filename, "command": command(kind)}
            try:
                path.write_text(source.replace(before, after, 1))
                with (logs / f"{name}.log").open("w") as output:
                    run = subprocess.run(command(kind), cwd=ROOT, env=env, stdout=output, stderr=subprocess.STDOUT, timeout=360)
                output = (logs / f"{name}.log").read_text()
                # A compile/runner failure is not evidence that a behavior test killed a mutant.
                assertion_failed = "AssertionError" in output or ("test result: FAILED" in output and "panicked at" in output)
                result.update(exit_code=run.returncode, verdict="killed" if run.returncode != 0 and assertion_failed else "survived" if run.returncode == 0 else "inconclusive")
            except subprocess.TimeoutExpired:
                result.update(verdict="inconclusive", reason="test timeout")
            finally:
                path.write_bytes(original)
                if hashlib.sha256(path.read_bytes()).digest() != hashlib.sha256(original).digest():
                    raise RuntimeError(f"{name}: source restoration failed")
            result["seconds"] = round(time.monotonic() - started, 2)
            report["mutants"].append(result)
            Path(args.report).write_text(json.dumps(report, indent=2) + "\n")
            print(f"{name}: {result['verdict']} ({result['seconds']}s)", flush=True)
    finally:
        Path(args.report).write_text(json.dumps(report, indent=2) + "\n")
    return 0 if report["mutants"] and all(m["verdict"] == "killed" for m in report["mutants"]) else 1


if __name__ == "__main__":
    raise SystemExit(main())

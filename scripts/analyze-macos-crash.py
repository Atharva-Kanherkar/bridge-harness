#!/usr/bin/env python3
"""Summarize one explicitly selected JSON .ips crash report, without uploading it."""

import argparse
import json
from pathlib import Path
import sys


MAX_REPORT_BYTES = 32 * 1024 * 1024
MAX_FRAMES = 20


def parse_report(text):
    """macOS writes a metadata JSON object followed by the crash JSON object."""
    decoder = json.JSONDecoder()
    documents = []
    remaining = text.lstrip("\ufeff \t\r\n")
    try:
        while remaining:
            value, end = decoder.raw_decode(remaining)
            if not isinstance(value, dict) or len(documents) == 2:
                raise ValueError("Expected a JSON crash report with optional metadata")
            documents.append(value)
            remaining = remaining[end:].lstrip()
    except (json.JSONDecodeError, RecursionError) as error:
        # Do not echo report fragments or diagnostic messages from the file.
        raise ValueError("Invalid JSON crash report") from error
    if not documents or not isinstance(documents[-1].get("threads"), list):
        raise ValueError("Unsupported report: expected a JSON .ips file with threads")
    return (documents[0] if len(documents) == 2 else {}), documents[-1]


def scalar(value):
    """Only bounded strings and integers can leave the report allowlist."""
    if isinstance(value, str):
        return value[:512]
    if type(value) is int:
        return value
    return None


def object_field(report, key):
    value = report.get(key)
    return value if isinstance(value, dict) else {}


def crash_thread(report):
    threads = report["threads"]
    triggered = [
        (index, thread)
        for index, thread in enumerate(threads)
        if isinstance(thread, dict) and thread.get("triggered") is True
    ]
    if len(triggered) == 1:
        return triggered[0]
    index = report.get("faultingThread")
    if type(index) is int and 0 <= index < len(threads):
        if isinstance(threads[index], dict):
            return index, threads[index]
    # Never diagnose from an idle main/worker stack when the crash thread is absent.
    return None, {}


def classify(symbols, exception, termination):
    stack = "\n".join(symbols)
    if "_objc_fatal" in stack and any(
        marker in stack for marker in ("weak_register_no_lock", "objc_initWeak")
    ):
        return "objc-weak-reference-abort"
    if "__rust_foreign_exception" in stack:
        return "foreign-exception-at-rust-boundary"
    if "panic_cannot_unwind" in stack or "panic_nounwind" in stack:
        return "rust-panic-at-non-unwind-boundary"
    if "objc_exception_throw" in stack:
        return "objective-c-exception"
    if any(marker in stack for marker in ("rust_begin_unwind", "rust_panic", "::panicking::")):
        return "rust-panic"
    if exception.get("signal") == "SIGABRT" or (
        termination.get("namespace") == "SIGNAL" and termination.get("code") == 6
    ):
        return "abort-cause-unknown"
    return "unknown"


def summarize(metadata, report):
    bundle = object_field(report, "bundleInfo")
    exception = object_field(report, "exception")
    termination = object_field(report, "termination")
    index, thread = crash_thread(report)
    raw_frames = thread.get("frames", [])
    frames = [frame for frame in raw_frames if isinstance(frame, dict)] if isinstance(raw_frames, list) else []
    symbols = [frame["symbol"] for frame in frames if isinstance(frame.get("symbol"), str)]
    images = report.get("usedImages", [])
    images = images if isinstance(images, list) else []
    selected_frames = []
    for frame in frames[:MAX_FRAMES]:
        image_index = frame.get("imageIndex")
        image = {}
        if type(image_index) is int and 0 <= image_index < len(images):
            image = images[image_index] if isinstance(images[image_index], dict) else {}
        name = image.get("name")
        selected_frames.append({
            "image": scalar(Path(name).name) if isinstance(name, str) else None,
            "symbol": scalar(frame.get("symbol")),
            "image_offset": scalar(frame.get("imageOffset")),
        })
    team = scalar(report.get("codeSigningTeamID"))
    return {
        "timestamp": scalar(report.get("captureTime")) or scalar(metadata.get("timestamp")),
        "version": scalar(bundle.get("CFBundleShortVersionString")) or scalar(metadata.get("app_version")),
        "executable": scalar(report.get("procPath")),
        "signing": {
            "identifier": scalar(report.get("codeSigningID")),
            "team_id": team,
            "status": "team-id-absent" if team == "" else "team-id-recorded" if team else "unknown",
            "verification": "Report metadata only; signature and notarization not verified",
        },
        "status": {
            "exception_type": scalar(exception.get("type")),
            "signal": scalar(exception.get("signal")),
            "termination_namespace": scalar(termination.get("namespace")),
            "termination_code": scalar(termination.get("code")),
            "classification": classify(symbols, exception, termination),
            "jit_cause": "Not established by crash frames or SIGABRT alone",
        },
        "crash_thread": {
            "index": index,
            "main_thread_reported": thread.get("queue") == "com.apple.main-thread" or thread.get("name") == "main",
            "frames": selected_frames,
            "omitted_frames": max(0, len(frames) - MAX_FRAMES),
        },
    }


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("report", type=Path, help="Path to one JSON macOS .ips crash report")
    args = parser.parse_args(argv)
    try:
        with args.report.open("rb") as source:
            raw = source.read(MAX_REPORT_BYTES + 1)
        if len(raw) > MAX_REPORT_BYTES:
            raise ValueError("Report exceeds the 32 MiB input limit")
        metadata, report = parse_report(raw.decode("utf-8-sig"))
        result = summarize(metadata, report)
    except OSError as error:
        print("Cannot read report ({})".format(type(error).__name__), file=sys.stderr)
        return 2
    except (UnicodeError, ValueError) as error:
        message = "Report is not UTF-8" if isinstance(error, UnicodeError) else str(error)
        print(message, file=sys.stderr)
        return 2
    # JSON escaping prevents control characters in report fields reaching the terminal.
    print(json.dumps(result, indent=2, ensure_ascii=True))
    return 0


if __name__ == "__main__":
    sys.exit(main())

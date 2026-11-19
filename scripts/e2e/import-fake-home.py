#!/usr/bin/env python3
"""Build the fake user's home for scripts/e2e/import-fake-user.mjs.

    python3 -I scripts/e2e/import-fake-home.py <home-dir>

It holds a real Claude Code 2.1.325 session (the sanitized fixture, with an
Explore subagent), a billing chat with a pasted screenshot, thinking, a search,
a failing command, a compaction and a /rename title, three small docs chats,
600 short chats in three more projects, one background run in the OS temp
folder, one Claude memory and a CLAUDE.md, plus traces of Codex, Conductor and
T3 Code so the "Found on this Mac" list shows what is coming.
"""
import base64, json, os, shutil, sys, uuid

home = os.path.abspath(sys.argv[1])
repo = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
shutil.rmtree(home, ignore_errors=True)
os.makedirs(os.path.join(home, ".claude"))
shutil.copytree(os.path.join(repo, "testing/fixtures/import/claude/v2/demo/projects"), os.path.join(home, ".claude/projects"))

def write(rel, records):
    path = os.path.join(home, rel)
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w") as out:
        for record in records:
            out.write(json.dumps(record) + "\n")

png = base64.b64encode(open(os.path.join(repo, "assets/bridge-icon.png"), "rb").read()).decode()
sid, cwd = "7b1f0c2e-3a4d-4e5f-8a9b-0c1d2e3f4a5b", "/work/billing-service"
def user(i, at, content, **extra): return {"type": "user", "uuid": f"u{i}", "sessionId": sid, "timestamp": at, "cwd": cwd, "message": {"role": "user", "content": content}, **extra}
def assistant(i, at, blocks): return {"type": "assistant", "uuid": f"a{i}", "sessionId": sid, "timestamp": at, "cwd": cwd, "message": {"role": "assistant", "model": "claude-opus-5-5", "content": blocks}}
write(f".claude/projects/-work-billing-service/{sid}.jsonl", [
    {"type": "queue-operation", "operation": "enqueue", "timestamp": "2026-11-12T09:00:00Z", "sessionId": sid},
    user(1, "2026-11-12T09:00:01Z", [{"type": "image", "source": {"type": "base64", "media_type": "image/png", "data": png}}, {"type": "text", "text": "The invoice email looks wrong, see the screenshot. Can you find where the header logo is rendered?"}]),
    assistant(1, "2026-11-12T09:00:05Z", [{"type": "thinking", "thinking": "The header is probably rendered by the email template. I should grep for the logo asset before guessing.", "signature": "sig"}]),
    assistant(2, "2026-11-12T09:00:06Z", [{"type": "tool_use", "id": "t1", "name": "Grep", "input": {"pattern": "logo", "path": "templates"}}]),
    user(2, "2026-11-12T09:00:07Z", [{"type": "tool_result", "tool_use_id": "t1", "content": "templates/invoice.html:12: <img src=\"{{ logo_url }}\" width=\"240\">"}]),
    assistant(3, "2026-11-12T09:00:09Z", [{"type": "tool_use", "id": "t2", "name": "Bash", "input": {"command": "npm run render-email -- --preview"}}]),
    user(3, "2026-11-12T09:00:15Z", [{"type": "tool_result", "tool_use_id": "t2", "content": "Error: logo_url is undefined in preview context", "is_error": True}]),
    assistant(4, "2026-11-12T09:00:20Z", [{"type": "text", "text": "The template sets a fixed width of 240px and the preview context never passes `logo_url`, so the image collapses. I added `logo_url` to the preview context and switched the width to `max-width: 240px`."}]),
    {"type": "system", "subtype": "compact_boundary", "uuid": "c1", "timestamp": "2026-11-12T10:30:00Z", "content": "Conversation compacted", "compactMetadata": {"trigger": "auto", "preTokens": 171000}},
    {"type": "user", "uuid": "s1", "timestamp": "2026-11-12T10:30:01Z", "isCompactSummary": True, "isVisibleInTranscriptOnly": True, "message": {"role": "user", "content": "Summary: fixed the invoice email header. The preview context now passes logo_url and the template uses max-width. Remaining: add a snapshot test."}},
    user(4, "2026-11-12T10:31:00Z", "Add a snapshot test for the email header."),
    assistant(5, "2026-11-12T10:31:30Z", [{"type": "text", "text": "Added `test/email-header.test.js` with a snapshot of the rendered header. 14 tests pass."}]),
    {"type": "custom-title", "customTitle": "Invoice email header fix", "sessionId": sid},
])
for n, ask in enumerate(["Fix the broken sidebar links", "Write the getting started page", "Speed up the docs build"], start=1):
    s = str(uuid.UUID(int=n))
    write(f".claude/projects/-work-docs-site/{s}.jsonl", [
        {"type": "user", "uuid": f"x{n}", "sessionId": s, "timestamp": f"2026-11-0{n}T08:00:00Z", "cwd": "/work/docs-site", "message": {"role": "user", "content": ask}},
        {"type": "assistant", "uuid": f"y{n}", "sessionId": s, "timestamp": f"2026-11-0{n}T08:01:00Z", "cwd": "/work/docs-site", "message": {"role": "assistant", "content": [{"type": "text", "text": "Done."}]}},
    ])
# Enough chats that the import runs for a couple of dozen batches, so its
# progress is on screen long enough to watch. IMPORT_FAKE_CHATS sets how many.
for n in range(int(os.environ.get("IMPORT_FAKE_CHATS", "600"))):
    s, project = str(uuid.UUID(int=1000 + n)), ["api-gateway", "mobile-app", "data-pipeline"][n % 3]
    turns = []
    for t in range(3):
        turns.append({"type": "user", "uuid": f"g{n}-u{t}", "sessionId": s, "timestamp": f"2026-10-{(n % 28) + 1:02d}T09:{t:02d}:00Z", "cwd": f"/work/{project}", "message": {"role": "user", "content": f"Task {n}.{t}: tidy the {project} handlers"}})
        turns.append({"type": "assistant", "uuid": f"g{n}-a{t}", "sessionId": s, "timestamp": f"2026-10-{(n % 28) + 1:02d}T09:{t:02d}:30Z", "cwd": f"/work/{project}", "message": {"role": "assistant", "content": [{"type": "text", "text": "Done, the handlers are tidier. " * 20}]}})
    write(f".claude/projects/-work-{project}/{s}.jsonl", turns)
# A run Claude Code made in the OS temp folder: a background run, left out.
probe = "9d2e4c1a-0b3f-4a5e-8c7d-6e5f4a3b2c1d"
write(f".claude/projects/-private-var-folders-h7-T-probe/{probe}.jsonl", [
    {"type": "user", "uuid": "p1", "sessionId": probe, "timestamp": "2026-11-15T07:00:00Z", "cwd": "/private/var/folders/h7/T/probe", "message": {"role": "user", "content": "ping"}},
])
os.makedirs(os.path.join(home, ".claude/projects/-work-billing-service/memory"))
open(os.path.join(home, ".claude/projects/-work-billing-service/memory/MEMORY.md"), "w").write("- The billing service uses pnpm, not npm.\n- Email templates live in templates/.\n")
open(os.path.join(home, ".claude/CLAUDE.md"), "w").write("Prefer small commits. Run tests before saying done.\n")
for folder in [".codex/sessions/2026/11/17", "conductor/workspaces/billing-service/tokyo", "conductor/workspaces/billing-service/lima", "conductor/workspaces/docs-site/paris", ".t3/userdata"]:
    os.makedirs(os.path.join(home, folder), exist_ok=True)
open(os.path.join(home, ".codex/sessions/2026/11/17/rollout-2026-11-17T10-00-00-abc.jsonl"), "w").write("{}\n")
print(home)

#!/usr/bin/env python3
"""Build the fake user's home for scripts/e2e/import-fake-user.mjs.

    python3 -I scripts/e2e/import-fake-home.py <home-dir> [<bridge-data-dir>]

It holds a real Claude Code 2.1.325 session (the sanitized fixture, with an
Explore subagent), a billing chat with a pasted screenshot, thinking, a search,
a failing command, a compaction and a /rename title, three small docs chats,
600 short chats in three more projects, one background run in the OS temp
folder, one Claude memory and a CLAUDE.md, one Codex chat, an OpenCode store
with a chat and its subagent, Conductor's workspaces, T3 Code's threads and
worktree, Orca's worktree and multi-agent run, and Superset's workspaces,
chats and agent configuration, so the "Found on this Mac" list shows every
tool.
"""
import base64, json, os, shutil, sqlite3, subprocess, sys, uuid

home = os.path.abspath(sys.argv[1])
# The data folder the Bridge under test uses: a Slack inbox run Bridge made
# there is in Claude Code's history, and must not come back as a chat.
bridge_data = os.path.abspath(sys.argv[2]) if len(sys.argv) > 2 else None
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
# A second Claude account's folder, as added in Settings (the fake user adds
# it as `~/.claude-work`): one chat that ~/.claude does not have.
work = "5c3a2b1d-4e5f-4a6b-8c7d-9e0f1a2b3c4d"
write(f".claude-work/projects/-work-billing-service/{work}.jsonl", [
    {"type": "user", "uuid": "w1", "sessionId": work, "timestamp": "2026-11-19T12:00:00Z", "cwd": "/work/billing-service", "message": {"role": "user", "content": "Rotate the Stripe keys before Friday's release."}},
    {"type": "assistant", "uuid": "w2", "sessionId": work, "timestamp": "2026-11-19T12:00:30Z", "cwd": "/work/billing-service", "message": {"role": "assistant", "content": [{"type": "text", "text": "Rotated both keys and updated the deploy secrets."}]}},
    {"type": "custom-title", "customTitle": "Rotate the Stripe keys", "sessionId": work},
])
# A run Claude Code made in the OS temp folder: a background run, left out.
probe = "9d2e4c1a-0b3f-4a5e-8c7d-6e5f4a3b2c1d"
write(f".claude/projects/-private-var-folders-h7-T-probe/{probe}.jsonl", [
    {"type": "user", "uuid": "p1", "sessionId": probe, "timestamp": "2026-11-15T07:00:00Z", "cwd": "/private/var/folders/h7/T/probe", "message": {"role": "user", "content": "ping"}},
])
os.makedirs(os.path.join(home, ".claude/projects/-work-billing-service/memory"))
open(os.path.join(home, ".claude/projects/-work-billing-service/memory/MEMORY.md"), "w").write("- The billing service uses pnpm, not npm.\n- Email templates live in templates/.\n")
open(os.path.join(home, ".claude/CLAUDE.md"), "w").write("Prefer small commits. Run tests before saying done.\n")
for folder in [".codex/sessions/2026/11/17"]:
    os.makedirs(os.path.join(home, folder), exist_ok=True)
# One Codex chat, shaped like a real rollout: what Codex showed is in
# item_completed events.
def codex_line(kind, payload): return json.dumps({"timestamp": "2026-11-17T10:00:00Z", "ordinal": 0, "type": kind, "payload": payload})
def codex_item(item): return codex_line("event_msg", {"type": "item_completed", "item": item})
with open(os.path.join(home, ".codex/sessions/2026/11/17/rollout-2026-11-17T10-00-00-codex-thread-1.jsonl"), "w") as out:
    out.write("\n".join([
        codex_line("session_meta", {"id": "codex-thread-1", "cwd": "/work/billing-service", "timestamp": "2026-11-17T10:00:00Z", "originator": "codex_exec", "source": "exec"}),
        codex_line("turn_context", {"model": "gpt-6.1", "effort": "medium"}),
        codex_item({"type": "UserMessage", "id": "cu1", "content": [{"type": "text", "text": "Why is the refund job slow?", "text_elements": []}]}),
        codex_item({"type": "CommandExecution", "id": "cc1", "command": ["/bin/zsh", "-lc", "rg -n refund jobs/"], "cwd": "/work/billing-service", "aggregated_output": "jobs/refund.py:40: for row in rows:", "exit_code": 0, "status": "completed"}),
        codex_item({"type": "AgentMessage", "id": "ca1", "content": [{"type": "text", "text": "It loads every row before filtering. Filtering in SQL makes it 30x faster."}]}),
    ]) + "\n")
with open(os.path.join(home, ".codex/session_index.jsonl"), "w") as out:
    out.write(json.dumps({"id": "codex-thread-1", "thread_name": "Speed up the refund job", "updated_at": "2026-11-17T10:05:00Z"}) + "\n")
# An OpenCode store, with the columns of OpenCode 2.1's own schema: a chat
# with a command and an edit, its subagent, and a background run in the temp
# folder. Its credential table holds a canary nothing may ever show.
store = os.path.join(home, ".local/share/opencode/opencode.db")
os.makedirs(os.path.dirname(store), exist_ok=True)
db = sqlite3.connect(store)
db.executescript("""
CREATE TABLE credential (id text PRIMARY KEY, integration_id text, label text NOT NULL, value text NOT NULL, connector_id text, method_id text, active integer, time_created integer NOT NULL, time_updated integer NOT NULL);
CREATE TABLE session_v2 (id text PRIMARY KEY, project_id text NOT NULL, workspace_id text, parent_id text, fork_session_id text, fork_boundary text, slug text NOT NULL, directory text NOT NULL, path text, title text, version text NOT NULL, share_url text, metadata text, agent text, model text, time_created integer NOT NULL, time_updated integer NOT NULL, time_compacting integer, time_archived integer);
CREATE TABLE session_message (id text PRIMARY KEY, session_id text NOT NULL, type text NOT NULL, seq integer NOT NULL, time_created integer NOT NULL, time_updated integer NOT NULL, data text NOT NULL);
""")
db.execute("INSERT INTO credential VALUES('c1',NULL,'key','CANARY-opencode-secret',NULL,NULL,1,1,1)")
def oc_session(sid, parent, directory, title, at):
    db.execute("INSERT INTO session_v2(id,project_id,parent_id,slug,directory,title,version,time_created,time_updated) VALUES(?,?,?,?,?,?,?,?,?)", (sid, "p", parent, sid, directory, title, "2.1.1", at, at))
def oc_row(sid, seq, kind, data):
    db.execute("INSERT INTO session_message VALUES(?,?,?,?,?,?,?)", (f"{sid}-{seq}", sid, kind, seq, 1_795_000_000_000 + seq, 1_795_000_000_000 + seq, json.dumps(data)))
def oc_tool(call, name, inp, out, meta): return {"type": "tool", "id": call, "name": name, "state": {"status": "completed", "input": inp, "content": [{"type": "text", "text": out}], "metadata": meta}}
oc_session("ses_checkout", None, "/work/storefront", "Speed up the checkout page", 1_795_000_000_000)
oc_row("ses_checkout", 1, "user", {"time": {}, "text": "The checkout page takes four seconds to load. Why?"})
oc_row("ses_checkout", 2, "assistant", {"time": {}, "agent": "build", "model": {"id": "big-pickle", "providerID": "opencode", "variant": "high"}, "content": [
    {"type": "reasoning", "text": "Probably the cart query. Let me measure before changing anything."},
    oc_tool("call_1", "shell", {"command": "npm run profile -- checkout"}, "cart.load 3.6s (N+1: 48 queries)", {"exit": 0}),
    oc_tool("call_2", "task", {"description": "Find every cart query", "prompt": "List the cart queries", "subagent_type": "explore"}, "Found 3 call sites.", {"sessionId": "ses_explore"}),
    oc_tool("call_3", "edit", {"filePath": "/work/storefront/src/cart.ts", "oldString": "for", "newString": "join"}, "Edited", {"diff": "--- a/src/cart.ts\n+++ b/src/cart.ts\n-for (const item of items) await load(item)\n+await loadAll(items)\n"}),
    {"type": "text", "text": "The cart loaded each item with its own query. One joined query brings the page to 0.4 s."},
]})
oc_session("ses_explore", "ses_checkout", "/work/storefront", "Find every cart query (@explore subagent)", 1_795_000_000_100)
oc_row("ses_explore", 1, "user", {"time": {}, "text": "List the cart queries"})
oc_row("ses_explore", 2, "assistant", {"time": {}, "model": {"id": "big-pickle", "variant": "default"}, "content": [
    oc_tool("call_9", "grep", {"pattern": "cart.load", "path": "/work/storefront"}, "src/cart.ts:12\nsrc/checkout.ts:40\nsrc/api.ts:8", {}),
    {"type": "text", "text": "Found 3 call sites: cart.ts, checkout.ts and api.ts."},
]})
oc_session("ses_probe", None, "/private/var/folders/h7/T/probe", "probe", 1_795_000_000_200)
oc_row("ses_probe", 1, "user", {"time": {}, "text": "ping"})
db.commit()
db.close()
# Conductor, as it lays things out (0.97.0): a repository with three
# workspaces as real git worktrees under ~/conductor/workspaces/storefront/,
# and its database built from the real schema (rebuilt from its own
# rollbacks): "Fix the cart total" is active, "Old spike" is archived in
# Conductor, and lima sits on a detached HEAD. A Claude Code chat ran in the
# active one. Its env_vars table holds a canary nothing may ever show.
def git(cwd, *args):
    env = dict(os.environ, GIT_AUTHOR_NAME="Fake", GIT_AUTHOR_EMAIL="fake@example.com", GIT_COMMITTER_NAME="Fake", GIT_COMMITTER_EMAIL="fake@example.com", HOME=home)
    subprocess.run(["git", *args], cwd=cwd, env=env, check=True, capture_output=True)
store_repo = os.path.join(home, "code/storefront")
os.makedirs(store_repo)
git(store_repo, "init", "-q", "-b", "main")
with open(os.path.join(store_repo, "cart.js"), "w") as out:
    out.write("export const total = lines => lines.reduce((sum, line) => sum + line.price, 0);\n")
git(store_repo, "add", "-A")
git(store_repo, "commit", "-q", "-m", "first")
workspaces = os.path.join(home, "conductor/workspaces/storefront")
os.makedirs(workspaces)
for directory, branch in [("tokyo", "fix-cart-total"), ("oslo", "old-spike")]:
    git(store_repo, "worktree", "add", "-q", "-b", branch, os.path.join(workspaces, directory))
git(store_repo, "worktree", "add", "-q", "--detach", os.path.join(workspaces, "lima"))
with open(os.path.join(workspaces, "tokyo/cart.js"), "a") as out:
    out.write("// uncommitted: multiply by qty\n")
tokyo = os.path.realpath(os.path.join(workspaces, "tokyo"))
conductor_db = os.path.join(home, "Library/Application Support/com.conductor.app/conductor.db")
os.makedirs(os.path.dirname(conductor_db))
fixtures = os.path.join(repo, "testing/fixtures/import/conductor")
db = sqlite3.connect(conductor_db)
db.executescript(open(os.path.join(fixtures, "schema-181.sql")).read())
for line in open(os.path.join(fixtures, "migrations-181.txt")):
    number, description = line.rstrip("\n").split("|", 1)
    db.execute("INSERT INTO _sqlx_migrations(version,description,installed_on,success,checksum,execution_time) VALUES(?,?,'2026-11-20',1,x'00',1)", (int(number), description))
db.execute("INSERT INTO env_vars(id,key,value,scope) VALUES('e','STRIPE_KEY','CANARY-conductor-secret','user')")
db.execute("INSERT INTO repos(id,name,root_path,storage_version,setup_script,run_script,run_script_mode) VALUES('r','storefront',?,3,'pnpm install','pnpm dev','concurrent')", (os.path.realpath(store_repo),))
db.execute("INSERT INTO projects(id,name) VALUES('p','storefront')")
db.execute("INSERT INTO project_repositories(id,project_id,repository_id,position) VALUES('pr','p','r',0)")
for key, directory, name, branch, state in [("w1", "tokyo", "Fix the cart total", "fix-cart-total", "active"), ("w2", "oslo", "Old spike", "old-spike", "archived"), ("w3", "lima", None, None, "active")]:
    db.execute("INSERT INTO workspaces(local_id,project_id,directory_name,workspace_name,branch,state) VALUES(?,?,?,?,?,?)", (key, "p", directory, name, branch, state))
db.execute("INSERT INTO sessions(id,workspace_id,agent_type,claude_session_id,title) VALUES('s1','w1','claude','6f1e2d3c-4b5a-4c6d-8e7f-0a1b2c3d4e5f','CANARY-conductor-secret')")
db.commit()
db.close()
conductor_chat = "6f1e2d3c-4b5a-4c6d-8e7f-0a1b2c3d4e5f"
slug = "".join(c if c.isalnum() else "-" for c in tokyo)
write(f".claude/projects/{slug}/{conductor_chat}.jsonl", [
    {"type": "user", "uuid": "c1", "sessionId": conductor_chat, "timestamp": "2026-11-19T15:00:00Z", "cwd": tokyo, "message": {"role": "user", "content": "The cart total ignores quantities. Fix it."}},
    {"type": "assistant", "uuid": "c2", "sessionId": conductor_chat, "timestamp": "2026-11-19T15:00:40Z", "cwd": tokyo, "message": {"role": "assistant", "content": [{"type": "text", "text": "The total now multiplies each price by its quantity."}]}},
    {"type": "custom-title", "customTitle": "Cart total ignores quantities", "sessionId": conductor_chat},
])
# T3 Code, as its current build lays things out (migration 102): a project
# with a thread in a real git worktree under ~/.t3/worktrees/<repo>/, whose
# Claude Code session is still on disk (T3 Code titles it in the transcript);
# a thread whose session Claude Code already deleted, so only T3 Code's own
# copy is left; and a deleted thread. Its auth table holds a canary.
pricing = os.path.join(home, "code/pricing")
os.makedirs(pricing)
git(pricing, "init", "-q", "-b", "main")
with open(os.path.join(pricing, "price.js"), "w") as out:
    out.write("export const price = n => n * 1.2;\n")
git(pricing, "add", "-A")
git(pricing, "commit", "-q", "-m", "first")
t3_tree = os.path.join(home, ".t3/worktrees/pricing/feat-discount-07db51fef00c511d")
git(pricing, "worktree", "add", "-q", "-b", "feat/discount", t3_tree)
with open(os.path.join(t3_tree, "price.js"), "a") as out:
    out.write("// uncommitted: export const discount = (n, pct) => n * (1 - pct / 100);\n")
t3_tree = os.path.realpath(t3_tree)
kept, gone = "1da2ef59-9231-40f6-8da5-ba7f64dc5e6d", "43440686-a49e-487a-9544-a94e260774a6"
write(f".claude/projects/{''.join(c if c.isalnum() else '-' for c in t3_tree)}/{kept}.jsonl", [
    {"type": "custom-title", "customTitle": "Add a discount helper", "sessionId": kept},
    {"type": "user", "uuid": "t1", "sessionId": kept, "timestamp": "2026-11-20T17:18:31Z", "cwd": t3_tree, "entrypoint": "t3-code", "message": {"role": "user", "content": "Add a function discount(n, pct) to price.js. Keep it tiny."}},
    {"type": "assistant", "uuid": "t2", "sessionId": kept, "timestamp": "2026-11-20T17:18:40Z", "cwd": t3_tree, "entrypoint": "t3-code", "message": {"role": "assistant", "content": [{"type": "text", "text": "Added discount(n, pct): it returns n * (1 - pct / 100)."}]}},
])
t3_db = os.path.join(home, ".t3/userdata/statev2.sqlite")
os.makedirs(os.path.dirname(t3_db), exist_ok=True)
t3_fixtures = os.path.join(repo, "testing/fixtures/import/t3code")
db = sqlite3.connect(t3_db)
db.executescript(open(os.path.join(t3_fixtures, "schema-102.sql")).read())
for line in open(os.path.join(t3_fixtures, "migrations-102.txt")):
    number, name = line.rstrip("\n").split("\t", 1)
    db.execute("INSERT INTO effect_sql_migrations(migration_id,name) VALUES(?,?)", (int(number), name))
db.execute("INSERT INTO auth_sessions(session_id,subject,scopes,method,issued_at,expires_at) VALUES('CANARY-t3code-secret','owner','[]','bootstrap','2026-11-20','2027-11-20')")
db.execute("INSERT INTO projection_projects(project_id,title,workspace_root,scripts_json,created_at,updated_at) VALUES('p','pricing',?,?,'2026-11-20T17:00:00Z','2026-11-20T17:00:00Z')",
           (os.path.realpath(pricing), json.dumps([{"id": "i", "name": "Install", "command": "bun install", "icon": "configure", "runOnWorktreeCreate": True}])))
def t3_thread(tid, title, native, worktree, branch, deleted=False):
    provider_thread = f"provider-thread:provider:claudeAgent:native-thread:{native}"
    payload = {"id": tid, "projectId": "p", "title": title, "providerInstanceId": "claudeAgent", "modelSelection": {"instanceId": "claudeAgent", "model": "claude-haiku-4-5"},
               "branch": branch, "worktreePath": worktree, "pullRequests": [], "activeProviderThreadId": provider_thread,
               "lineage": {"parentThreadId": None, "relationshipToParent": None, "rootThreadId": tid}, "historyOrigin": "native", "deletedAt": "2026-11-20T18:00:00Z" if deleted else None}
    db.execute("INSERT INTO orchestration_v2_projection_threads(thread_id,project_id,title,default_provider,runtime_mode,interaction_mode,active_provider_thread_id,created_at,updated_at,deleted_at,payload_json,provider_instance_id) VALUES(?,?,?,?,?,?,?,?,?,?,?,?)",
               (tid, "p", title, "claudeAgent", "full-access", "default", provider_thread, "2026-11-20T17:18:29Z", "2026-11-20T17:18:40Z", "2026-11-20T18:00:00Z" if deleted else None, json.dumps(payload), "claudeAgent"))
    db.execute("INSERT INTO orchestration_v2_projection_provider_threads(provider_thread_id,thread_id,provider,driver,provider_session_id,status,updated_at,payload_json) VALUES(?,?,?,?,?,?,?,?)",
               (provider_thread, tid, "claudeAgent", "claudeAgent", "provider-session:provider-instance:claudeAgent:shared", "idle", "2026-11-20T17:18:40Z",
                json.dumps({"id": provider_thread, "driver": "claudeAgent", "nativeThreadRef": {"driver": "claudeAgent", "nativeId": native, "strength": "strong"}})))
def t3_message(tid, mid, role, text, at):
    db.execute("INSERT INTO orchestration_v2_projection_messages(message_id,thread_id,role,streaming,created_at,updated_at,payload_json) VALUES(?,?,?,0,?,?,?)",
               (mid, tid, role, at, at, json.dumps({"id": mid, "role": role, "text": text})))
t3_thread("mcp:f1cf9272", "Add a discount helper", kept, t3_tree, "feat/discount")
t3_message("mcp:f1cf9272", "k1", "user", "Add a function discount(n, pct) to price.js. Keep it tiny.", "2026-11-20T17:18:31Z")
t3_thread("b87ac664", "Casual conversation", gone, None, "main")
t3_message("b87ac664", "g1", "user", "hi", "2026-09-07T20:35:00Z")
t3_message("b87ac664", "g2", "assistant", "Hi! What are we working on today?", "2026-09-07T20:35:01Z")
t3_thread("deleted-1", "Thrown away", "55555555-5555-4555-8555-555555555555", None, "main", deleted=True)
t3_message("deleted-1", "x1", "user", "This thread was deleted in T3 Code.", "2026-11-20T18:00:00Z")
db.commit()
db.close()
# Orca, as its 1.4.271 build lays things out (schemas dumped from what that
# build wrote): the storefront repository with a worktree Orca made under
# ~/orca/workspaces/, and a multi-agent run. Its lead chat ran on OpenCode's
# "Speed up the checkout page" session, which the OpenCode import brings; its
# worker ran as an Orca chat in the worktree, on a session OpenCode no longer
# has, so it comes from Orca's own copy. A Pi chat (Bridge does not run Pi), a
# deleted chat, and an OpenCode terminal worker Orca kept no session for are
# not carried. Its settings, a worker's environment and its runtime token
# hold canaries nothing may ever show.
orca_tree = os.path.join(home, "orca/workspaces/storefront/refund-copy")
git(store_repo, "worktree", "add", "-q", "-b", "refund-copy", orca_tree)
with open(os.path.join(orca_tree, "cart.js"), "a") as out:
    out.write("// uncommitted: refund copy\n")
orca_tree = os.path.realpath(orca_tree)
orca_main = os.path.realpath(store_repo)
orca = os.path.join(home, "Library/Application Support/orca")
orca_fixtures = os.path.join(repo, "testing/fixtures/import/orca/1.4.271")
os.makedirs(os.path.join(orca, "profiles/local-default"))
with open(os.path.join(orca, "orca-profile-index.json"), "w") as out:
    json.dump({"schemaVersion": 1, "activeProfileId": "local-default", "profiles": [{"id": "local-default", "name": "Personal", "kind": "local"}]}, out)
with open(os.path.join(orca, "orca-runtime.json"), "w") as out:
    json.dump({"authToken": "CANARY-orca-secret"}, out)
orca_repo = "6e1ad727-cfd9-4417-b1dc-bdcb536e895f"
def orca_tree_id(path): return f"{orca_repo}::{path}"
db = sqlite3.connect(os.path.join(orca, "profiles/local-default/profile-state.db"))
db.executescript(open(os.path.join(orca_fixtures, "profile-state.sql")).read())
def orca_document(domain, payload, revision=1):
    db.execute("INSERT INTO profile_state_documents(domain,payload,domain_version,revision,updated_at,content_hash) VALUES(?,?,1,?,0,'h')", (domain, json.dumps(payload), revision))
orca_document("schemaVersion", 1)
orca_document("settings", {"opencodeSessionCookie": "CANARY-orca-secret", "agentDefaultEnv": {"ANTHROPIC_API_KEY": "CANARY-orca-secret"}})
orca_document("repos", [{"id": orca_repo, "path": orca_main, "displayName": "storefront", "addedAt": 1795419147883, "kind": "git"}])
orca_document("worktreeMeta", {orca_tree_id(orca_main): {"displayName": ""}, orca_tree_id(orca_tree): {"displayName": "Refund email copy", "baseRef": "main"}}, 7)
db.commit()
db.close()
db = sqlite3.connect(os.path.join(orca, "agent-session-domain.db"))
db.executescript(open(os.path.join(orca_fixtures, "agent-session-domain.sql")).read())
def orca_chat(sid, agent, place, title, handle):
    record = {"schemaVersion": 2, "sessionId": sid, "provider": agent, "conversationName": title, "createdAt": 1795419158665, "updatedAt": 1795419465708,
              "location": {"executionHostId": "local", "wslDistro": None, "workspaceId": orca_tree_id(place), "workspaceKind": "git-worktree"},
              "accountHome": {"kind": "opencode", "locator": {"kind": "unmanaged"}},
              "providerHandleChain": [{"linkId": "l1", "handle": handle, "origin": "created", "mintedAtFence": 2, "observedAt": 1}] if handle else []}
    db.execute("INSERT INTO agent_session_records(session_id, record_json) VALUES(?,?)", (sid, json.dumps(record)))
def orca_items(sid, items):
    db.execute("INSERT INTO journal_chat_state(session_id, workspace_id, epoch) VALUES(?, ?, 'e1')", (sid, orca_tree_id(orca_tree)))
    for index, (role, text) in enumerate(items):
        payload = {"itemId": f"{sid}-{index}", "revision": 0, "observedAt": 1795419160874 + index * 1000, "body": {"kind": "message", "role": role, "blocks": [{"type": "text", "text": text}]}}
        db.execute("INSERT INTO journal_items(session_id, epoch, item_id, ordinal, revision, sequence, sequence_index, owner_fence, payload_json) VALUES(?, 'e1', ?, ?, 0, ?, 0, 1, ?)", (sid, f"{sid}-{index}", index, index * 2, json.dumps(payload)))
orca_chat("orca-lead", "opencode", orca_main, "Speed up the checkout page", {"transport": "acp", "agent": "opencode", "nativeId": "ses_checkout"})
orca_chat("orca-worker", "opencode", orca_tree, "Write the refund email copy", {"transport": "acp", "agent": "opencode", "nativeId": "ses_refund_gone"})
orca_items("orca-worker", [("user", "Write the copy for the refund confirmation email."), ("assistant", "Done: \"Your refund is on its way. It reaches your card in 3 to 5 days.\"")])
orca_chat("orca-pi", "pi", orca_main, "Ask Pi about fonts", {"transport": "jsonl-rpc", "agent": "pi", "nativeId": "pi-1"})
orca_chat("orca-deleted", "opencode", orca_main, "Thrown away in Orca", {"transport": "acp", "agent": "opencode", "nativeId": "ses_deleted"})
db.execute("INSERT INTO agent_session_store_meta(key, value) VALUES('chat.deleted:orca-deleted', ?)", (json.dumps({"chatId": "orca-deleted", "sessionIds": ["orca-deleted"]}),))
db.commit()
db.close()
db = sqlite3.connect(os.path.join(orca, "orchestration.db"))
db.executescript(open(os.path.join(orca_fixtures, "orchestration.sql")).read())
db.execute("INSERT INTO runs(id, objective, coordinator_orca_session_id, coordinator_orca_session_id_generation, consumer_generation) VALUES('run_1', 'Ship the checkout fixes', 'orca-lead', 1, 1)")
db.execute("INSERT INTO dispatch_contexts(id, run_id, task_id, assignee_orca_session_id, status) VALUES('ctx_1', 'run_1', 'task_1', 'orca-worker', 'completed')")
db.execute("INSERT INTO dispatch_contexts(id, run_id, task_id, assignee_handle, status) VALUES('ctx_2', 'run_1', 'task_2', 'term_w', 'completed')")
db.execute("INSERT INTO worker_terminal_resources(id, origin_dispatch_id, owner_dispatch_id, terminal_handle, resume_session) VALUES('w2', 'ctx_2', 'ctx_2', 'term_w', ?)",
           (json.dumps({"agent": "opencode", "launchConfig": {"agentEnv": {"OPENAI_API_KEY": "CANARY-orca-secret"}}}),))
db.commit()
db.close()
# Superset (Paradev), as its 2.0.11 host lays things out (schemas rebuilt
# with its own migrations, rows shaped as that host wrote them): the
# storefront project with a worktree workspace under ~/.paradev/worktrees/,
# an archived one, a chat on OpenCode's "Speed up the checkout page" session
# (which the OpenCode import brings), a chat whose Claude Code session is
# gone (Superset's own copy), a terminal session with Amp (Bridge does not
# run Amp), and an agent configuration with an API key. Its sign-in files,
# host token and the key hold canaries nothing may ever show.
superset_home = os.path.join(home, ".paradev")
superset_org = "7f3c2b1a-4d5e-4f60-8a9b-0c1d2e3f4a5b"
superset_dir = os.path.join(superset_home, "host", superset_org)
os.makedirs(superset_dir)
for name, body in [("auth-token.enc", "CANARY-superset-secret"), ("config.json", json.dumps({"apiKey": "CANARY-superset-secret"}))]:
    with open(os.path.join(superset_home, name), "w") as out:
        out.write(body)
with open(os.path.join(superset_dir, "manifest.json"), "w") as out:
    json.dump({"pid": 1, "endpoint": "http://127.0.0.1:1", "authToken": "CANARY-superset-secret", "organizationId": superset_org}, out)
superset_tree = os.path.join(superset_home, "worktrees/storefront/badge-copy")
git(store_repo, "worktree", "add", "-q", "-b", "badge-copy", superset_tree)
with open(os.path.join(superset_tree, "cart.js"), "a") as out:
    out.write("// uncommitted: badge copy\n")
superset_tree = os.path.realpath(superset_tree)
superset_fixtures = os.path.join(repo, "testing/fixtures/import/superset")
def superset_db(name, head):
    db = sqlite3.connect(os.path.join(superset_dir, f"{name}.db"))
    db.executescript(open(os.path.join(superset_fixtures, f"{name}-{head}.sql")).read())
    for when in open(os.path.join(superset_fixtures, f"{name}-{head}.txt")).read().split():
        db.execute("INSERT INTO __drizzle_migrations(hash, created_at) VALUES('h', ?)", (int(when),))
    return db
db = superset_db("host", 38)
db.execute("INSERT INTO projects(id, repo_path, name, created_at, updated_at) VALUES('p1', ?, 'storefront', 1, 1)", (os.path.realpath(store_repo),))
db.execute("INSERT INTO workspaces(id, project_id, worktree_path, branch, name, type, created_at, updated_at) VALUES('w-badge', 'p1', ?, 'badge-copy', 'Badge copy', 'worktree', 1, 1)", (superset_tree,))
db.execute("INSERT INTO workspaces(id, project_id, worktree_path, branch, name, type, archived_at, archive_reason, created_at, updated_at) VALUES('w-old', 'p1', ?, 'old-badge', 'Old badge', 'worktree', 1795427138800, 'deleted', 1, 1)", (os.path.join(superset_home, "worktrees/storefront/old-badge"),))
db.execute("INSERT INTO terminal_sessions(id, origin_workspace_id, created_at) VALUES('t-amp', 'w-badge', 1)")
db.execute("INSERT INTO terminal_agent_bindings(terminal_id, workspace_id, agent_id, agent_session_id, started_at, last_event_at, last_event_type) VALUES('t-amp', 'w-badge', 'amp', 'amp-1', 1, 1, 'Stop')")
db.execute("INSERT INTO host_agent_configs(id, preset_id, label, command, args_json, prompt_transport, prompt_args_json, env_json, display_order, created_at, updated_at) VALUES('cfg-seeded', 'claude', 'Claude', 'claude', '[]', 'argv', '[]', '{}', 0, 1, 1)")
db.execute("INSERT INTO host_agent_configs(id, preset_id, label, command, args_json, prompt_transport, prompt_args_json, env_json, display_order, created_at, updated_at) VALUES('cfg-key', 'claude', 'Claude (work key)', 'claude', '[]', 'argv', '[]', ?, 1, 1, 1)", (json.dumps({"ANTHROPIC_API_KEY": "CANARY-superset-secret"}),))
db.commit()
db.close()
db = superset_db("chat", 5)
db.execute("INSERT INTO chat_sessions_local(session_id, scope_id, harness, harness_session_id, epoch, status, updated_at) VALUES('c-open', 'w-badge', 'opencode-acp', 'ses_checkout', 'e1', 'idle', 1795426792536)")
db.execute("INSERT INTO chat_sessions_local(session_id, scope_id, harness, harness_session_id, epoch, status, updated_at) VALUES('c-copy', 'w-badge', 'claude-acp', '6a7b8c9d-0e1f-4a2b-8c3d-4e5f6a7b8c9d', 'e1', 'idle', 1795426792536)")
events = [
    {"type": "session", "session": {"status": "running", "harness": "claude-acp"}},
    {"type": "item", "turnId": "t1", "item": {"id": "u1", "startedAtMs": 1795427148315, "kind": "user_message", "content": [{"type": "text", "text": "Shorten the cart badge copy to one word."}]}},
    {"type": "item", "turnId": "t1", "item": {"id": "a1", "startedAtMs": 1795427152979, "kind": "agent_message", "text": ""}},
    {"type": "item", "turnId": "t1", "item": {"id": "a1", "startedAtMs": 1795427152979, "kind": "agent_message", "text": "Done: the badge now says \"Items\"."}},
    {"type": "turn", "turn": {"id": "t1", "status": "completed", "startedAtMs": 1}},
]
for seq, event in enumerate(events, 1):
    db.execute("INSERT INTO chat_journal(session_id, epoch, seq, ts, event_json) VALUES('c-copy', 'e1', ?, 1795427148315, ?)", (seq, json.dumps(event)))
db.commit()
db.close()
# A real chat in a subfolder of the storefront repository: it lands under the
# repository's project. A /usage probe: not a chat. A Slack inbox run Bridge
# itself made in its data folder: not a chat either.
sub = os.path.join(os.path.realpath(store_repo), "src")
os.makedirs(sub, exist_ok=True)
badge = "2b7c9d1e-5f6a-4b8c-9d0e-1f2a3b4c5d6e"
write(f".claude/projects/{''.join(c if c.isalnum() else '-' for c in sub)}/{badge}.jsonl", [
    {"type": "user", "uuid": "b1", "sessionId": badge, "timestamp": "2026-11-18T09:00:00Z", "cwd": sub, "message": {"role": "user", "content": "Make the cart badge show the item count."}},
    {"type": "assistant", "uuid": "b2", "sessionId": badge, "timestamp": "2026-11-18T09:00:20Z", "cwd": sub, "message": {"role": "assistant", "content": [{"type": "text", "text": "The badge now shows the number of items in the cart."}]}},
])
usage = "3c8d0e2f-6a7b-4c9d-8e1f-2a3b4c5d6e7f"
write(f".claude/projects/-work-billing-service/{usage}.jsonl", [
    {"type": "user", "uuid": "q1", "sessionId": usage, "timestamp": "2026-11-18T08:00:00Z", "cwd": "/work/billing-service", "message": {"role": "user", "content": "/usage"}},
])
if bridge_data:
    run_cwd = os.path.join(bridge_data, "chats", "5d9e1f3a-7b8c-4d0e-9f2a-3b4c5d6e7f80")
    run = "4d9e1f3a-7b8c-4d0e-9f2a-3b4c5d6e7f80"
    write(f".claude/projects/{''.join(c if c.isalnum() else '-' for c in run_cwd)}/{run}.jsonl", [
        {"type": "user", "uuid": "s1", "sessionId": run, "timestamp": "2026-11-18T07:00:00Z", "cwd": run_cwd, "message": {"role": "user", "content": "Slack inbox bridge connector: list unread messages."}},
        {"type": "assistant", "uuid": "s2", "sessionId": run, "timestamp": "2026-11-18T07:00:10Z", "cwd": run_cwd, "message": {"role": "assistant", "content": [{"type": "text", "text": "No unread messages."}]}},
    ])
print(home)

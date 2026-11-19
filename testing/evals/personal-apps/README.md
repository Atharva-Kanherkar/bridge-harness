# Personal app evals

Issue #1026, slice 5. These evals measure whether a chat can build a working personal app from a plain ask, on real agents and real MCP servers, and whether the right app opens later.

## Running

```sh
cargo build -p bridge-client --bin bridge -p bridged --bin bridged
node scripts/eval-personal-apps.mjs --bin-dir src-tauri/target/debug [--concurrency 3]
node scripts/eval-personal-apps.mjs --rescore testing/evals/personal-apps/results/<run>   # score saved events again
node scripts/eval-personal-apps.mjs --resume  testing/evals/personal-apps/results/<run>   # redo read, render and lock in place
```

**Setup**

- A `bridged` daemon runs on a fresh data directory, so your own Bridge state is untouched.
- Each case is one real chat (`bridge exec`), on the agent the case names.
- Playwright comes from `BRIDGE_PLAYWRIGHT_MODULE` or `playwright`.

**Scoring.** The scorer reads Bridge's own stored draft, never the model's text.

| Check | What it means |
|---|---|
| drafted | The chat called `apps_draft`, or Bridge's record shows a draft (OpenCode calls it from code mode). |
| server | The app is bound to the server the ask named. |
| reads chosen | The app has at least one read tool. |
| writes match the ask | The app has write tools exactly when the ask needs one. |
| live read with sample arguments | One of the app's reads, run live through the router with the draft's `sample_args`, answers without a server error. |
| renders (light), renders (dark) | The view shows text or controls in Bridge's real `McpAppFrame`, same sandbox and CSP, in each theme. |
| follows Bridge's theme | Its background (or, if transparent, its text) changes with the theme. |
| the view's own call works | After typing the sample query and pressing Enter or the nearest button, a call from the view succeeds. |
| no network from the view | No request and no CSP violation from the view's frames. |
| locks | The draft locks as an app. |

**Writes are never sent.** In the view a write returns Bridge's "Preview only, nothing was sent" answer, and the eval never approves one. Real servers only ever get reads.

**Routing.** Twenty fixed asks run in fresh Codex and Claude chats:

- 12 should open a specific app;
- 8 should open nothing.

Every ask is read-only, because Codex calls MCP tools without asking even in Ask mode. Opening any locked app for the right server counts as correct.

Raw runs (`results/`) hold live read output from your accounts and are gitignored. Never commit them.

## Results (2026-11-19, five runs)

**Servers:**

- Codex: OpenAI Developer Docs (HTTP), Railway, agentic-memory.
- Claude Code: claude.ai Claude Docs.
- Firestore could not connect, because its credentials file is missing. Every Firestore case is a correct "not connected" refusal.
- Slack is excluded.

### Build: apps drafted, of the 6 cases whose server connects

| Run | Change | Drafted |
|---|---|---|
| 1 (baseline) | none | 1 |
| 2 | `apps_draft` says when to use it | 2 |
| 3 | plus a chat-level apps note, and router name correction | 3 (stopped before OpenCode finished) |
| 4 and 5 | plus a guessed tool name is answered with the server's real tools | 6 |

Final checks (run 5, after the frame fixes below):

| Case | Agent | Score | Still failing |
|---|---|---|---|
| docs | Claude | 11/11 | none |
| memory | Codex | 11/11 | none |
| Railway | Codex | 11/11 | none |
| docs | Codex | 10/11 | its `sample_args` do not fit the search tool (its own search works) |
| docs | OpenCode | 10/11 | its view waits for the host to send `ui/initialize`, so it never initializes. The description now says the view sends it; not yet re-measured. |
| Claude Docs | Claude | 9/11 | the app needs a doc link, and there is no sample link |

### Routing (run 5)

| Agent | Should open | Opened | False opens (of 8) | Drafted a new app instead | Answered with the server's own tools |
|---|---|---|---|---|---|
| Claude | 10 | 4 | 0 | 0 | 1 |
| Codex | 10 | 5 | 0 | 0 | 1 |

- **Explicit asks open the right app on both agents.** For example: "Open my memory search app", or "Pull up the app for searching OpenAI documentation, prefilled with…".
- **Indirect asks mostly do not.** For example: "I want to look something up in the docs myself", or "Show me my Railway deployment logs". The chat answers or does nothing instead.
- **No unrelated ask opened an app.**
- **Run 4 regressed and was fixed.** It showed Codex drafting a new app when asked to open a saved one; run 5, with `apps_open` and `apps_draft` separated in the guidance, shows none.

## What the evals found, and what changed

| Finding | Fix |
|---|---|
| Asked for "an app for X", Codex wrote its own web app (files, `npm start`, demo data) in 4 of 6 cases. | `apps_draft`'s description says when to use it. A chat-level apps note sits next to the `visualize` note. |
| Claude refused when the server was not one of its own tools. | Both say to call it anyway; Bridge finds the server on the person's other agents. |
| Chats guessed server names (`openai-developer-docs` for `openaiDeveloperDocs`). | The router corrects a name only when exactly one connected server matches it, ignoring case and punctuation. Otherwise the refusal lists the connected servers. Adapters gained a names-only `servers` request. |
| Chats guessed tool names. Railway's are kebab-case to Bridge but shown to Codex with underscores. | A refusal for an unknown tool lists the server's real tools, marked read or write (at most 60). |
| The stronger note made Codex draft a new app when asked to open a saved one. | The guidance separates `apps_open` (open or use a saved app) from `apps_draft` (build or revise). |
| Views using Bridge's theme variables came out black on white in dark mode. Some never send `ui/initialize`; others never apply the variables. | Every view document declares both color schemes. Bridge writes the current theme into it. A small prelude applies later theme changes. |
| A generated view had the MCP Apps handshake backwards. | The description says the view sends `ui/initialize` first. |

Not fixed here:

- **Write previews can look like successes.** A view showed "Redeploy started: queued" for Bridge's "Preview only, nothing was sent" answer. Nothing was sent, but a preview should be impossible to present as a success.
- **Indirect asks route poorly.**

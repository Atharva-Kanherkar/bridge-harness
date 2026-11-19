# feat/personal-app-evals — Test Contract

Issue #1026, slice 5. Plan: "Plan: personal app evals and reachability (slice 5)" in Bridge Notes.

## Functional Behavior

**Eval runner.** `scripts/eval-personal-apps.mjs`:

- Runs real build and routing chats against a `bridged` daemon on a fresh data directory.
- Scores Bridge's stored drafts (see `testing/evals/personal-apps/README.md`).
- Never approves a write.
- Never uses Slack.
- Raw results are gitignored.

**Product changes the eval found:**

1. **`apps_draft` guidance.** The description says when to use it, and to call it even when the server is not among the chat's own tools. Top-level chats get an apps note in their rendering note. `apps_open` and `apps_draft` are kept apart. The view sends `ui/initialize` first.
2. **Router names.**
   - A server name no agent has is corrected only when exactly one connected server matches it, ignoring case and anything but letters and digits.
   - Otherwise the refusal lists each agent's connected servers.
   - The Claude sidecar and Codex adapter answer a names-only `servers` request (no tools, no calls).
3. **Unknown tools.** A refusal for an unknown tool lists the server's real tools, marked read or write, capped at 60.
4. **Theme in views.** Every view document:
   - declares `color-scheme: light dark`;
   - carries the host theme and variables as an initial `:root` style (dropping values that could leave the rule);
   - has a prelude that applies `hostContext` theme and variables from the host's own messages.

## Unit Tests

- Scorer:
  - Codex and Claude tool events;
  - build checks;
  - theme by background or text;
  - routing verdicts, including drafted-instead and same-server apps.
- Router: name correction (one, none, several) and the connected listing.
- Sidecar: `servers` lists names and statuses only.
- Draft: a guessed tool name is answered with the real tools, capped.
- Frame:
  - the color-scheme meta and prelude precede markup;
  - the prelude applies only the host's messages;
  - the initial style drops unsafe values.

## Evals (manual, results in the README and PR)

Five runs on Codex, Claude Code and OpenCode against real servers, recording before and after numbers for building and routing.

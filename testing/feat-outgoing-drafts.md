# feat/outgoing-drafts: Test Contract

Slice 1 of #920 (Outgoing). A chat agent drafts a message addressed to a person or a place; Bridge checks it against built-in style rules, renders it the way the destination shows it, and the person edits, fixes, copies or discards it from a card in the transcript. **No network sends in this slice**: the card offers Copy, never Send.

## Functional Behavior

### The agent tool: `outgoing_draft`

- Offered on the `bridge` MCP server to top-level chats only (the same `Access::Write` chats that get `notes_create` and `memory_propose`). Read-only sessions (workers, briefings) are not offered it, and a call from one is refused with 403.
- Takes exactly `{ to, body, reason }`. Any other field fails the call (400) instead of being ignored.
- `to` is one of:
  - `{"slack": {"channel": "#dev"}}`, `{"slack": {"channel": "#dev", "thread": "<permalink>"}}`, `{"slack": {"user": "@nina"}}`
  - `{"github": {"repo": "owner/name", "number": 123}}`, optionally `"replyTo": 456`
  - `{"clipboard": "slack" | "github" | "plain"}`
  - Anything else (two families at once, empty strings, a repo without a slash, number 0) is refused with a message saying what is accepted.
- `body`: 1 to 8,000 characters after trimming. A body holding secret material (the same detector `memory_propose` uses) is refused, and nothing is stored.
- `reason`: 1 to 300 characters; why the person wants this message.
- `attachments` (optional): up to 4 absolute paths to local image files, only when the person asked to include an image or screenshot. Each must be a PNG, JPEG, GIF or WebP by its content (magic bytes), not just its extension, and at most 10 MB. Anything else is refused and nothing is stored. GitHub destinations refuse attachments (the API cannot attach uploads to a comment).
- On success one `outgoing_messages` row is stored with `state = draft`, `revision = 1`, and the tool returns `{ id, state, destination, rendered, lint, note }`. `note` tells the agent the person reviews and sends it themselves and that it should not repeat the text.
- Agent drafts get every fixable hard rule applied before storing (em dashes, AI attribution lines). Each fix is recorded as a lint finding with `fixed: true`, so the card can say what changed.
- At most 20 drafts per chat; the 21st is refused (429) so a looping agent cannot flood the store.

### Style rules (built-in, slice 1)

| Rule | Applies to | Fixable | Fix |
|---|---|---|---|
| `no_em_dash` | every destination | yes | `a — b` and `a—b` become `a, b`; an em dash in a code span or fence is left alone |
| `no_ai_attribution` | every destination | yes | lines like `Co-Authored-By: Claude ...`, `Generated with [Claude Code](...)`, `🤖 Generated with ...` are removed |
| `slack_table` | Slack clipboard only | no | a Markdown table is reported (a pasted table stays raw pipes) |
| `slack_heading` | Slack clipboard only | no | a `#` heading is reported (render turns it into bold) |
| `slack_length` | Slack destinations | no | a body over 5,000 characters is reported (the connector's per-element limit) |
| `max_lines` | Slack destinations and Slack clipboard (15 lines) | no | reported with the count |

- Lint is a pure function of `(body, format)`. Linting an already-fixed body reports no fixable findings (idempotence).
- An edit by the person is linted but **not** auto-fixed: findings come back with `fixed: false` and the card offers Fix.

### Rendering

- `slackMarkdown` (Slack channel, thread or DM): the body as written. The Slack connector's send tool takes standard Markdown, including tables, headings and fenced code with a language, and converts it itself.
- `slack` (Slack clipboard): `**b**` / `__b__` become `*b*`; `*i*` / `_i_` become `_i_`; `~~s~~` becomes `~s~`; `[t](u)` becomes `<u|t>`; `# H` becomes `*H*`; `* ` / `+ ` / `- ` bullets become `• `; fenced code and inline code are untouched.
- `github`: the body as written.
- `plain`: emphasis markers removed, `[t](u)` becomes `t (u)`, headings lose their `#`, code fences removed but their content kept.
- The destination picks the format: Slack destinations render `slackMarkdown`, GitHub destinations render `github`, clipboard uses its own format.

### State machine and revisions

- States in this slice: `draft`, `copied`, `discarded`.
- `update_outgoing { id, revision, body?, applyFixes?, attachments? }`: `attachments` replaces the list (the card uses it to remove one), validated like the tool's. refused when `revision` is not the current one ("this draft changed since you opened it") or the item is `discarded`. On success `revision` increases by one, lint and rendered are recomputed, state returns to `draft` if it was `copied`.
- `applyFixes: true` applies every fixable rule to the (new or current) body.
- `copy_outgoing { id, revision }`: returns `{ text }`, the rendered text for the destination's format, and moves state to `copied`. Refused on a stale revision or a discarded item.
- `discard_outgoing { id }`: moves to `discarded`; idempotent.
- `read_outgoing_attachment { id, index }`: the image as `{ mime, dataBase64 }`, re-validated by magic bytes at read time; refused for a missing or changed file.
- `list_outgoing { sessionId }`: the chat's items, oldest first, discarded included (the card shows them as discarded).
- Every change publishes `outgoing/changed { sessionId, id }` (transient).

### The card

- A tool row for `outgoing_draft` (Claude `mcp__bridge__outgoing_draft`, Codex `mcpToolCall` server `bridge` tool `outgoing_draft`, OpenCode `bridge_outgoing_draft`) renders as an Outgoing card instead of a generic tool row.
- While the call is in flight: a quiet placeholder ("Drafting a message…"). A refused call: one muted line that opens to the refusal text.
- A drawn card loads the item by id through `list_outgoing` and refetches on `outgoing/changed` for its chat.
- Shows: destination chip ("Slack · #dev · thread", "Slack · @nina", "GitHub · owner/repo#123", "Clipboard · Slack"), the rendered preview, a chip per lint finding ("Em dash fixed", "Table: Slack shows it as raw text"), and the reason.
- Actions: Edit (textarea, Save calls `update_outgoing` with the current revision), Fix (shown only when an unfixed fixable finding exists), Copy (calls `copy_outgoing`, writes `text` to the clipboard, shows "Copied"), Discard.
- Attachments show as thumbnails with the file name, Copy image (puts a PNG on the clipboard) and Remove.
- No Send button in this slice.
- Tailwind v4 tokens only; `src/designSystem.test.ts` stays green.

## Unit Tests

Rust (`bridge-core`):

- `outgoing::lint::tests::em_dash_spaced_and_tight_become_commas`
- `outgoing::lint::tests::em_dash_inside_code_is_left_alone`
- `outgoing::lint::tests::ai_attribution_lines_are_removed`
- `outgoing::lint::tests::slack_flags_tables_headings_and_length`
- `outgoing::lint::tests::github_and_plain_skip_slack_rules`
- `outgoing::lint::tests::fixing_is_idempotent`
- `outgoing::render::tests::slack_converts_emphasis_links_headings_and_bullets`
- `outgoing::render::tests::slack_leaves_code_untouched`
- `outgoing::render::tests::plain_strips_markup_and_keeps_link_targets`
- `outgoing::tests::destination_parses_every_accepted_shape_and_refuses_the_rest`
- `outgoing::tests::agent_draft_applies_fixes_and_records_them`
- `outgoing::tests::stale_revision_is_refused_and_current_one_bumps`
- `outgoing::tests::copy_returns_rendered_text_and_marks_copied`
- `outgoing::tests::edit_after_copy_returns_to_draft`
- `outgoing::tests::discarded_items_refuse_edits_and_copies`
- `outgoing::tests::list_is_per_chat_and_oldest_first`
- `outgoing_tools::tests::unknown_fields_are_refused`
- `outgoing_tools::tests::secret_material_is_refused_and_not_stored`
- `outgoing_tools::tests::read_only_access_cannot_draft`
- `outgoing_tools::tests::per_chat_cap_is_enforced`
- `notes::tool::tests` extended: `Access::Write.tools()` contains `outgoing_draft`, `Access::ReadOnly.tools()` does not.
- `store` migration test: schema 70 creates `outgoing_messages`.

Frontend (Vitest):

- `src/transcript/outgoing.test.ts`: detects all three harness shapes, ignores another server's `outgoing_draft`, extracts the id from Claude `content` text, Codex `result.structuredContent`, and OpenCode `state.output`.
- `src/components/OutgoingCard.test.tsx` (jsdom): placeholder while drafting; refused line; drawn card shows chip, preview, lint chips; Fix appears only with an unfixed fixable finding; Save sends the current revision; Copy writes the clipboard and shows Copied; Discard shows the discarded state; no Send button.

## Integration / Functional Tests

- `notes::tool::tests::the_loopback_route_serves_outgoing_draft`: over the real loopback route with a valid chat token, `GET tools` lists `outgoing_draft` for a top-level chat; `POST outgoing_draft` stores a row and returns its id; a worker session's token is refused.
- `mcp_apps::server` test: the server lists `outgoing_draft` when the core allows it and forwards the call.
- Protocol gates: `cargo test -p bridge-protocol` (artifacts match, params naming, deny-unknown-fields), `bridge-deck` registry and command-signature tests.

## Smoke Tests

- `bun run check` and `bun run build` are green.
- `bun run test` is green (sidecar node:test, vitest, cargo test --workspace); known load flakes rerun alone.

## E2E Tests

- Mock mode (`bun run dev`): the demo chat contains an `outgoing_draft` tool row; the card renders with a Slack chip, a fixed-em-dash chip and a table warning; Edit, Fix, Copy and Discard work against the mock store. Screenshots (dark, light, edit state) attached to the PR.
- Real app (release bundle): in a Claude chat, "draft a Slack message to #dev saying the deploy is done — keep it short" produces a card with the em dash fixed; Copy puts Slack-formatted text on the clipboard.

## Manual / cURL Tests

Against a running app, with a chat's notes URL and token from its `bridge` server environment:

```bash
curl -s -H "x-bridge-notes-auth: $TOKEN" "$URL/tools"                       # includes outgoing_draft
curl -s -H "x-bridge-notes-auth: $TOKEN" -X POST "$URL/outgoing_draft" \
  -d '{"to":{"slack":{"channel":"#dev"}},"body":"Deploy is done — all green","reason":"tell the team"}'
# -> state "draft", rendered "Deploy is done, all green", lint [{rule:"no_em_dash", fixed:true}]
curl -s -H "x-bridge-notes-auth: $TOKEN" -X POST "$URL/outgoing_draft" -d '{"to":{"email":"x"},"body":"hi","reason":"r"}'
# -> 400, names the accepted destinations
```

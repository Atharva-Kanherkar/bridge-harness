# Test contract — chat UX overhaul (branch `feat/issue-455-chat-ux`)

Locked before implementation. Four workstreams; each lands with the tests below
green in its own area, and the integrated branch passes `bun run check` and
`bun run test`.

## A. Rust — Claude tool normalization (`bridge-core/src/agent.rs`)

- A Claude `tool_use` block named `Bash` normalizes to `command.started` /
  `command.completed` (not flat `tool.*`); `Edit`, `Write`, `MultiEdit`
  normalize to `file_change.*`; other names stay `tool.*`.
- A Claude `Edit` input `{file_path, old_string, new_string}` produces a
  synthesized unified diff in the event data (`patch` or `diff` field) with
  correct `+`/`-` lines, so the frontend renders the same inline patch and
  diffstat as Codex.
- A Claude `Write` input synthesizes a full-file addition diff.
- Claude `tool_result` completion carries aggregated output under the same
  field the frontend reads for `command.completed`.
- Approval titles are harmonized: command approvals ask "Run this command?",
  file-change approvals ask "Edit these files?", MCP approvals name
  server · tool — identical wording across Claude and Codex paths.
- Existing Codex/OpenCode normalization behavior is unchanged (existing tests
  stay green).

## B. Projection — envelopes and humanization (`src/conversation.ts`, `src/humanize.ts`)

- `stripBridgeFences` removes any ```bridge-*``` fence (delegate, peek, steer,
  worker-result, arbitrary future tags) and is applied on BOTH
  `reduceConversation` (live) and `projectSessionEntry` (durable) — an
  assistant message containing a `bridge-delegate` fence renders without it on
  both paths; reload does not regrow envelopes.
- Worker panel objective: a result text beginning `[worker result] …` never
  replaces the spawn objective; the panel keeps the human objective.
- Reasoning coalescing concatenates the resolved display text (`text` falling
  back to `data.summary`), so Codex summary-only reasoning survives merging.
- Durable/live dedupe uses a stable shared key (not `entry.sequence` vs
  `event.id`); a streamed item that becomes durable renders exactly once.
- `humanizeResolution("acceptForSession") === "Allowed for this session"`;
  `humanizeCheckKind` fixes every underscore; steer outcomes map to plain
  words; unknown tokens degrade to sentence case, never raw.

## C. Transcript rendering (`src/components/AgentConversation.tsx`)

- An activity group that was open while live does NOT auto-collapse when the
  turn completes; user toggles are always respected.
- Group summary lines carry real counts ("Read 3 files · edited 1 · ran 2
  commands") instead of the generic phrase.
- Settled reasoning is labeled with its duration when known ("Thought for
  2m 14s") and keeps the last thought line visible in the collapsed summary;
  live reasoning lines wrap rather than truncate.
- Items render in stream order; reasoning/plans are not reordered ahead of
  activity that arrived first.
- Output longer than the display cap shows an explicit "earlier output
  hidden — show all" affordance instead of a silent front cut.
- Approval card: human title/body (via `humanize.ts`), policy code only inside
  a disclosure; resolved state shows the human label, never the raw decision
  token; card errors render in exactly one place.
- Auto-follow: streaming only re-pins when the user is at the bottom;
  scrolling up during a stream is never overridden.
- Optimistic user sends (text + attachments) render in the same single-bubble
  shape as the persisted message.

## D. App state and composer (`src/App.tsx`, composer components)

- Session switch keeps the previous forest rendered until the next forest
  resolves — durable cards (including pending approvals) never flash out.
- A turn with no activity for a threshold surfaces a stall notice with
  Stop / keep-waiting actions instead of an eternal spinner.
- Stop shows immediate "Stopping…" feedback until the turn clears.
- Approval resolution failures surface once (inline), not also as the global
  corner alert.
- Aside composer gains the main composer's typeahead wiring (slash/@/$ and
  ghost suggestion) and no longer disables the textarea during send; queued
  follow-ups are indicated in the aside footer.
- The host chip renders as a static label while only one host exists; the
  worktree toggle names its action/state unambiguously.

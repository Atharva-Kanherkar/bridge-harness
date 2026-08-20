# Contract — briefing as a scheduled worker on the user's harness, and the Work dashboard

Slice 6 of the Work epic. Everything below is written before the implementation and
reviewed against, not after it. The governing correction from the epic: Bridge does not
talk to connectors. The harness the user chose already holds them, so a briefing is a
worker spawned on that harness, asked for one typed result, committed to the durable
board that slice 5 built.

## Settings become writable

Work settings are read-only in production today: `work::stored_settings` reads them and
the only writer lives inside `mod tests`. This slice adds the write path, or nothing
else in it is reachable.

- `work/read_settings` returns the stored settings **and whether they were ever
  written**. `configured: false` with defaults is a fresh install; `configured: true`
  with `briefing: null` is a user who turned briefing off. Those are different stored
  states and stay different.
- `work/write_settings` validates in Rust and persists to `configuration_entries`
  under `(kind='work', id='settings')` with an upsert. The frontend may pre-empt an
  obvious mistake but is never the authority.
- Validation rules (each refused with a reason, never coerced):
  - `cooldownMinutes` in `0..=1440`.
  - `refreshIntervalMinutes`, when present, in `15..=1440` — the same floor the
    learning schedule enforces, for the same reason: a background model run per minute
    is a subscription drain, not a cadence.
  - limits: `maxWallSeconds` in `60..=3600`, `maxTurns` in `1..=64`, `maxToolCalls`
    in `1..=256`, optional token/cost ceilings strictly positive.
  - a briefing profile, when present: the harness must pass the briefing conformance
    gate (`adapter_may_brief`), and the model must be non-empty. An unsupported
    harness is refused with the gate's own reason even when the frontend is bypassed.

## The Settings surface offers only what the gate certifies

- `work/briefing_options` reports every registered adapter with: whether it passed the
  conformance gate, the refusal reason when it did not, its model catalog, and the
  **cheapest capable default** — the Fast-tier default from `resolve_model`, which is
  `haiku` for Claude. The default is chosen at the settings layer; `resolve_briefing`
  keeps refusing an empty model rather than inventing one.
- The Work section in Settings offers supported harnesses as selectable and shows the
  refused ones disabled with their reason. Model defaults to the cheapest capable one
  and stays user-overridable. Cadence, focus-refresh opt-in, and an explicit Off are
  all present.

## One claim path

Manual, focus, and cadence triggers all pass through `work_briefing_trigger::claim`.
No trigger starts a provider any other way.

- **One active run.** A second trigger while a run holds an unexpired lease observes
  the running run; it does not start another.
- **Lease.** Migration 26 adds `lease_owner`, `lease_expires_at`, and
  `cancellation_requested` to `work_brief_runs`. The lease is 15 minutes, heartbeated
  from the run loop, and the hard wall deadline (`maxWallSeconds`, capped at 3600s
  but leased at 15m intervals) is enforced inside the loop so a live worker never
  outlives its lease silently.
- **Reclaim is compare-and-swap.** An expired lease is settled by
  `UPDATE … SET status='failed', failure_code='lease_expired' WHERE id=? AND
  status='running' AND lease_expires_at=?` — zero rows means another trigger won and
  this one observes instead.
- **A stale worker cannot commit.** Before committing, the worker re-asserts its lease
  with `UPDATE … WHERE id=? AND lease_owner=? AND status='running' AND
  lease_expires_at > now`. Zero rows means the run was reclaimed; the worker abandons
  without touching the board.
- **Idempotency per occasion.** A schedule trigger's key is its cadence bucket; a
  focus trigger's key is its cooldown bucket; a manual trigger has no key and is
  repeatable. The partial unique index turns a raced insert into an observation.
- **Cooldown** applies to focus and schedule triggers, never to manual. It is read
  from settings and enforced in Rust.
- **Cancellation** flags the active run; the run loop notices, stops the provider,
  records terminal status and measured usage, and leaves the last good board intact.

## The run is a worker on the user's harness

- The run spawns a hidden session — `kind='briefing'`, no workspace, a scratch cwd —
  through the adapter registry on the harness the settings name, with the briefing
  runtime policy attached. No new connector transport, no MCP client, no Bridge-side
  tool inventory.
- The task text names no connector. The harness's own MCP configuration decides what
  is reachable; the sidecar's briefing gate decides what is allowed.
- The gate gains a **read-verb scope** alongside the reviewed-identity allowlist: a
  connector tool is allowed only when its server is in scope and its name begins with
  a read verb (`search`, `read`, `list`, `get`, `query`, `fetch`, `find`). Everything
  else — mutating verbs included, on a connected server included — is denied. The
  built-in denials (shell, filesystem, web, subagents, skills) are unchanged. The
  exact-identity mode from the conformance suite is untouched; its fifteen cases keep
  passing unmodified.
- **Evidence is observed, never taken on the model's word.** Bridge reads the
  sidecar's own stream: each successful `tool_result` earns a ledger entry, keyed to
  the `tool_use` id the model authored. The brief cites those ids; Bridge translates
  them to ledger references before commit. A citation of a call that failed, was
  denied, or never happened does not resolve, and the existing
  `parse_with_one_repair` refuses it — the single-repair behaviour is reused, not
  duplicated.
- An unparseable result after its one repair fails the run and leaves the committed
  board byte-identical (`abandon_run` touches no task).
- A connector the harness could not reach is marked failed **for that source only**;
  reconciliation already refuses to age a task whose own source was not read.
- Usage is measured from the provider's result messages and lands on the run row
  (tokens, cache reads, cost), so a run is always answerable for what it spent.
  The `usage_ledger` attribution line — background cost beside interactive cost in
  the usage dashboards — is deliberately **not** in this slice; it is issue #222.
- Briefing sessions stay hidden: `kind='briefing'` is already filtered from the rail,
  Mission Control, and default selection by `isHiddenSession`; the session row exists
  so the transcript stays inspectable.

## The Work page

- A dashboard header above the bands: how many things need attention, which tools the
  last run read, when it ran, and how it ended — from the board's existing
  `latestRun`/`sources` fields, with no secrets and no raw provider errors.
- Suggested rows carry real connector logos — Slack, Gmail, GitHub, Linear, Notion —
  as inline SVG with the source still named in text beside the mark. No network
  fetch; the glyph is never load-bearing.
- Rows stay ordered by harness-assigned rank under the pinned-first rule (already
  enforced by `orderTasks`; this slice adds a test that priority orders the suggested
  band).
- Clicking a row routes by the task's own fields: a task with a `workspaceId` opens
  Code on that workspace; anything else stays on Work. A model-authored URL is never
  a route (evidence links already pass through `checked_evidence_target`).
- Manual refresh calls `work/run_briefing`; the focus trigger fires only when the
  stored settings opted in; the cadence trigger runs from the maintenance thread.

## Required tests

- Settings round-trip: written, read back, and each invalid shape rejected from Rust
  with the frontend bypassed.
- Never-configured vs explicitly-off are different stored states.
- An unsupported harness is refused at write time with the gate's reason.
- The cheapest-model default per harness, and a user override winning over it.
- Claim: racing triggers (manual + focus + schedule) start at most one run; the
  losers observe. Idempotency keys are deterministic per occasion.
- Lease: heartbeat extends; an expired lease is reclaimed by CAS exactly once; a
  stale worker's pre-commit assertion fails after a reclaim and the board is
  unchanged.
- Cancellation records terminal status and preserves the board.
- A typed result commits a board; a malformed one is repaired once, then fails with
  the board byte-identical.
- Read-verb scope: a mutating tool on an in-scope server is denied; a read tool on an
  out-of-scope server is denied; oversized arguments are denied; built-ins stay
  denied. Both in the Rust policy and in the sidecar gate.
- Evidence: a cited id that matches an observed successful call resolves; a
  fabricated or failed id does not.
- Row routing: a workspace task routes to Code, everything else to Work.
- Logos render inline with the source in text and no network request.
- `bun run build` and `bun run test` green.

## Out of scope, restated

- Bridge holding connector credentials or speaking MCP directly.
- OS-level background execution while Bridge is not running.
- Automatic task dispatch and connector writes.
- The dormant connector-inventory half of slice 2 (`work_connectors::ConnectorInstance`
  and friends) is neither extended nor removed here.

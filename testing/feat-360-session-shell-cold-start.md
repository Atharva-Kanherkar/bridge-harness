# feat/360-session-shell-cold-start — Test Contract

Source: issue #360 "Session shell: collapse the double title bar, and narrate cold start".
Two independent parts, implemented by two parallel workers, amalgamated into one branch/PR.

---

## Part 1 — One chrome strip on the session view

### Functional Behavior

- On a session view (chat/orchestrator open, no work/projects/marketplace/automations/settings/grid view), `AppTitleBar` does **not** render. `SessionToolbar` is the only chrome row above the conversation.
- Every other view (Work / Projects / Marketplace / Automations / Settings / Mission Control grid) renders `AppTitleBar` exactly as before — unchanged.
- `titleBarActions` (`BypassBadge` + `UsageWidget`) reach the session row via `SessionToolbar`, not a second strip.
- The bypass warning sits on the session row, immediately before the window/session controls (dock, recall, actions menu). It stays a button with its full wording and keeps the `--warning` treatment. When not bypassing it renders nothing.
- The model label becomes a real control: the existing `ChatModelControl` in `compact` form replaces the inert `<p>` model text for non-direct sessions. Pill is capped at `max-w-[190px]`, ellipsizes, and exposes the full unpolluted label via `title`. Clicking opens the same model picker as the composer's.
- `modelDisplayName` strips a trailing `(Unlimited)` provider suffix (case-insensitive) so the pill reads e.g. `OpenCode Zen · Ox Alpha Free`.
- Direct chats keep today's behavior (no toolbar model pill; the composer owns model choice there).
- The dock/recall/actions icons render as one visually grouped cluster: a container padded ~2px with a subtle foreground-tint background (`rgba(foreground, 3.5%)` equivalent token/arbitrary value), preceded by a hairline separator — no borders or shadows beyond that. Elevation stays a lightness ladder.
- Both chrome strips carry `select-none` so a mis-started window drag never text-selects the title.
- Below the `sm` breakpoint the session row gains the nav-open button `AppTitleBar` used to provide (same icon, same aria), so mobile does not lose sidebar access.
- Net geometry: one `h-11` row, one hairline, one right-aligned cluster. Conversation area grows ~44px.

### Unit Tests

- `SessionToolbar.test.tsx`
  - renders `actions` children in the right cluster
  - root carries `data-tauri-drag-region="deep"` **and** `select-none`
  - shows the mobile-only nav button when `onOpenNav` provided (hidden ≥ sm via class)
  - end-to-end wording of the bypass badge survives (rendered through props)
- New `ChatModelControl.test.tsx` (extracted component)
  - compact pill caps width at `max-w-[190px]` when constrained, ellipsizes, `title` carries full label
  - `(Unlimited)` suffix stripped from displayed label
  - picker still opens and calls `onChange`
- `AppTitleBar.test.tsx` — existing expectations stay green.
- `App.test.tsx` source-contract test updated honestly if mount structure changed (no regression to "ships the real rail" assertions).

### Integration / Smoke

- `bun run build` green (tsc strict + vite).
- `bun run vitest run src/components/SessionToolbar.test.tsx src/components/AppTitleBar.test.tsx src/App.test.tsx` green.

---

## Part 2 — Cold start narrated, and measurably less dumb

### Functional Behavior (2a — narration)

- Backend: `start_chat` publishes phase events at real boundaries instead of being fully silent. New transient `CoreEvent::SessionStartup { session_id, phase }` delivered as a new protocol notification (registered exhaustively like every variant; not durable/replayed).
- Phases emitted only where actually observed:
  - `spawning` — after launch planning, at/before child spawn
  - `handshake` — entering readiness wait (OpenCode health poll, Codex `wait_for_response`, Claude Node boot)
  - `session_open` — provider create/resume completed
- No timer-driven fake phases anywhere. If a phase cannot be observed for a harness, it is not emitted for that harness.
- Frontend `bridgeApi.onSessionStartup(handler)` subscribes via the standard subscribe helper (no raw `listen()` — boundary test must stay green); browser/mock mode returns a no-op unlisten.
- `AgentConversation` renders one status row while a cold start is in flight:
  - appears as soon as a pending message exists with no active turn/streaming content
  - labels name the harness: `Starting OpenCode…` → `Waiting for OpenCode to answer…` → `Opening the session…` → `<model> is reading your message…`
  - elapsed counter appears after 2s and keeps running
  - first-launch note shown once when the session has no stored provider id
  - collapses ~2s after the first real token, handing off to the existing `Thinking…` shimmer without remounting the animation node
  - `prefers-reduced-motion` gets a static dot + label (no shimmer)
- Reuses `thinking-shimmer` / `thinking-pulse`; no new motion vocabulary.

### Functional Behavior (2b — speed, safe subset)

- OpenCode `wait_until_ready` backs off 10 → 15 → 25 → 40 → 100 ms between probes instead of flat 100ms.
- Claude sidecar launch sets `NODE_COMPILE_CACHE` (Node 22+) to a Bridge-owned cache dir so SDK module load warms across launches; failure to set it never blocks launch.
- Spawn→ready timing per harness recorded via `process_ledger` hook and surfaced through tracing logs (health-payload surfacing is a stretch goal, not a gate).

Explicitly out of scope for this PR (follow-ups): prewarm-on-intent, shared OpenCode server per workspace, collapsing the four serial frontend round trips, slimming `start_chat`'s return payload.

### Unit Tests

- Rust: backoff schedule unit test in `opencode_adapter.rs` tests module (sequence values, capped).
- Rust: `events.rs` exhaustive-match compiles ⇒ new variant wired into `kind()`/`payload()`.
- Frontend: narration reducer/hook test — given synthetic phase events + timestamps, produces expected label sequence, elapsed visibility at ≥2s, collapse after first token +2s, reduced-motion flag passthrough.
- `api.boundary.test.ts` stays green (subscribe helper used).

### Integration / Smoke

- `cargo check -p bridge-core` (workers) then full `bun run check` at amalgamation.
- `bun run build` green.
- Full `bun run test` green at amalgamation.

---

## Manual Tests (reviewer)

1. `bun run tauri dev`, open a chat session → exactly one 44px chrome row; title left, pill+badge+chips+icon cluster right; drag from title does not select text.
2. Toggle Work/Projects/Settings → old two-strip look intact.
3. Bypass approvals in Settings → warning pill appears on the session row immediately left of the controls, full wording, opens permissions on click.
4. Model pill click → picker opens, switching works, disabled during active turn.
5. Send first message to a cold OpenCode session → status row cycles honest labels, harness named, elapsed counter after 2s, hands off to Thinking… without motion restart.
6. macOS reduce-motion on → static dot + text during startup.

## E2E

N/A — desktop app; manual + unit coverage per above.

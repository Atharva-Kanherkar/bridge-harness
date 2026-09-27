# feat-direct-chat-mode: Test Contract

Branch: `feat/direct-chat-mode`.

## Functional Behavior

- The new-chat (Welcome) composer shows an `Orchestrator` / `Direct` segmented
  control whenever a workspace is selected for the draft.
  - At a 375px window width, the control and Send button remain fully inside
    the composer; the controls may wrap onto another line.
  - It is a `radiogroup` labelled "Chat mode" with two `radio` buttons carrying
    `aria-checked`; arrow keys move the selection.
  - Each option explains itself: Orchestrator = "Bridge plans the work and
    delegates to workers"; Direct = "Talk to the selected harness directly, with
    no Bridge orchestration". The active mode's explanation is visible below the
    composer and wired via `aria-describedby`.
  - Styled only with existing Tailwind tokens (`bg-accent`, `text-foreground`,
    `text-muted-foreground`, `border-border`), matching the composer chips.
  - Hidden when no workspace is selected (non-workspace chats are always direct).
- Default mode is Orchestrator; current behavior is unchanged when the toggle is
  never touched.
- Submitting a workspace draft in Direct mode calls
  `create_workspace_session { workspaceId, createWorktree, kind: "direct",
  harness, model }`. The native session is created with the selected available
  harness/model without resolving an orchestrator profile; effort can be
  applied afterwards.
- Submitting in Orchestrator mode sends `kind: "orchestrator"` (or omits it; the
  server default is orchestrator).
- Backend `create_workspace_session` with `kind = direct` persists a session row
  with `kind='direct'`, the workspace id, the workspace (or worktree) `cwd`,
  `depth=0`, label `Chat`.
- An isolated Direct chat records an owned worktree with the registry's
  `direct` kind, which tracks checkout ownership independently of the
  session's prompt kind.
- Because every start path (`start_chat`, `resume_for_send`) reads the stored
  `kind`, a direct workspace chat compiles the `DirectSession` prompt target on
  every turn and after restart: no `bridge_role` briefing, no
  `delegation_protocol`, while the harness's own configured session prompt is
  still delivered as project rules.
- Non-workspace chats (`create_chat`) and model selection are untouched.
- Wire: `kind` is optional; unknown values are rejected at decode.

## Unit Tests

Rust (`bridge-core`):
- `sessions::tests::direct_workspace_session_persists_direct_kind_in_workspace`:
  row has `kind='direct'`, `workspace_id='w'`, cwd = workspace path, depth 0.
- A Direct workspace session succeeds with its selected harness/model even if
  the orchestrator profile cannot be resolved.
- `sessions::tests::direct_workspace_session_launches_without_bridge_briefing`:
  capturing adapter sees instructions that contain no `bridge_role` /
  `delegation_protocol` / orchestrator briefing text; the orchestrator variant
  of the same test does contain them. Second start (simulated later turn) also
  omits them.
- Existing `welcome_chat_*` / `configured_model_*` tests stay green.

Rust (`bridge-protocol`):
- `CreateWorkspaceSessionParams` round-trips `kind: "direct"`, omits it when
  `None`, rejects `kind: "worker"`, and carries optional `harness`/`model`.
  Generated schema/TS artifacts regenerated.

Frontend (Vitest, jsdom):
- `SessionModeToggle.test.tsx`: renders radiogroup, reflects `value`, click and
  ArrowRight/ArrowLeft call `onChange`, description id wired to
  `aria-describedby`, disabled blocks changes.
- `api` mock: `createWorkspaceSession(id, false, "direct")` pushes a session
  with `kind: "direct"` and the workspace id.

## Integration / Functional Tests

- `bridged` dispatch and Tauri command pass `kind` through to
  `api::create_workspace_session` (compile-enforced; covered by `cargo test
  --workspace` and `bun run check`).
- Protocol drift test (`cargo test -p bridge-protocol`) passes with regenerated
  artifacts.

## Smoke Tests

- `bun run build` green.
- `bun run test` green (sidecar + vitest + cargo workspace).

## E2E Tests

N/A: no E2E harness drives the Tauri app. Covered by the capturing-adapter
Rust test plus the component test.

## Manual / cURL Tests

1. `bun run dev`, pick a project on the Welcome screen, confirm the
   Orchestrator/Direct control appears in the composer and the explanation
   text updates when toggled.
2. Choose Direct, send a message: the new session appears in the workspace with
   the "Chat" role label (not "Orchestrator") in the model chip.
3. In the desktop app, start a Direct workspace chat, restart the app, send a
   follow-up: the prompt compilation record target is `session` / `direct`.

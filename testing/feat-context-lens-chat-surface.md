# feat-context-lens-chat-surface — Test Contract

Implements issue #768: an in-chat Context surface for every live window in a
chat (chat model, orchestrator, workers). Locked before implementation. Design
reference: `docs/mockups/context-lens-in-chat.html`.

## Scope

In this PR:

- Capture real context readings from the harness: Claude (`query.getContextUsage()`
  through the sidecar), Codex (`thread/tokenUsage/updated` `last.totalTokens` +
  `modelContextWindow`), ACP agents (`usage_update` `used` / `size`), OpenCode
  (last assistant step tokens, window from the model catalog).
- Persist readings in a new `context_readings` table and keep
  `sessions.context_percent` current.
- One new wire method `sessions/get_context_windows`.
- UI: context ring in the composer (replacing the usage dot), the usage dot moved
  to the sidebar's bottom rail beside Settings and Gitplace, a Context lens
  modal (one tab per window + the selected window's detail), and the
  model-switch divider showing the window change and what was carried.

Out of scope (follow-ups on #768): per-turn chips on older replies, one-click
disabling of MCP servers / skills, remote hosts.

## Functional Behavior

### Readings (backend)

1. **Claude.** After each `result` frame the sidecar calls
   `run.getContextUsage()` once (never per stream frame), with an 8 s timeout and
   at most one call in flight. On success it writes one `context_usage` frame:
   `{type:"context_usage", model, usedTokens, windowTokens, autoCompactTokens,
   autoCompactEnabled, categories:[{name,tokens,kind}], mcpServers:[{name,tokens,
   tools}], memoryFiles:{count,tokens}, skills:{included,total,tokens},
   agentsTokens, messages:{toolCalls,toolResults,attachments,assistant,user,
   toolsByType:[{name,tokens}]}}`. Lists are capped (categories 16, mcpServers 16,
   toolsByType 8); names are control-char stripped and truncated to 80 chars.
   A missing method, a throw, a timeout, or a malformed payload writes nothing
   and never kills the session. Catalog and usage-probe modes never call it.
2. **Rust intake of the Claude frame.** `context_usage` frames are intercepted
   before normalization (like Codex rate-limit frames): never written to the
   session event log or forest, recorded as a reading with state `measured`,
   then `StateChanged` is published. A frame from a replaced reader launch is
   dropped.
3. **Codex.** A `usage.updated` event carrying `tokenUsage.last.totalTokens` and
   `tokenUsage.modelContextWindow` records a reading with state `reported`
   (used = `last.totalTokens`). The cumulative `total` is never used. The ledger
   row also gets `context_used_tokens = last.totalTokens`.
4. **ACP.** `usage.used_tokens` + `usage.context_window` → reading `reported`.
5. **OpenCode.** used = input + output + reasoning + cache read + cache write of
   the step; window from `model_catalog::context_window_tokens`; state
   `estimated` (the window is Bridge's, not the provider's).
6. **Window fallback.** When the harness reports used tokens but no window, the
   window comes from `model_catalog::context_window_tokens` and the reading is
   `estimated`.
7. **Recording.** A reading stores session id, turn id, harness, model and
   `provider_session_id` as they are on the session at record time, plus used,
   window, state, source, and the breakdown JSON (Claude only). Used tokens are
   never negative; a window ≤ 0 records nothing. `sessions.context_percent` is
   set to `round(100 * used / window)` clamped to 0..=100, only for the row's
   current session.
8. **Stale readings.** The current reading of a session is the newest reading
   whose harness, model and provider session id match the session *now*. After a
   model switch (provider session cleared or changed) the old reading is never
   current. Old readings appear under `earlier` instead.

### `sessions/get_context_windows { sessionId }`

9. Returns `{ sessionId, windows, earlier, bridge }`.
   - `windows`: the requested session first, then every descendant session
     (`parent_session_id` chain, any depth) ordered by depth then start time.
     Each window: `sessionId, label, kind, role ("chat" | "orchestrator" |
     "worker"), harness, model, status, depth, current | null,
     unavailableReason | null`.
   - `current`: `usedTokens, windowTokens, percent, state ("reported" |
     "measured" | "estimated"), source, observedAt, turnId, autoCompactTokens,
     compactionOwner ("harness" | "bridge"), segments, consumers, forecast`.
   - `segments`: from Claude categories (kind `used` / `free` / `buffer` /
     `deferred`), largest first; empty for harnesses that do not split.
   - `consumers`: top tool-result types and MCP servers by tokens, max 6.
   - `forecast`: `{growthPerTurn, turnsRemaining, samples}` only when there are
     ≥ 3 readings for the current thread and positive average growth; target is
     `autoCompactTokens` when known, else the window. Otherwise `null`.
   - `unavailableReason` is set exactly when `current` is null: "Cursor does not
     report context usage", "No reading yet since this model started", etc.
   - `earlier`: the last reading of each previous (harness, model, provider
     thread) of the requested session, newest first, max 8.
   - `bridge`: stable and variable prompt token estimates from the latest prompt
     compilation, or null.
10. Unknown session id → `BridgeError::NotFound`-style error, not an empty list.
11. Params reject unknown fields.

### Model switch divider

12. `session.model_changed` entries gain `previousWindowTokens` and
    `windowTokens` (from the model catalog). The divider shows
    `Sonnet 5.5 → Opus 5.5`, then a second line `window 200k → 1M · fresh thread ·
    carried summary + 2 decisions + 3 files` from the fields that exist. Old
    entries without the new fields render exactly as today.

### UI

13. **Composer ring.** The trailing control that showed `ChatUsageDot` now
    shows `ContextRing` for the selected session: a ring filled to
    `session.contextPercent` and the percent in mono. Null percent → empty ring,
    label "–", tooltip "No context reading yet". Colour: foreground ≤ 74%,
    `text-warning` 75–89%, `text-destructive` ≥ 90% (the `contextPressure`
    thresholds). Click opens the Context lens modal. Accessible name
    "Context window: 38% used" / "Context window: no reading yet".
14. **Usage dot moves.** `ChatUsageDot` is no longer in either composer slot. It
    renders in the sidebar's bottom rail, after Gitplace, with its card opening
    upward and not clipped. Card content unchanged.
15. **Context lens modal.** The composer ring opens a `Dialog` labelled
    "Context lens" over the chat (portalled, scrim, Escape and the close
    button dismiss it). The dock gains no pane and `DOCK_PANES` is unchanged.
    Switching chats closes the lens. While it is open the Browser and Clone
    webviews hide, like under every other modal. It shows:
    - a tab strip (`role="tablist"`) with one tab per window: harness mark,
      role, model, ring and percent; an unavailable window's tab is dashed and
      shows "–", never a percent. The strip is omitted when the chat is the
      only window.
    - the selected window's detail: a pressure hero using `contextPressure()`
      copy, washed and inked by level (healthy success, high warning, critical
      destructive), a large ring with the percent inside, `used of window`,
      free tokens, the reading time and the state badge; a composition bar
      whose segments wear the `ctx-1…ctx-6` hues in rank order with an honest
      hatched "Not attributed" remainder (`used − sum(used segments)`), an
      auto-compact marker and a legend; a forecast line labelled Estimated;
      "What fills it" and "Biggest consumers" rows with proportional bars;
      "What Bridge adds" tiles and "Earlier in this chat" (chat and
      orchestrator windows only); and the ownership note ("Claude compacts
      this window itself").
    - an unavailable window's detail is a dashed hero with the reason and no
      percent.
    - polling: fetch on open, then every 5 s while open and on
      `session.contextPercent` change; serial (no overlap); stop after 3
      consecutive failures or a missing-method error and show a quiet notice.
    - colour: the segment hues are theme tokens (`--ctx-1…6`, both modes) in
      `src/index.css`; no palette class and no hex literal in components.
16. Never draws an unavailable reading as 0%. Estimated values carry the
    dashed Estimated badge.
17. Tailwind v4 only. Inline `style` only for computed widths.

## Unit Tests

Sidecar (`node --test sidecar/claude-agent/test/context-usage.mjs`):
- `contextUsageFrame` maps a full SDK response to the frame shape above.
- caps lists, strips control chars, truncates names.
- aggregates `mcpTools` by `serverName`.
- returns null for malformed input (missing totalTokens / maxTokens ≤ 0).
- `readContextUsage` returns null on timeout, on throw, and when the method is
  absent.

Rust (`bridge-core`):
- `context_windows::tests::claude_frame_becomes_a_measured_reading`
- `..::codex_usage_uses_last_total_not_running_total`
- `..::acp_usage_becomes_a_reported_reading`
- `..::opencode_reading_uses_catalog_window_and_is_estimated`
- `..::missing_window_falls_back_to_catalog_as_estimated`
- `..::recording_sets_session_context_percent_clamped`
- `..::reading_after_model_switch_is_not_current`
- `..::windows_include_descendants_in_depth_order`
- `..::forecast_needs_three_samples_and_positive_growth`
- `..::unknown_session_is_an_error`
- `..::cursor_window_reports_unavailable_reason`
- `usage::tests` Codex ledger row carries `context_used_tokens`.
- `live_turn` test: a `context_usage` frame records a reading and writes no
  session event.
- `agent`/sessions test: `session.model_changed` payload carries window fields.

Frontend (Vitest):
- `ContextRing.test.tsx`: percent, null, thresholds, accessible name, click.
- `ContextLensDialog.test.tsx`: modal mounts in a portal, one tab per window,
  unavailable tab shows no percent, detail hero with pressure and coloured
  composition, unattributed remainder, consumers, forecast, Bridge share,
  earlier list, tab switching, focus request, single-window strip omission,
  polling stops after a missing method, no fetch while closed, close button.
- `AgentConversation` model-change row: new second line; old payload unchanged.
- `BridgeSidebar` test: usage dot renders in the rail.
- `App.test.tsx`: composer no longer contains the usage dot; the ring opens
  the Context lens modal and no Context dock tab exists.

## Integration / Functional Tests

- `bridged` dispatch routes `sessions/get_context_windows` and rejects unknown
  params.
- `protocol_mirror` covers the new params/result.
- Protocol artifacts regenerated; `tsgen::checked_in_artifacts_match_the_contract`
  green; Tauri command signature test green.

## Smoke Tests

- `bun run build` green.
- `bun run test` green (sidecar + vitest + cargo).

## E2E Tests

N/A automated. Manual run below covers the user path.

## Manual Tests

1. `bun run dev` (mock mode): composer shows the ring at the mock percent;
   clicking opens the Context lens modal with a tab per window; Usage dot
   sits in the sidebar rail and its card opens.
2. Built app, fresh Claude chat, one message: within one turn the ring shows a
   real percent and the detail view shows categories. Then
   `sqlite3 -readonly <data>/bridge.db "select state,used_tokens,window_tokens from context_readings order by id desc limit 3"` shows a `measured` row and
   `select context_percent from sessions where id='<id>'` is non-null.
3. Switch the chat to another model: divider shows the window change; ring shows
   "–" until the new model reports.

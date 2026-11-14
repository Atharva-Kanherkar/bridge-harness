# feat/apps-core-registry: Test Contract

Apps slice 2, PR A (issue #990, locked 2026-11-15). The built-in apps become
manifests owned by core. Core stores whether each app is on and where it is
pinned, gates the `bridge` MCP tools and their prompt text on it, computes the
badges, and announces changes with `apps/changed`. PR B (app events into
memory) has its own contract.

## Functional Behavior

### Registry

- `apps/list_apps` on a fresh DB returns 12 apps in this order: notes, trails,
  pulls, workflows, memory, mission-control, terminals, usage, projects,
  agents, store, outgoing. All `enabled: true`, all `pinned: null`. `store`
  has `canDisable: false`; every other app has `canDisable: true`.
- Each summary carries the full `AppManifest` (`id`, `name`, `description`,
  `version`, `source: "builtin"`, `views`, `tools`, `events`, `memory`).
- `outgoing` has the single view slot `card` and the tool `outgoing_draft`.
- Tool ownership: `notes_search`, `notes_read` (asksFirst false) and
  `notes_create`, `notes_edit`, `notes_append`, `notes_set`, `notes_draw`
  (asksFirst true) belong to notes; `memory_search`, `memory_propose` (false)
  to memory; `outgoing_draft` (false) to outgoing. Core tools that no app
  owns: `visualize`, `settings_read`, `settings_change` (false) and
  `notify_user` (true).

### Pins

- `apps/set_app_pinned {id, pinned: true}` appends the app at the next
  position and returns the full list. Pinning an already pinned app is a
  no-op that keeps its position.
- A 4th pin is refused with Bridge's `invalid` error (the code every domain
  refusal uses): "You can pin 3 apps. Unpin one first." Nothing changes.
- Unpinning closes the gap: pins at 0, 1, 2 with the middle one unpinned
  leave positions 0 and 1 in the original order.
- Pinning an app that is off is refused: "Turn <Name> on before pinning it."
- An unknown id is refused: "Bridge has no app called <id>."
- Pins and their order survive closing and reopening the database.

### On and off

- `apps/set_app_enabled {id, enabled: false}` turns the app off and, in the
  same transaction, unpins it. Turning it on again restores `enabled: true`
  and leaves it unpinned.
- `store` refuses to turn off: "The Store cannot be turned off."
- Only departures are stored: an app that was never changed has no row.
- Rows whose id this build does not know are ignored on read.

### Tool gating

- With an app off, `GET /notes-tool/<chat>/tools` omits every tool that app
  owns; core tools are unaffected.
- With an app off, `POST /notes-tool/<chat>/<tool>` for one of its tools
  returns 403 `{"error":"<Name> is turned off in Bridge. The person can turn
  it on in the Store."}` before any dispatch.
- The `bridge` server's `initialize` instructions include an app's paragraph
  only when at least one of that app's tools is listed for the chat; the
  visualize sentence and the core-tool paragraphs (notify_user, settings)
  stay whenever those tools are listed.
- With memory off, `memory_tools::has_memory_search` is false, so no
  `SESSION_GUIDANCE` is added to a chat.
- The tools the Claude sidecar auto-allows (`BRIDGE_READ_TOOLS`,
  `BRIDGE_REVIEWED_TOOLS`, `BRIDGE_PREFERENCE_TOOLS` in
  `sidecar/claude-agent/options.mjs`) are exactly the bridge tools with
  `asksFirst: false`.

### Badges

- workflows: runs where `status = 'waiting_approval'`, or `unread = 1` and
  status is not `running` or `queued`.
- trails: trail summaries in state `needsYou` that are not archived.
- memory: count of proposed records in the account scope, uncapped.
- Every other app: no badge (`null`).
- An app that is off has `badge: null`.
- A badge of zero is reported as `{count: 0}` for apps with a badge source
  that is on.
- A badge source that errors yields `null` for that app; the list still
  succeeds.

### Change hint

- Both setters publish `apps/changed` once per successful call that changed
  state.
- `MemoryChanged`, `WorkflowRunChanged` and `TrailsChanged` also produce
  `AppsChanged`, and a lagged client is reconciled with it.
- `apps/changed` is Transient with no payload (`null`), like
  `attention/changed`.

### Frontend

- The launcher, the sidebar pins and the Store read the app list from core
  (`useApps`). Off apps leave the launcher grid, "Needs you" and the pins.
  `outgoing` never appears in the launcher grid.
- The sidebar Apps badge equals the sum of the badges core returns.
- `apps/changed` makes every view re-read the list, at most once per 250 ms.
- The Store's Apps tab shows an On/Off switch per app (not for `store`) and
  the pin control; a refusal shows core's message inline.
- First run against a backend: a non-empty localStorage
  `bridge.apps.pins.v1` is pinned in order through core, then removed. It
  runs once.
- Mock mode serves the same 12 apps with in-memory state and fires the hint.

### Protocol

- Protocol 1.35. A 1.35 client refuses a 1.34 daemon; a 1.34 client pairs
  with a 1.35 daemon.

## Unit Tests

Rust, bridge-core:

- `apps::manifest::tests::unknown_slot_is_rejected`
- `apps::manifest::tests::duplicate_tool_in_one_app_is_rejected`
- `apps::manifest::tests::a_tool_owned_by_two_apps_is_rejected`
- `apps::manifest::tests::bad_ids_events_and_memory_lines_are_rejected`
- `apps::builtin::tests::builtins_validate_and_list_twelve_apps_in_order`
- `apps::builtin::tests::every_bridge_tool_is_owned_by_one_app_or_core`
- `apps::builtin::tests::sidecar_allowlist_matches_asks_first`
- `apps::tests::attention_owner_covers_every_kind`
- `apps::registry::tests::fresh_db_lists_twelve_apps_on_and_unpinned`
- `apps::registry::tests::fourth_pin_is_refused`
- `apps::registry::tests::unpin_closes_the_gap_and_keeps_order`
- `apps::registry::tests::pins_survive_reopening_the_database`
- `apps::registry::tests::pinning_an_off_app_is_refused`
- `apps::registry::tests::turning_off_unpins`
- `apps::registry::tests::store_cannot_be_turned_off`
- `apps::registry::tests::unknown_ids_are_refused_and_stored_unknowns_ignored`
- `apps::registry::tests::only_departures_are_stored`
- `apps::badges::tests::workflow_badge_matches_the_frontend_predicate`
- `apps::badges::tests::memory_badge_counts_proposals_without_a_cap`
- `apps::badges::tests::trails_badge_counts_needs_you`
- `apps::badges::tests::off_apps_have_no_badge`
- `notes::tool::tests::an_off_app_leaves_the_tool_list_and_its_calls_are_refused`
- `mcp_apps::notes_tools::tests::instructions_follow_the_listed_tools`
- `memory_tools::tests::no_guidance_when_memory_is_off`
- `events::tests::badge_sources_also_publish_apps_changed`

Rust, bridge-protocol:

- round trip for `AppManifest`, `AppSummary`, `ListAppsResult`,
  `SetAppPinnedParams`, `SetAppEnabledParams`
- the existing schema drift and params naming gates
- handshake test naming 1.34 and 1.35 in both directions

Vitest:

- `src/apps/store.test.ts`: loads the list, re-reads on `apps/changed`,
  coalesces bursts, keeps only the newest response
- `src/apps/pins.test.ts`: the one-time localStorage import
- `src/apps/registry.test.ts`: every core id has a frontend descriptor except
  `outgoing`, and every descriptor has a core id
- `src/components/AppLauncher.test.tsx`: off apps leave the grid and "Needs
  you"
- `src/components/BridgeSidebar.test.tsx`: the Apps badge is the sum of core
  badges
- `src/components/StoreApps.test.tsx`: the On/Off switch turns an app off and
  the launcher list drops it

## Integration / Functional Tests

- `protocol_wire` round trip of a real `list_apps` payload into the wire type
  (the core function returns the typed struct, so this is also compile-time).
- `src-tauri/src/lib.rs` registry and handler 1:1 test, and the bridge-deck
  command signature test, cover the three new commands.
- `notes::tool` over loopback HTTP: list and call with Notes off and on.

## Smoke Tests

- `bun run build` and `bun run test` green.
- `cargo test -p bridge-protocol` green (drift).

## E2E Tests

- Mock mode (`bun run dev`): open the Store, turn Notes off, confirm the
  launcher no longer lists it and the pin is gone; turn it on again. Dark and
  light screenshots of the Store toggle and the launcher.

## Manual / cURL Tests

```sh
# Built daemon on a short data dir
bridged --data-dir /tmp/bapps --health-addr 127.0.0.1:4395 &
bridge exec --data-dir /tmp/bapps --method apps/list_apps --params '{}'
bridge exec --data-dir /tmp/bapps --method apps/set_app_pinned --params '{"id":"notes","pinned":true}'
bridge exec --data-dir /tmp/bapps --method apps/set_app_pinned --params '{"id":"pulls","pinned":true}'
# restart the daemon, list again: notes at 0, pulls at 1
bridge exec --data-dir /tmp/bapps --method apps/set_app_enabled --params '{"id":"store","enabled":false}'   # invalid_params
```

Live tool gating: start a Claude chat and a Codex chat, ask each to list its
`bridge` tools, turn Notes off, start new chats, ask again. Expected: no
`notes_*` the second time. OpenCode is reported as pending (OpenCode 2.x
attaches no `bridge` tools at all).

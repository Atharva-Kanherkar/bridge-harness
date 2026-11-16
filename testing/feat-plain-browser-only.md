# feat/plain-browser-only — Test Contract

Throwaway signed-in browser clones are switched off for agents and in the UI.
The code stays. An ordinary, not signed in browser is the only browser an agent
is told about.

## Functional Behavior

Agent side (`bridge-core`, `live_turn.rs`):
- Every turn on a runtime that supports trusted turn context carries one plain
  browser note. It says: use ordinary browser automation with a fresh profile
  that is not signed in, this is the only browser there is, there is no
  signed-in browser, clone or cookie import, and what to do when a task needs
  sign-in.
- No turn carries the clone request prompt or the clone drive prompt.
- No `clone-request-<session>` script is written to the tool directory.
- A runtime without trusted turn context still gets no browser note.
- The attached-tab browser bridge context is unchanged.
- Credential context and the session frame are unchanged, and the session
  frame is still delivered once.
- One constant, `OFFER_BROWSER_CLONES`, restores the old behaviour.

Frontend:
- The dock Browser pane is `SimpleBrowser` only: address bar and tabs, no
  clone viewport, no "Open a throwaway copy of a site" button.
- The Browser dock tab never shows a clone attention dot.
- No clone request card is mounted in the app.
- Settings has no "Browser copies" row under Permissions, no `clones` section,
  and search returns no clone rows. The "API keys" row stays.
- A dock layout saved with pane `clone` still lands on Browser.

Not removed: `CloneSurface`, `CloneRequestInbox`, `ClonesPage`, `cloneSettings`,
the `api.ts` clone methods and mocks, every Rust clone module, the wire
methods and generated protocol artifacts.

## Unit Tests

Rust (`cargo test -p bridge-core`):
- `live_turn::tests::turn_offers_only_the_plain_browser` — the context has the
  plain note, has no `clone-request-`, and the tool directory holds no
  `clone-request-chat`.
- `live_turn::tests::plain_browser_note_names_no_clone_tool` — the note text
  has no tool path and states the browser is not signed in and is the only one.
- Existing `clone_application_request_approval_handoff_and_turn_cleanup_repeat`
  still passes, minting the request capability through the orchestrator.
- Existing clone_browser_tool, clone_orchestrator, browser_clone tests pass
  unchanged.

Frontend (vitest):
- `BrowserPane.test.tsx` — shows the address bar; no clone viewport; no
  throwaway button; never calls `browserCloneState`.
- `App.test.tsx` — Browser pane in workspace and direct chats shows browser
  pages and no clone viewport; no Browser attention dot.
- `sections.test.ts`, `settingsSearch.test.ts`, `SettingsScreen.test.tsx` — no
  clones section, no clone search rows, no "Browser copies" row.
- Existing `CloneSurface.test.tsx`, `CloneRequestInbox.test.tsx`,
  `ClonesPage.test.tsx`, `api.clone.test.ts`, `dockLayout.test.ts` pass
  unchanged.

## Integration / Functional Tests

- `bun run test` (sidecar, vitest, cargo workspace) green.
- `cargo test -p bridge-protocol` green: no protocol drift.

## Smoke Tests

- `bun run build` green.
- `bun run check` green.

## E2E Tests

- Mock mode (`vite`), headless Chrome: open a chat, open the Browser pane, see
  the plain browser in dark and light; open Settings, Permissions, see no
  "Browser copies" row.

## Manual / cURL Tests

- `git diff --stat origin/main` shows no deleted file.
- `grep -rn "clone-request\|Bridge browser clone" ` over a live turn's injected
  context: N/A without a live provider; covered by the Rust turn test.

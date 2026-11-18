# feat/personal-app-store — Test Contract

Issue #1026, slice 4c-1. Plan: "Plan: My apps in the Store, launcher and memory (slice 4c)" in Bridge Notes, Revision 2. Memory (4c-2) is out of scope.

## Functional Behavior

- **Generation.** Every locked personal app has a generation (uuid), minted at its first lock and removed on delete. Opens, approvals and ledger rows carry it. An open whose generation is not current is refused. Locked content hashes are unchanged.
- **Surface opens.**
  - `apps/open_personal_app_surface {appId}` creates a Surface open of the latest locked version: owner `surface`, no session, bound to the app id, generation, version and content hash.
  - Opening again replaces the previous Surface open for that app and revokes its pending approval.
  - `apps/close_personal_app_surface {openId}` removes it and its approval.
  - Daemon start clears all Surface opens.
- **Owner.** `get_personal_app_open` and `call_personal_app_tool` take `owner: {kind: "chat", sessionId} | {kind: "surface"}`. The open's stored owner must equal it, otherwise "This app is unavailable here".
- **Reads.**
  - A chat open follows the chat's access mode, as before.
  - A Surface open is allowed.
  - Both need the OpenCode opt-in for OpenCode connections.
- **Runtime.** A Surface open never uses a chat's live process.
- **Off.** Turning a personal app off, in one transaction:
  - unpins it;
  - revokes the pending approvals of all its opens;
  - removes its Surface opens.

  While it is off:
  - its chat opens refuse calls;
  - `apps_open` neither lists nor opens it;
  - Surface creation is refused.
- **Delete.** Deleting removes the app's versions, drafts, opens, approvals, generation, on/off row and pin.
- **Claiming a write.** The token is claimed in the same transaction that checks the app is on and the generation is current.
- **Ledger.** Each call records `outcome`: `confirmed` (Ok, with `isError` not true), `failed` (refused before dispatch, or `isError: true`) or `unknown` (a transport error after dispatch began). It also records `generation`. It never records arguments.
- **Catalogue.**
  - `apps/list_apps` returns `personal: [{app, enabled, pinned}]`.
  - Personal apps can be pinned, sharing the 3 pins, and turned off.
  - `AppsChanged` fires on lock, delete and toggle.
  - Protocol 1.43.
- **Frontend.**
  - The Store's Apps tab has a "My apps" section with Open, Pin, On/off and Delete (with confirmation).
  - ⌘K has a "My apps" section.
  - Sidebar pins resolve personal apps.
  - A `personal-app` view, keyed by app id, takes part in back and forward.
  - The Surface screen renders the app with `owner: surface` and closes its open on unmount.
  - Off or deleted apps show that state.

## Unit Tests

- Generation: minted once, kept across versions, new after delete and recreate. An old open is refused after recreating with identical content.
- Owner mismatch refused both ways. Surface reads allowed where a chat in Ask mode is refused.
- Off revokes approvals, and a claim after off is refused. Delete removes pins and on/off rows.
- Ledger outcome classification for Ok, `isError: true`, a refusal before dispatch, and a transport error.
- No MCP tool and no settings catalog key creates a Surface open.
- Registry: personal ids pin and toggle. Unknown and deleted ids are dropped from pins.
- Frontend: Store "My apps" actions, launcher section, `placesEqual` with an app id, pins for personal apps, and the Surface screen opening and closing.

## E2E

- Real app, Codex-locked app:
  1. Open it from the Store and from ⌘K; back and forward work; pin it.
  2. Do a read, then approve a write and check it lands once.
  3. Start a write and turn the app off before approving: the approval is refused.
  4. Delete it and recreate the same slug: the old pane is refused.

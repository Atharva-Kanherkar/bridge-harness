# feat/clone-trigger — test contract

The last hop: make the browser-clone scenario startable from the app. Stacked on
the integration (#759) and parts 1–4. Adds the `clones/*` wire methods, the
frontend's real calls, and a Start control in the dock. macOS runtime; the wire
methods exist on every platform. Part of #758.

## Shape of the thing

- `bridge-protocol`: a new `clones` domain — `request_clone`, `clone_state`,
  `takeover_clone`, `hand_back_clone`, `destroy_clone`, with `CloneSnapshot`
  (no cookie value). Registered, dispatched, and mirrored; artifacts regenerated.
- `bridge-core::api`: the five functions, macOS bodies over the orchestrator,
  `Ok(None)` / "not available" off macOS.
- `src-tauri`: the five Tauri commands (the blocking ones on the blocking pool),
  registered in the handler.
- `src/api.ts`: real `call("clones/…")` under Tauri, mapped to the dock's
  snapshot; the mock fixture still drives dev and tests. A new `requestClone`.
- `CloneSurface`: a Start control (type a site → `requestClone`), and every
  action now carries the session id.
- `App`: passes the active session id to the pane.

## 1. Wire contract — Rust gates

| # | Behaviour | Assertion |
|---|---|---|
| 1.1 | The methods are registered and mirrored | `bridge-protocol` tests pass; `params_types_are_named_after_their_method` holds; checked-in artifacts match |
| 1.2 | Every command matches its wire params | `bridge-deck` `every_command_signature_matches_its_contracted_params` passes |
| 1.3 | The registry and the handler stay 1:1 | the lib.rs registry↔handler test passes |
| 1.4 | The crate builds on every platform | the `clones` api is cross-platform; macOS-gated bodies, `cargo check --workspace` clean |

## 2. Frontend — `CloneSurface.test.tsx`, `api.boundary.test.ts`

| # | Behaviour | Assertion |
|---|---|---|
| 2.1 | The empty state offers a Start control | "no clone" shows a site input and a Start clone button, and no takeover/destroy |
| 2.2 | Start requests a clone for the typed site | typing `youtube.com` and pressing Start calls `requestClone(session, "youtube.com", "chrome", "import")` |
| 2.3 | Actions carry the session | takeover/hand back/destroy call the api with the pane's session id |
| 2.4 | The mock path still works | outside Tauri the fixture drives the surface; the 12 existing surface tests stay green |

## 3. Repo gates

`bun run check`, `bun run test`, `bun run build` green. The four `updater.test.ts`
and four `bridge-core` PTY failures pre-exist on `origin/main` here.

## What this finishes, and what it does not

- **Finishes:** the click path exists end to end. In the desktop app, the dock's
  Start control calls `clones/request_clone`, which drives the real orchestrator
  (spawn guarded clone, sign in, arm guard, mint the agent tool); the dock polls
  `clones/clone_state` and renders the real redacted frame; takeover/destroy hit
  the runtime; the clone is destroyed on session end.
- **Still ahead:** an agent *asking* (rather than the person starting it) needs
  a turn-side request signal and the approval card; streaming frames
  (`Page.startScreencast`) and input forwarding during takeover; a TTL scheduler
  tick calling `sweep_expired`. And the packaged-app screen recording, which
  needs a release bundle to capture.

# codex/issue-157-open-harness-id — Test Contract

First slice of #157 (RPC layer of the #155 ACP marketplace epic). Opens the
harness identifier so an agent installed at runtime can own a session. No
marketplace methods, no notifications, no installer — those are the following
PRs, and the sequencing note at the bottom says why.

## The decision, and its trade-off

The issue offers two shapes: a validated newtype `HarnessId(String)`, or
`Builtin(...) | Acp(String)`. This branch takes **both, at different layers**,
because the two layers want opposite things.

- **The wire** (`bridge_protocol::HarnessId`) is a validated newtype over
  `String`, serialized transparently. A client compiled today must accept an id
  that did not exist when it was compiled; a closed TypeScript union is exactly
  the thing that cannot. Every value protocol 0.8 could produce —
  `"claude"`, `"codex"`, `"opencode"`, `"shell"` — serializes byte-identically.
- **The core** (`bridge_core::model::Harness`) keeps the closed enum and gains
  two arms: `Acp(AcpAgentId)` and `Unknown(String)`. Exhaustiveness is retained
  precisely where behaviour is bespoke — each built-in has a hand-written
  adapter — so adding a built-in still fails to compile until every site names
  it.

Trade-off, stated plainly: exhaustive `match` is kept where it buys
compile-time safety over differing behaviour, and given up where it would
break forward compatibility. The cost is one conversion boundary
(`HarnessId ↔ Harness`) that must be total in both directions, which is
covered by tests below rather than by the type system alone.

## Grammar, and why collision checking is structural

The wire id is one of exactly two shapes:

```
HarnessId := "claude" | "codex" | "opencode" | "shell"   (built-in)
           | "acp:" <registry-id>                        (ACP agent)
<registry-id> := [a-z0-9][a-z0-9._-]{0,63}
```

The bare namespace is reserved for built-ins: a bare id that is not a built-in
is **rejected**, not accepted as an agent. Without that rule `gemini` is
permanently ambiguous — a future built-in, or an ACP agent installed today.

The `acp:` prefix is not decoration. **The live registry ships an entry whose
id is `opencode`**, which collides head-on with Bridge's built-in OpenCode
adapter (verified against `testing/fixtures/acp-registry-v1.json`, 38 entries).
`AdapterRegistry::register` already rejects a duplicate id
(`adapters.rs:329`), so an unprefixed scheme would have made that collision a
runtime registration failure the first time anyone installed OpenCode over ACP.
Prefixing makes the collision the issue asks us to validate against
*structurally impossible* rather than merely detected.

## Functional Behavior

- Every harness id protocol 0.8 could send or receive round-trips unchanged
  through 0.9. No existing wire value, DB value, or adapter-registry key moves.
- A wire `HarnessId` that is neither a built-in nor a well-formed `acp:` id is
  rejected at the dispatch boundary with `invalid_params`, naming the value —
  not coerced, and not carried into the runtime.
- `harness_name(&Harness)` returns the **one canonical string** used as the
  wire value, the `sessions.harness` column value, and the `AdapterRegistry`
  key. There is no second spelling of a harness anywhere.
- The `sessions.harness` column is already `TEXT NOT NULL` (`store.rs:584`,
  `store.rs:839`). No schema migration is required and none is added; the
  change is to stop *interpreting* that text lossily.
- `store::harness` no longer falls through `_ => Harness::Shell`
  (`store.rs:1435-1438`). An id this build cannot interpret becomes
  `Harness::Unknown(raw)`, preserving the raw string. Today's fallthrough turns
  a corrupt or forward-dated row into a **runnable Shell session**; that is the
  "default that misrepresents it" the issue's acceptance criterion names.
- A session whose harness is uninstalled or unknown loads, lists, and replays
  its full history, and renders under its own id. Starting a turn on it fails
  with an adapter-not-found error naming the harness. History never vanishes
  and is never silently re-attributed to a different agent.
- `Harness::Unknown` is unreachable from the wire — only from reading a row
  written by a different build. It is never constructed as a guess.
- Protocol minor bump 0.8 → 0.9, TypeScript regenerated, `HarnessId` becomes
  `string` in `src/protocol/generated/protocol.ts`.

### Compatibility caveat, recorded deliberately

Widening a value domain that appears in *results* is not purely additive for a
strictly-typed old client: a Rust client built against 0.8 deserializing a
`BridgeState` containing `acp:gemini` would fail its enum decode. It is filed
as a minor bump anyway because the new values are unreachable on this build —
nothing can install an ACP agent until the installer lands in #156 — so the
window in which a 0.8 client could meet one is empty. If that stops being true
before #156 lands, this becomes a major bump.

## Unit Tests

- `messages::sessions::tests::harness_ids_accept_builtins_and_acp_ids` — the
  four built-ins and a representative `acp:` id parse; each serializes back to
  the identical string.
- `messages::sessions::tests::harness_ids_reject_malformed_values` — empty,
  over-length, uppercase, leading punctuation, whitespace, an unknown bare id,
  a bare `acp:`, and a nested `acp:acp:x` are all refused.
- `messages::sessions::tests::builtin_harness_ids_serialize_exactly_as_protocol_0_8`
  — pins the four legacy strings against a literal, so a refactor cannot
  silently rename a persisted value.
- `messages::sessions::tests::acp_prefix_makes_builtin_collision_unrepresentable`
  — the registry's real `opencode` entry maps to `acp:opencode` and is never
  equal to the built-in `opencode`.
- `lib::tests::harness_id_and_core_harness_convert_in_both_directions` — the
  conversion is total over built-ins and `Acp`, and round-trips.
- `lib::tests::unknown_harnesses_are_not_representable_on_the_wire` —
  `Harness::Unknown` cannot produce a valid `HarnessId`; the failure is
  explicit rather than a coercion to Shell.
- `store::tests::harness_column_round_trips_every_shape` — built-in, `acp:`,
  and unknown values survive write → read → write byte-identically.
- `store::tests::unknown_harness_rows_do_not_become_shell` — the regression
  test for the removed fallthrough; asserts a corrupt row is `Unknown`, not a
  runnable harness.
- `acp_registry::tests::every_live_registry_id_is_a_valid_harness_id` — all 38
  ids in the pinned fixture produce valid `acp:` harness ids, so the catalog
  cannot contain an entry Bridge is structurally unable to name.
- `protocol_mirror::tests::*` — existing mirror assertions extended over the
  new arms; core and wire continue to agree on every value.

## Integration / Functional Tests

- A session created with an `acp:` harness id round-trips
  create → persist → reload → replay → snapshot, and the id in the
  `BridgeState` result equals the id sent. This is the issue's headline
  acceptance criterion — an id unknown at compile time surviving the whole
  path — exercised without an installer by writing the id directly.
- A `sessions.harness` row holding an uninstalled `acp:` id loads, appears in
  the state snapshot, and replays its events; `start_session` on it returns an
  adapter error naming the harness rather than starting a different one.
- Dispatch rejects a malformed harness id in `sessions/create_chat` with
  `invalid_params` before any core call.
- The existing daemon and client suites pass unchanged, proving the built-in
  path is untouched.

## Smoke Tests

- `cargo test -p bridge-core` — 467 tests passing on `a1c9261` before this
  branch; the count must rise and none may fail.
- `cargo test -p bridge-protocol -p bridge-client -p bridged`.
- `cargo build -p bridge-core -p bridge-protocol -p bridge-client -p bridged`.
  (`--workspace` fails on `bridge-deck` for a missing `binaries/bridged-*`
  sidecar; that is pre-existing and out of scope.)
- The generated-TypeScript freshness test passes after regenerating with
  `cargo run --manifest-path src-tauri/Cargo.toml -p bridge-protocol --bin generate-protocol-artifacts`.
- `bun run build` and `bun run test` succeed.
- Threaded tests repeated several times to rule out flake.

## Out of scope on this branch

Named so the PR is not read as a partial delivery of the issue:

- Marketplace methods (catalog/install/auth) and their notifications — next PR.
- The `CoreEvent` variant and background catalog refresh — next PR; it needs a
  notification-registry entry, which is a separate reviewable change.
- Install and auth methods — blocked on #156, which owns the installer and the
  `AcpAdapter`. Implementing them here would ship stubs that report failure for
  reasons unrelated to what the user did, which is the opposite of the epic's
  "failures must be legible" boundary.

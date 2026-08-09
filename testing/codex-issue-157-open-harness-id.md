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

### Amendments — two sites the contract did not anticipate

Both were found by following acceptance criterion 4 ("no `match` on harness
falls through to a default that misrepresents it") through code the original
contract had not looked at. Recorded here rather than shipped silently.

1. **`delegation::normalize_harness`** folds a model-written harness hint into
   a canonical id, and `runtime_harness()` ends `.unwrap_or_else(|| "codex")`.
   A valid id the table did not recognize was therefore rewritten to Codex —
   the exact failure mode the criterion names. It is currently unreachable only
   because `validate()` rejects unrecognized hints first, which is an invariant
   held by call order rather than by types. `acp:` ids are now accepted, which
   both closes that gap and makes an installed agent delegable at all —
   without it the marketplace never reaches the delegation tree, which is the
   epic's whole point.

   `shell` is deliberately **not** added to the table even though it now
   parses. Admitting it would let a delegation directive spawn a shell worker
   where it previously could not; widening what a model may delegate to is a
   product decision, not a side effect of opening an identifier.

2. **Frontend harness labels.** Three near-identical `harnessLabel` copies
   lived in `App.tsx`, `BridgeSidebar.tsx`, and `AgentConversation.tsx`, each
   ending in a capitalize-the-id fallback that renders `acp:gemini` as
   `Acp:gemini`. They are now one `utils.harnessLabel` that strips the
   namespace and shows the agent under the id it was installed by. Three
   divergent copies is how the next harness ends up mislabelled in two of the
   three, so this is consolidated rather than patched three times.

### Compatibility caveat, recorded deliberately

Widening a value domain that appears in *results* is not purely additive for a
strictly-typed old client: a Rust client built against 0.8 deserializing a
`BridgeState` containing `acp:gemini` would fail its enum decode. It is filed
as a minor bump anyway because the new values are unreachable on this build —
nothing can install an ACP agent until the installer lands in #156 — so the
window in which a 0.8 client could meet one is empty. If that stops being true
before #156 lands, this becomes a major bump.

## Unit Tests

`HarnessId` landed in `messages/common.rs` rather than `messages/sessions.rs`:
`common.rs` is documented as "wire types more than one domain needs" (both
`sessions` and `state` use it), and `JsSafeI64` there is the exact precedent —
a validated newtype with transparent `Serialize`, a hand-written `Deserialize`,
and a custom `JsonSchema`. The module convention of one file per *method
domain* is preserved by not inventing a `harness` domain module. Test paths
below reflect that.

- `messages::common::tests::harness_ids_accept_builtins_and_acp_ids`
- `messages::common::tests::harness_ids_reject_malformed_values` — empty,
  over-length, uppercase, leading punctuation, whitespace, an unknown bare id,
  a bare `acp:`, and a nested `acp:acp:x` are all refused.
- `messages::common::tests::builtin_harness_ids_serialize_exactly_as_protocol_0_8`
  — pins the four legacy strings against a literal, so a refactor cannot
  silently rename a persisted value.
- `messages::common::tests::acp_prefix_makes_builtin_collision_unrepresentable`
  — the registry's real `opencode` entry maps to `acp:opencode` and is never
  equal to the built-in `opencode`.
- `messages::common::tests::rejection_names_the_value_without_echoing_an_unbounded_one`
  — added while writing the error type: the rejection must name what was sent
  without letting a 10 KB payload inflate the error it provokes.
- `messages::common::tests::harness_id_schema_publishes_the_grammar_as_a_string`
  — the published schema carries a `pattern` and **no** `enum`, so a generated
  client cannot close the set over today's values.
- `protocol_mirror::harness_ids_round_trip_with_identical_wire_values` — the
  existing mirror assertion, extended over `Acp`.
- `protocol_mirror::builtin_harnesses_keep_the_wire_values_protocol_0_8_published`
- `protocol_mirror::a_registry_agent_named_like_a_builtin_stays_distinct_on_the_wire`
- `protocol_mirror::unknown_harnesses_serialize_under_their_own_id_but_are_not_valid_parameters`
  — the deliberate outbound-tolerant / inbound-strict asymmetry.
- `protocol_mirror::a_stored_harness_id_is_never_read_as_a_different_harness`

  The two conversion tests the contract placed in `lib::tests` live in
  `protocol_mirror` instead, beside the existing core↔wire mirror assertions
  they belong with; `lib.rs` no longer holds harness code at all.

- `store::tests::the_harness_column_round_trips_every_shape`
- `store::tests::an_unreadable_harness_row_never_becomes_a_runnable_harness` —
  the regression test for the removed fallthrough.
- `store::tests::a_session_of_an_uninstalled_harness_still_loads_with_its_history`
  — a real SQLite round trip: the session appears in the snapshot and replays
  its entry.
- `acp_registry::tests::every_live_registry_id_can_name_a_harness` — all 38 ids
  in the pinned fixture produce valid `acp:` harness ids, so the catalog cannot
  contain an entry Bridge is structurally unable to name.
- `acp_registry::tests::a_registry_id_matching_a_builtin_does_not_shadow_it` —
  pins the colliding set to exactly `["opencode"]`, so a second collision
  appearing upstream is a test failure rather than a surprise.
- `utils.test.ts` → `harnessLabel` — five cases covering built-ins, an `acp:`
  agent, the built-in/registry pair that share a name, an uninterpretable id,
  and the no-harness fallback.

## Integration / Functional Tests

- `bridged/tests/daemon.rs::a_harness_unknown_at_compile_time_round_trips_entirely_over_rpc`
  — the issue's headline acceptance criterion, driven over the Unix socket
  with no desktop app running: `sessions/create_chat` with `acp:gemini`, the id
  echoed back unchanged, a fresh `state/get_state` agreeing, replay reachable,
  the unprefixed `gemini` refused, and `opencode` still resolving to the
  built-in. `create_chat` needs no registered adapter, so this is provable now
  rather than after #156's installer exists.
- `store::tests::a_session_of_an_uninstalled_harness_still_loads_with_its_history`
  covers the storage half: the row loads, snapshots, and replays.
- Dispatch rejects a malformed harness id with `invalid_params`; the existing
  `dispatch_validates_params_against_the_contract` already asserted this for a
  bare unknown id and still passes unchanged.
- The existing daemon and client suites pass unchanged, proving the built-in
  path is untouched.

## Smoke Tests

- `cargo test -p bridge-core` — 467 tests passing on `a1c9261` before this
  branch; the count must rise and none may fail.
- `cargo test -p bridge-protocol -p bridge-client -p bridged`.
- `cargo build -p bridge-core -p bridge-protocol -p bridge-client -p bridged`.
- `cargo test -p bridge-deck`. The contract first wrote this off as blocked by
  the missing `binaries/bridged-*` sidecar, but that crate holds the gate that
  asserts `generate_handler![...]` matches the method registry, and it is the
  one place `Harness` is mapped to the `HarnessId` schema reference
  (`src-tauri/src/lib.rs:1464`) — exactly what this change touches. Running
  `sh scripts/prepare-daemon.sh debug` stages the sidecar and the gate runs.
  Nothing is committed: `src-tauri/binaries` is gitignored.
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

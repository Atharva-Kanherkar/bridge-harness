# feat/briefing-policy-conformance — Test Contract

Issue #205, slice 2 of 7 under epic #203. The security gate for every
model-generated Suggested work item. Locked before implementation.

Slice 1 (#204, merged) contracted `WorkBriefLimits` and left the briefing runner
unbuilt. This slice builds the authority that runner will execute under. It does
not select a model, canonicalize connector results, parse tasks, or schedule
anything — those are later slices.

## The shape of the problem

A briefing run reads connected tools with a model in the loop. Everything that
makes an agent useful for coding — a shell, a filesystem, a web fetch, a
subagent — is a liability here, because the text coming back from a connector is
untrusted and may be trying to use them. So briefing authority is not a weaker
`WriteMode`; it is a different axis, and it is a deny-list by construction with
an allowlist of exact tool identities punched through it.

## Functional Behavior

### Briefing authority is its own axis

- `BriefingRuntimePolicy` is a distinct type from `delegation::WriteMode`. A
  start request can carry a write mode, a briefing policy, both, or neither.
- Existing `ReadOnly` / `Shared` / `Isolated` / `Full` behavior is byte-for-byte
  unchanged when no briefing policy is present. Pinned by asserting the existing
  per-adapter permission payloads are identical with the policy absent.
- A briefing policy present and a write mode above `ReadOnly` is a refusal, not
  a merge: briefing never gets a writable tree.

### Default deny

- `supports_briefing` defaults to `false` for every adapter. A newly registered
  adapter that says nothing is unsupported.
- An adapter id not in the built-in contract table is unsupported.
- A provider version that does not match the version the conformance suite was
  recorded against is unsupported.
- A permission representation the policy compiler does not recognize is
  unsupported. Unknown shape is never treated as empty.
- Every one of the above is a `BriefingUnsupported` with a reason a human can
  read, never a silent `false`.

### What is denied

- Built-in tool families, denied by identity and by default: filesystem read and
  write, shell/exec, web fetch and search, skills, subagents, computer use,
  notebook edit, and any tool not on the allowlist.
- All MCP/connector tools are denied until Bridge supplies an exact reviewed
  identity. An identity is `(server, tool)` compared exactly — not by prefix,
  not case-insensitively, not by pattern.
- A tool whose name reads like a read but is not the allowlisted identity is
  denied. `notion_search` allowlisted does not admit `notion_search_and_update`,
  `Notion_Search`, or `notion_search ` (trailing space).
- Denial is terminal for that call and recorded. It never escalates to a prompt.

### What cannot hang a background run

- An approval request, a permission escalation, and an MCP elicitation are each
  answered automatically with a denial, immediately, without waiting for a
  human. A briefing run has no human attached; a prompt that waits is a hang.
- The auto-denial is recorded as a normalized event so the run's transcript
  shows what was refused.

### Limits at the runtime boundary

- Wall time, turn count, tool-call count, per-call argument bytes, and total
  output bytes are enforced by Bridge, not by trusting the provider to respect
  a number in a prompt. Sourced from `WorkBriefLimits`.
- Argument bytes are checked *before* dispatch; output bytes are checked while
  streaming and stop the run when exceeded.
- Every limit breach terminates the run with a typed reason, distinguishable
  from a provider crash and from a clean finish.

### Drift disables rather than widens

- If the tool list a provider presents at run time differs from the one the
  policy was compiled against, briefing is disabled for that run.
- If two normalized tool names collide, briefing is disabled — an ambiguous
  identity cannot be matched exactly.
- Drift never resolves toward more authority.

### Normalized events

- Tool events carry a stable call id and reach a terminal status of success or
  failure. A call that was denied is a failure with the denial reason, not an
  absence.

## Unit Tests

`bridge-core`, `briefing_policy` module:

- `briefing_authority_is_not_a_write_mode` — the two are independent; every
  `WriteMode` round-trips unchanged with no policy present.
- `a_briefing_policy_refuses_a_writable_write_mode`
- `every_adapter_defaults_to_briefing_unsupported`
- `an_unknown_adapter_id_is_unsupported_with_a_reason`
- `a_provider_version_the_suite_did_not_certify_is_unsupported`
- `an_unrecognized_permission_representation_is_unsupported_not_empty`
- `every_builtin_tool_family_is_denied` — filesystem, shell, web, skill,
  subagent, computer-use, notebook, exec.
- `an_empty_allowlist_denies_every_connector_tool`
- `one_exact_allowlisted_identity_is_permitted`
- `a_read_named_mutation_is_denied` — the misleading-name fixture.
- `identity_matching_is_exact` — prefix, case, and whitespace variants denied.
- `duplicate_normalized_tool_names_disable_briefing`
- `a_tool_list_that_drifted_disables_briefing`
- `oversized_arguments_are_refused_before_dispatch`
- `oversized_output_stops_the_run_while_streaming`
- `each_limit_breach_has_its_own_terminal_reason`
- `an_approval_request_is_auto_denied_without_waiting`
- `a_permission_escalation_is_auto_denied_without_waiting`
- `an_mcp_elicitation_is_auto_denied_without_waiting`
- `a_denied_call_reaches_a_terminal_failure_status_with_its_reason`
- `a_denied_call_keeps_its_stable_call_id`

`bridge-core`, adapter capability:

- `claude_declares_briefing_support` — and the suite that certifies it passes.
- `codex_declares_no_briefing_support_with_a_reason`
- `opencode_declares_no_briefing_support_with_a_reason`
- `an_unsupported_adapter_never_falls_back_to_another` — asked for briefing on
  Codex, the answer is a refusal naming Codex, not a silent switch to Claude.
- `a_briefing_start_on_an_unsupported_adapter_is_refused_at_the_boundary`

`sidecar/claude-agent` (node:test). Named as prose, matching the sidecar's
existing tests — `node:test` takes a description, not an identifier:

- "briefing options deny every built-in tool handed down from Bridge"
- "nothing is pre-approved, so no call can bypass the gate" — an `allowedTools`
  entry would skip `canUseTool` and its argument check.
- "briefing options pin strict MCP config and only allowlisted servers"
- "a briefing run inherits no settings, plugins, or dialog capability" — the
  withheld `supportedDialogKinds` is what makes an elicitation unable to park a
  run, since the SDK emits no dialog kind a consumer has not declared.
- "the gate permits exactly one reviewed identity"
- "the gate denies an unknown tool"
- "the gate denies a read-named mutation"
- "the gate denies every built-in family even without the deny-list"
- "the gate refuses oversized arguments"
- "the gate refuses arguments it cannot measure"
- "a decision resolves without awaiting anything outside itself" — the pin that a
  permission decision cannot become a wait for input.
- "a denial carries the tool use id so the call reaches a terminal status"
- "an empty allowlist denies everything"
- "absent briefing config leaves write-mode options untouched"
- "briefing options do not carry a write mode's permissions"
- "a prompt-injected result cannot widen the gate"
- "briefingOptions is usable directly and matches what buildOptions applies"

## Integration / Functional Tests

- One shared adversarial conformance suite, defined once and run against every
  adapter claiming `supports_briefing`. Adding an adapter to that list without
  passing the suite fails the build.
- `the_conformance_suite_gates_the_capability` — flipping a contract entry to
  `supports_briefing: true` without a passing suite result fails.
- Required adversarial fixtures, each its own case:
  - known allowlisted read succeeds
  - unknown tool denied
  - misleading read-named mutation denied
  - schema drift denied
  - duplicate normalized names disable briefing
  - prompt-injected connector result attempting a second tool call — the second
    call is denied and the injection is not followed
  - permission prompt auto-denied
  - MCP elicitation auto-denied
  - timeout terminates with the wall-time reason
  - cancellation terminates cleanly and is not reported as a crash
  - oversized arguments refused pre-dispatch
  - oversized results stop the stream
  - provider crash is distinguishable from a limit breach
  - provider-version mismatch fails closed
  - malformed policy configuration fails closed
- `protocol_mirror` / method-registry gates stay green; no new wire method is
  added by this slice.

## Smoke Tests

- `cargo test --manifest-path src-tauri/Cargo.toml --workspace` green.
- `npm test --prefix sidecar/claude-agent` green.
- `npx vitest run` green.
- `bun run build` green.
- Regenerating protocol artifacts leaves the tree clean.
- `scripts/check-builtin-adapters.sh` still produces its report, now including
  the briefing capability per adapter.

## E2E Tests

N/A for this slice — there is no briefing runner and no UI yet (both explicitly
out of scope). The conformance suite is the closest thing to end-to-end here and
is run in full.

## Manual / cURL Tests

No HTTP surface is added. Manual verification is the conformance report:

```bash
cargo test --manifest-path src-tauri/Cargo.toml -p bridge-core briefing -- --nocapture
```

```bash
npm test --prefix sidecar/claude-agent
```

Expected: every adapter appears with an explicit briefing verdict, Claude
supported and the other two refused with a stated reason.

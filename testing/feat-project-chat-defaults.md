# feat/project-chat-defaults - Test Contract

## Functional Behavior
- Settings exposes application and stable-project scoped new-chat defaults. The initial inventory is launch target (Direct, Orchestrator, enabled specialist), harness/model, advertised effort, existing User approval/Full access posture, current checkout/isolated worktree, and Steer/Queue. Existing preset IDs reference instructions; transient branch operations, secrets, browser grants and attachments are excluded.
- Resolution is explicit draft overrides > project fields > application fields > existing preset/profile/runtime defaults. Inherit differs from false, no specialist, and provider-default model/effort. Validate the coherent target/model/effort combination in the shared Rust core.
- Defaults are versioned database records with revision checks, strict decoding and bounded values. Project ownership comes from the workspace record. No lookup by project name/path/remote. Saving and resolving starts no provider, worktree, browser, or model call.
- New drafts load defaults and show their origin. Switching projects re-resolves inherited fields, preserves edited fields and unsent contents, ignores stale responses, and revalidates scope-dependent choices. Creation returns the exact session identity and snapshots final settings before first dispatch; stale reviewed defaults are rejected.
- Model/effort and access edits stay in the current chat. Explicit save-as-default actions affect future chats only. Existing session snapshots remain unchanged by later defaults or preset edits. Existing special-purpose/worker launches keep their policy.
- Specialist startup preserves existing host dispatch, role, path and isolation gates, including an attachment-only first submission. Mode/agent or checkout transitions use a visible continuation/new-chat boundary with handoff rather than relabeling a live provider.
- Project/chat access uses the existing two public approval postures, independently from filesystem sandbox and network policy. The broader four-mode permission feature is not advertised by this change. Full access in one session cannot change another session. Scoped policy never auto-grants browser/prompt mutation/path-scope approvals. Root launch/resume must carry the explicit effective provider write/approval configuration. Worker authority is bounded by its parent and role. Unsupported restrictive launch configurations fail before spawning.
- Live permission changes stop affected runtimes before persisting a new posture, invalidate pending permissions, and prevent stale grants or resumed provider authority. Legacy sessions retain explicit legacy behavior; new defaults do not copy a global full-access flag into every project.
- Error/rollback behavior never claims persistence or a permission transition succeeded when it did not. Failed creation starts no partially configured runtime. Required checkout isolation never silently downgrades.

## Unit Tests
- Rust chat_defaults: precedence, explicit null/false, grouped model validation, catalog effort validation, strict schema/unknown keys, disabled/deleted presets, project binding, optimistic revision, reset, malformed records, session snapshots, parent authority and missing-session errors.
- TypeScript draft resolver: explicitly edited fields survive retargeting; inherited fields follow project scope; stale async responses do not apply; unsupported targets are diagnosed; agent/attachment submission and exact session selection.
- Settings UI: scope/inheritance/reset, effective source labels, acknowledged save, save failure, project changes and accessible controls.
- Access policy tests: scoped full/ask isolation, no special-approval bypass, concrete adapter parameters and safe runtime transitions.

## Integration / Functional Tests
- Database round trips, reopen/restart persistence, revision conflicts, creation settings committed together, worktree cleanup, immutable existing snapshots, concurrent exact identities.
- Typed protocol/daemon/Tauri and browser/mock parity. Authenticate through existing RPC. Wrong scope/unknown fields/invalid IDs/unsupported values fail before effects. Regenerated artifacts have no drift.
- Model switches, resumes, child dispatch and handoffs preserve scoped authority.

## Smoke Tests
- bun run build, bun run check and bun run test are green before PR.
- Sidecar tests and protocol/core/daemon suites are included in the full test run.
- Start the built frontend, open Settings, save two project defaults, start drafts and confirm visible effective choices.

## E2E Tests
- Real approved browser: application defaults -> project overrides -> new draft -> override -> project switch -> save -> reload; verify no accidental session creation and correct first chat configuration. Test invalid selection/save failure where injectable.
- Host integration tests assert the authoritative path using temporary databases and deterministic fake adapters; no paid provider calls or user credential changes are required.

## Mutation and Fuzz Tests
- Run deterministic seeded property/fuzz tests over layered overrides, malformed/untrusted payloads, scope IDs and model/effort/access combinations. Record seed/case counts and failures.
- Targeted mutation tests deliberately break precedence, revision checks, scope validation and access isolation. Each mutant must be killed by a relevant behavior test. Restore source after each mutant. Record killed/survived/not-tested honestly; this is targeted mutation testing, not a claim of exhaustive coverage.

## Manual / cURL Tests
- Use the authenticated daemon protocol smoke harness with temporary data and exact request IDs. Read defaults -> save app/project -> resolve -> create -> inspect snapshot -> stale save/invalid scope -> read unchanged state.
- Check the UI in the approved browser and record actions/results. No public deployment is required for this desktop/daemon feature.
- PR description lists every command run, exact results, mutation/fuzz evidence, browser checks, and any material unverified environment/provider limitations. No test result is inferred from another test.

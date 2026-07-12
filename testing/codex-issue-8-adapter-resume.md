# codex/issue-8-adapter-resume — Test Contract

## Functional Behavior

- Adapter lifecycle uses typed `StartRequest` and `ResumeRequest`; write mode, model, effort, instructions, provider session ID, and checkpoint context cannot be positionally confused.
- `HarnessAdapter` exposes native-resume capability discovery and explicit `ShutdownReason`.
- Codex capability discovery inspects the installed app-server protocol schema for `thread/resume`; it is never assumed from branding/version alone.
- Codex native resume initializes app-server then sends `thread/resume` with the stored thread ID and current sandbox/approval configuration.
- Claude fresh start uses a newly minted `--session-id`; native resume uses `--resume <stored-id>` and never also sends `--session-id`.
- Restoration order is deterministic: hot process reuse → discovered native resume → checkpoint-restored fresh process → fresh only when no relevant prior context exists.
- A native failure appends `session.resume_failed`, preserves the old provider ID for audit, starts a fresh provider with projected checkpoint context, and stores `checkpoint_restored` only after that fresh start succeeds.
- A fresh session is always labeled `fresh`; no attempted/native-eligible state is exposed as completed native restoration.
- App-start recovery marks live processes stopped, expires their leases, and records separate native/checkpoint eligibility without changing the last honest restoration mode.
- Explicit shutdown reasons are persisted before runtime termination.

## Unit Tests

- Typed request validation and adapter registry dispatch for start versus resume.
- Codex schema capability detection positive/negative and exact `thread/resume` request framing.
- Claude argument construction for fresh versus resume, including mutual exclusion of `--session-id` and `--resume`.
- Restoration controller table covers hot, native success, native failure → checkpoint success, checkpoint failure → fresh, and unrelated fresh.
- Every failure/success stores the matching forest event and restoration mode; no fresh provider ID is written under `native`.
- Migration adds recovery eligibility idempotently and preserves existing session-head restoration modes/provider IDs.

## Integration / Functional Tests

- Existing stopped orchestrator with provider ID attempts native resume; successful provider response retains the thread/session ID.
- Native error records failure and checkpoint-restored fallback receives compact stored context.
- Restart recovery distinguishes sessions eligible for native resume from those requiring checkpoint restoration.
- Ignored/authenticated live tests kill and resume one Codex thread and one Claude session, then verify retained context.

## Smoke Tests

- `bun run test` passes.
- `bun run check` passes.
- `bun run build` passes.
- `git diff --check origin/main...HEAD` passes.

## Acceptance Audit

- Query each session head and verify `restoration_mode` describes the process actually running, not an attempted or merely eligible mode.
- Static audit proves Claude resume cannot mint a new session ID and Codex resume cannot silently call `thread/start`.

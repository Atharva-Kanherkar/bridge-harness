# Reliable, harness-independent composer dictation

Status: scope approved by the user on 2026-09-21; implementation in progress.
Approval includes the opt-in local speech backend. It is not a claim that live
dictation works. Existing uncommitted implementation is preserved.

## Recommendation and scope decision

Use **Omnigent's independent speech service** with **T3 Code's operation and draft
ownership rules**. The coding harness consumes ordinary text only after Send;
it should not determine whether the composer can accept speech.

Recommend an opt-in local transcription backend, initially evaluating
`sherpa-onnx` for streaming. Keep the current Codex transport experimental and
optional, not the universal microphone's prerequisite. Choose the exact local
model only after measuring accuracy, language coverage, footprint, and latency
on supported Macs. No new hosted speech subscription is required by this design.

**Approved scope change:** [issue #651](https://github.com/Atharva-Kanherkar/bridge-harness/issues/651)
explicitly excludes a Bridge-owned STT model. A downloadable local backend changes
that constraint. The user approved the change after reviewing this plan. Model
installation in the shipped app must still require explicit user action.

If the scope is later narrowed to the original constraint, keep the lifecycle, capability, and draft
safety work below, but ship an explicitly limited Codex experimental option.
Independent Codex dictation for other harnesses would need a separately validated
transport/session, explicit provider selection, and verified history/billing
behavior. Do not claim universal voice or silently borrow another login.

## What the reference projects actually do

Sources were inspected at T3 Code commit
`1de563c1491c7d82563e4553bf5bf689ce6adbb9` and Omnigent commit
`e047da5f734e0db77a6d4c32abb3dce433a2e216`. PR states below were checked through
GitHub's API on the research date. These are code/release findings, not local
benchmark results.

| Reference | Verified behavior | What Bridge should take |
| --- | --- | --- |
| T3 Code, merged implementation | Local Apple transcription on supported iOS devices. A shared controller owns the operation; platform code supplies capture and transcription. Current main documentation says environment transcription is not implemented. | Snapshot draft owner, revision, text, and selection; invalidate cancelled operations immediately; reject stale results; never auto-submit. |
| T3 Code, proposed desktop/environment implementation | Desktop PR #8010 is closed and unmerged. Environment PR #8928 is open and unmerged, proposing server-owned `transcribe-cpp`/Moonshine and model management. | Speech service independent of coding harness; explicit, verified model installation. Do not describe this as shipped desktop functionality. |
| T3 Code, proposed web dictation | PR #10195 is open and unmerged. Browser recording posts to an OpenAI-compatible transcription endpoint; optional text cleanup follows. Earlier PR #5213 is closed and unmerged. | Useful recording/error patterns, but not our default: BYOK adds a service and browser-side credential handling is unsuitable for Bridge's native secret boundary. |
| Omnigent, shipped dictation | Optional server-side streaming STT shipped in v0.7.0. Current code uses a standalone dictation endpoint, `sherpa-onnx`, AudioWorklet capture, partial/final events, and stop-tail flushing. Browser Web Speech is another path; Electron prefers the configured server path. | Advertise speech availability independently of chat creation; own one take end-to-end; use streaming partials with explicit finalization and deterministic fake-engine tests. |

T3's relevant sources: [voice contract](https://github.com/pingdotgg/t3code/blob/1de563c1491c7d82563e4553bf5bf689ce6adbb9/docs/internals/voice-input.md),
[controller](https://github.com/pingdotgg/t3code/blob/1de563c1491c7d82563e4553bf5bf689ce6adbb9/packages/client-runtime/src/voice-input/controller.ts),
[Apple binding](https://github.com/pingdotgg/t3code/blob/1de563c1491c7d82563e4553bf5bf689ce6adbb9/apps/mobile/src/native/voiceTranscription.ios.ts),
[merged iOS PR](https://github.com/pingdotgg/t3code/pull/8614),
[desktop proposal](https://github.com/pingdotgg/t3code/pull/8010),
[environment proposal](https://github.com/pingdotgg/t3code/pull/8928),
[web proposal](https://github.com/pingdotgg/t3code/pull/10195).

Omnigent's relevant sources: [release](https://omnigent.ai/releases/0.7.0),
[design](https://github.com/omnigent-ai/omnigent/blob/e047da5f734e0db77a6d4c32abb3dce433a2e216/designs/server-dictation.md),
[capture and transport](https://github.com/omnigent-ai/omnigent/blob/e047da5f734e0db77a6d4c32abb3dce433a2e216/web/src/lib/dictation.ts),
[mic routing](https://github.com/omnigent-ai/omnigent/blob/e047da5f734e0db77a6d4c32abb3dce433a2e216/web/src/components/ComposerMicButton.tsx),
[draft insertion](https://github.com/omnigent-ai/omnigent/blob/e047da5f734e0db77a6d4c32abb3dce433a2e216/web/src/hooks/useDictationInsert.ts),
[engine](https://github.com/omnigent-ai/omnigent/blob/e047da5f734e0db77a6d4c32abb3dce433a2e216/omnigent/server/dictation.py),
[model script](https://github.com/omnigent-ai/omnigent/blob/e047da5f734e0db77a6d4c32abb3dce433a2e216/scripts/fetch-dictation-models.sh).

Do not copy either implementation wholesale:

- Omnigent's default model script selects an English Nemotron streaming model
  described as approximately 650 MB, plus approximately 38 MB of punctuation
  weights. This is not evidence of Hindi/Hinglish accuracy or Bridge performance.
- Browser Web Speech constructor presence does not establish a working backend
  or local processing. Do not use it as Bridge's silent fallback.
- T3's iOS binding is not a drop-in macOS/Tauri implementation.
- Optional LLM cleanup can change identifiers or intent. Leave it out; users
  review the transcript before sending.

## Current Bridge gaps found in source

These findings describe the pre-rework implementation, not necessarily an
installed release. The screenshots alone do not identify the running build.

1. `src-tauri/bridge-core/src/voice.rs` requires an idle, already-live Codex direct
   chat. `src/App.tsx` reduces the capability result to a boolean, and
   `ComposerPill.tsx` hides the mic when false. This explains the missing control
   for Claude and fresh drafts under the current design.
2. `App.tsx` clears its active voice reference on provider error/closed events
   without stopping capture there. Subsequent frames can continue buffering.
3. Async startup uses a shared wanted boolean, not an operation generation and
   owner check after every await. Chat switches and rapid stop/restart can let
   stale startup work interfere with the next recording.
4. Transcript updates rebuild the composer from a start-time base draft while
   editing remains enabled. Later updates can overwrite intervening edits;
   insertion is append-only and trims existing trailing whitespace.
5. `src/voiceCapture.ts` uses ScriptProcessor and resamples each buffer separately.
   It has no cross-buffer fractional position or explicit trailing-sample flush.
   Startup buffering and the serialized append promise chain are not bounded.
6. Backend lease expiry is only checked during another start. There is no complete
   active timeout/terminal cleanup contract. A disappearing runtime between
   capability check and start can leave an inserted lease behind.
7. Codex's voice request function currently returns after writing JSON-RPC to
   stdin, not after a successful provider response. Schema presence and numeric
   version acceptance do not prove authentication, entitlement, successful start,
   or dictation-only behavior.

The existing prerelease parser fix addresses the reported
`0.155.0-alpha.9.2` rejection against a `0.153.4` minimum. It does **not** establish
voice compatibility. The existing test contract's completed lifecycle phase needs
to be reopened when implementation resumes; automated coverage did not cover
these gaps or prove authenticated end-to-end transcription.

## Proposed contract

The flow is microphone capture → independent voice service → draft preview →
explicit insertion/review → normal Send to the selected coding harness.

- `VoiceProvider` selection belongs to Voice settings, independently of the
  selected coding agent. Default to the explicitly enabled local backend; do not
  silently switch between local processing and external accounts.
- Capabilities work before a chat exists. Return typed states such as `ready`,
  `needsSetup`, `unsupported`, and `failed`, with provider, supported locales,
  audio limits, reason, and recovery action. Starting/recording/flushing are
  operation states, not provider availability.
- Local detection checks supported platform, loadable engine, verified model
  files, and model language support. Microphone permission is requested only on
  user action, and failures remain distinct from engine failures.
- Codex detection resolves the actual executable, parses its version, inspects
  the required schema structurally, and validates initialization. Start must
  correlate the request response and ready/error notification with a deadline.
  Cache probes by executable identity/version and relevant configuration, with
  refresh after changes; never equate a successful pipe write with readiness.
- Each take has a voice session ID, client generation, owner key, sequence, and
  explicit terminal outcome. The owner can be a fresh draft, not a provider thread.
  Late callbacks cannot mutate a newer take. Correlate provider events too;
  assigning a fresh Bridge ID to an old thread event is not sufficient.
- Keep one microphone take per app. Use a bounded queue and negotiated duration
  and byte limits. On overflow, stop with an actionable error rather than
  retaining unbounded audio or silently discarding speech.
- Use states `idle → preparing → recording → flushing → review/idle`, plus error
  and cancellation paths. Stop flushes buffered PCM and awaits a terminal result;
  cancel invalidates immediately and cleans up owned resources exactly once.
- Initially display partials in a separate preview. Freeze draft editing and
  submission during capture/finalization, but retain Cancel. Snapshot owner,
  text, revision, and selection before permission requests; validate before
  insertion. External changes cancel insertion rather than restoring stale text.
  Final text replaces only the captured selection; preserve surrounding content.
- Keep the mic discoverable: unavailable states show why and offer setup/retry.
  Click-to-toggle, hold-to-talk, keyboard controls, and `/voice` share one controller.
  Enter during recording stops the take; it must not also send the message.
- Audio is transient, excluded from conversation storage and ordinary logs.
  Avoid logging transcript text. Expose provider, state, timing, byte counts, and
  bounded error codes for diagnosis. Do not request speech before explicit action.

## Delivery in six reviewable parts

### Progress: first safety slice

- Extracted a transport-independent frontend controller with per-take ownership,
  bounded startup/audio queues, readiness/append/finalization deadlines, and late
  callback rejection. A pipe write alone no longer enters the recording state.
- Kept partial transcription in a preview; final insertion checks owner, exact
  draft text, revision, and selection. Draft mutations are tracked even if text is
  edited away and back within a React batch. Send/editing are blocked during a take.
- Added visible unavailable-mic reasons/retry, click/hold/keyboard controls,
  cancellation, and capture cleanup on provider/device errors. Navigating away
  from the composer or hiding the document cancels capture. The currently wired
  Codex path explicitly discloses that it is experimental and sends audio to Codex.
- Fixed the backend start-failure lease leak and bounded encoded input before
  base64 allocation.
- Remaining in Part 1: provider-side correlated request errors, active lease
  expiry, same-provider-thread late-event isolation, and capability cache identity.
  Local engine/service, AudioWorklet capture, model setup, and packaged validation
  remain outstanding. Do not mark the six-part project complete from these changes.

The initial test-file write hit disk exhaustion (about 100 MB free). Removing one
385 MB generated static archive from this worktree restored limited headroom;
source files were not removed. Larger model/runtime evaluation needs additional
space. No speech model was selected, downloaded, or installed by that first slice.

Validation for this slice (2026-09-21): production frontend build passed; all
2,348 frontend tests passed across 195 files; all 2,563 native core tests passed
(11 ignored). Focused controller, capture, hook, and composer coverage includes
58 tests. Existing bundler/React test warnings remain. The full workspace/release
test command and packaged live microphone validation have not been rerun for this
slice. The React review specifically checked stable subscriptions, StrictMode
cleanup, draft ownership, and keyboard access.

### Progress: native engine evaluation

After cleanup approval, the identified Xcode artifact cache was already absent;
this turn deleted no Xcode files. A subsequent check showed 5.8 GiB available,
allowing the native spike to proceed. See the reproducible
[engine evaluation and decision record](voice-engine-evaluation.md).

Verified sherpa-onnx 1.13.8 and the Omnigent English model against release
checksums, compiled a native C ABI benchmark, and completed 18 takes on this M4
without Python, accounts, coding sessions, or microphone access. The two public
speech fixtures matched reference words in the text-inspected four-thread run;
silence stayed empty. One-thread decoding met the provisional throughput budget
with much less aggregate CPU than four threads; peak memory was about 1.1 GiB.
These results establish a viable integration candidate, not live dictation or a
production model choice. The primary model card also corrects the reference
download script's Apache-2.0 claim: the ASR weights name the NVIDIA Open Model
License. Broader accuracy, baseline Macs, and license notices remain release gates.

The spike changed no production runtime dependency, automatic model installation,
or provider-selection behavior. The independent service follows below.

### Progress: independent service foundation (2026-09-22)

- Added a daemon-owned local provider/stream interface and bounded worker service.
  It has no coding-adapter, database, microphone, credential, or download dependency.
  Test-only fake inference proves fresh-draft start, PCM delivery, revised partials,
  finalization, and cleanup without creating a coding session.
- Protocol 1.19 separates `ownerKey` from optional `sessionId`, types provider and
  capability states, discloses on-device versus remote processing, and generates
  the transcript event schema and TypeScript. Older clients and daemons reject
  this changed contract rather than misinterpreting fresh-draft events.
- Added two-frame native backpressure, strict PCM/sequence/byte limits, bounded
  startup/RPC waits, active idle expiry, and take-specific cancellation. Ownership
  remains busy until the engine actually releases; delayed results cannot enter
  a replacement take. Real native-helper kill/reap is still Part 3 work.
- Updated controller/hook ownership checks and partial-replacement semantics.
  Explicit local selection cannot fall back to ready Codex. Capability results
  are invalidated synchronously when owner/provider/context changes. The React
  review retained stable subscriptions and primitive effect dependencies.
- Production local speech intentionally reports `needsSetup`: there is no shipped
  fake engine or automatic model download. The visible app still explicitly uses
  experimental Codex until the provider/settings UI and native helper are wired.

Validation: production frontend build and Rust workspace check passed; 174
protocol tests plus two doctests passed, including generated-artifact and
bidirectional stale-version checks. The focused run passed 64 frontend voice
tests, 22 native voice tests, and 33 event/ACP-event tests. The full frontend run
passed 2,351 tests before three additional controller tests were added and passed
in the focused run. Full native workspace/release tests and live packaged audio
have not been rerun for this milestone. No source/cache cleanup or model download
was required. Commits remain local; no push or PR is implied.

### Part 1 — Immediate safety and honest diagnostics

Files: `src/App.tsx`, `src/voiceCapture.ts`, `ComposerPill.tsx`, core `voice.rs`,
`codex_adapter.rs`, and focused tests.

Fix all terminal paths to release capture; introduce generation/owner guards;
bound queues and deadlines; clean leases on every start failure. Surface the
unavailable reason rather than hiding the mic. Separate version, protocol,
authentication, and voice-start errors. Reopen the lifecycle acceptance checklist.

Acceptance: delayed permission, stop-before-ready, chat switch, runtime failure,
and rapid restart cannot leak a mic, change a different draft, or hang indefinitely.
This part is valid even if the local-engine scope change is declined.

### Part 2 — Engine decision and independent service foundation

The local-backend scope is approved. Evaluate
`sherpa-onnx` streaming first; compare a smaller local option only if footprint or
accuracy fails the agreed bar. [Upstream sherpa documentation](https://k2-fsa.github.io/sherpa/onnx/index.html)
supports macOS and local inference; Bridge integration/packaging still needs proof.

Use consented test recordings covering English prose, code identifiers, paths,
silence, noise, short utterances, and the languages users actually require.
Include Hindi/Hinglish only if those are a product requirement; do not infer
support from the engine name. Record corrections needed, tail loss, cold/warm
latency, peak memory, CPU, download size, model license, and macOS architecture.
Set release thresholds before choosing the model; this plan supplies no invented
benchmark results.

Introduce a daemon-owned `VoiceProvider` abstraction with a deterministic fake
provider. Capabilities no longer require an existing coding session. Evolve the
typed protocol and regenerate schemas/TypeScript using repository tooling;
preserve stale-client/daemon compatibility checks.

Acceptance: a new empty draft can probe/start a fake take without launching Codex,
Claude, or OpenCode. Produce an engine decision record before adding its runtime.

### Part 3 — Local runtime and explicit setup

Add the selected engine behind the abstraction. Integrate through the existing
Rust/native stack rather than requiring a user-managed Python environment.
Evaluate native-library isolation during the spike; any helper process must be
bundled, supervised, cancellable, and independently resource-bounded.

Provide a settings/setup flow with language and size disclosure, explicit download,
pinned checksum, atomic installation, retry, and model removal. Never download on
app launch or a capability probe. Load lazily, serialize model lifecycle changes,
and coordinate removal/shutdown with in-flight inference.

Acceptance: no model is a recoverable setup state; corrupt/interrupted installs
are rejected; inference works offline after setup; audio is not retained.

### Part 4 — Capture and finalization

Replace the capture internals with AudioWorklet after verifying support in the
packaged WKWebView. Use a packaged module compatible with CSP, continuous
resampling, explicit s16le conversion, bounded chunks, and a flush acknowledgement.
If unsupported on the minimum macOS target, document and test a native capture
alternative; do not assume browser support establishes Tauri support.

Keep worklet flush, provider flush, and overall timeout distinct. Stop media tracks
promptly, but retain resources required by noninterruptible inference until it
settles. Test 16/44.1/48 kHz input and release within a partial output chunk.

Acceptance: no lost trailing words in fixtures, bounded memory under slow
consumers, duration limits enforced, and deterministic cleanup on every exit.

### Part 5 — Shared composer experience

Extract orchestration from `App.tsx` into a dedicated controller/hook. Connect
`ComposerPill`, voice settings, shortcuts, and slash dispatch to the same state.
Use existing Tailwind v4 tokens and existing settings surfaces.

Ship a visible mic for fresh and existing drafts regardless of coding harness,
with honest setup/unavailable states. Show partial preview and finalization,
preserve caret selection, allow cancellation, and require normal Send. Keep
inline editable streaming regions as a later refinement, not a prerequisite.

Acceptance: Codex, Claude, and OpenCode composers use the same local dictation
behavior; switching drafts cancels safely; dictation never creates a coding turn.

### Part 6 — Packaged validation and rollout

Use fake-provider integration tests for permission timing, malformed frames,
bounded queues, duplicate/late events, generation changes, failed inference,
no-speech, stop-tail handling, and shutdown. Verify no audio/transcript leaks into
logs or persisted conversation data. Run `bun run build`, `bun run test`, and
generated-artifact checks before any PR.

Manually test the exact packaged macOS build: permission allow/deny/revoke,
input unplug, sleep/wake, cancellation, rapid restart, fresh/existing drafts,
offline inference, cold model load, model removal, and normal explicit Send.
Record executable path/build revision. Keep packaging smoke credential-free and
isolated from signing/notarization secrets. Record real-engine results separately
from fake-provider CI results.

Retain Codex voice behind an experimental setting until an authenticated test
proves request acknowledgement, transcript routing, finalization, and the absence
of unintended coding turns/history effects. Document any provider costs rather
than asserting that using an existing login makes speech free.

Acceptance: published evidence for the chosen engine and target Macs; no
completion claim based solely on a visible icon, successful compilation, or a
minimum-version check. No push, PR creation, or merge is implied by this plan.

## TypeSafe guidance applied

The requested skill and [live design guidance](https://docs.typesafe.ai/concepts/how-to-build-with-system-one)
keep deterministic routing, ownership, and side effects in code. This task needs
no semantic classifier, Jev call, or additional AI cleanup step. Speech recognition
is delegated to the selected STT backend; it does not decide permissions, provider
fallback, draft ownership, or when to send.

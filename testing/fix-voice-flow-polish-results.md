# Voice reliability and UI review results

Reviewed against `testing/fix-voice-flow-polish.md` on 2026-10-13.

## Confirmed bugs and changes

- Startup release kept the microphone running until model readiness. Capture now flushes and stops immediately, retaining even the short tail of a take with no previously emitted frame.
- Previously admitted startup audio could exceed the smaller live queue cap as soon as readiness arrived. Its remaining bytes now drain separately from the bounded additional live backlog.
- The finish timer included healthy audio delivery, so a queue taking more than ten seconds could lose the entire take. Flush, drain, individual append and final completion each have bounded deadlines.
- Local startup withheld the cancellation ID until the model loaded. Admission now returns the ID immediately; readiness and sanitized load failures arrive through the existing transient event stream. A native watchdog preserves the startup deadline and cancellation/lease invariants.
- Public native admission rejected a busy local engine before reaching its existing release wait. Real daemon cancellation/restart testing reproduced the failure; local requests now reach that bounded wait.
- Enter on Cancel finished recording instead of cancelling. Native button activation now works, and the textarea receives focus after voice controls finish.
- Tab could accept a hidden suggestion into a draft owned by dictation. Voice now bypasses composer autocomplete key handlers.
- Space release after focus moved could leave recording active. Shared controls track the release at window level and clear stale holds between takes.
- Setup probes could overlap and apply stale responses after newer status/removal results. Polling is sequential, requests carry a generation, mutations are guarded, and initial status failures expose Retry.
- Silent takes had no feedback, platform microphone errors had no recovery instructions, and identical partials repainted the app. These now produce useful feedback and avoid redundant updates; input-meter updates are capped at five per second.

## UI changes

Provisional speech appears at the captured selection in the writing surface, while the underlying draft remains unchanged until finalization. A compact, achromatic status row replaces the separate provider/preview/instruction stack. Shared controls show distinct loading, recording and finishing states, remain cancellable, and retain click, hold and keyboard interactions.

Voice settings show the setup action and storage disclosure together, keep technical model details collapsed, and offer a microphone test using the production capture/controller/local provider. The test creates no chat and has no Send action. Native progress and Tailwind v4 theme utilities replace the computed-width progress element. Input activity comes from captured PCM, not a decorative animation; animations respect reduced motion.

## Verification

- `bun run build`: passed on final application source; self-contained `voiceCaptureWorklet-*.js` emitted.
- Focused frontend voice/capture/hook/composer/settings suites: 112 tests passed across six files.
- Focused native voice suite: 31 tests passed.
- Full `bun run test`: passed on final application source. Frontend: 3,325 tests across 261 files. Core Rust: 3,081 passed, 17 existing credential-dependent/benchmark tests ignored. Remaining Rust workspace, integration, sidecar, release and documentation suites passed.
- Native helper compiled with `-Wall -Wextra -Werror`.
- Production helper with the installed model: three speech takes (6.625 s, 16.715 s, 6.625 s), every reference word matched after case/punctuation normalization; one second of silence produced no words. Helper readiness measured 0.72–1.19 s in this fixture run. These are model/helper measurements, not microphone-to-screen latency.
- Final real daemon JSON-RPC smoke: passed. Three consecutive speech takes matched all 84 reference words; three immediate startup cancel/restart sequences passed. Take admission returned in 0.14–0.82 ms and cancel RPCs returned in 0.09–0.13 ms in this run. State inspection confirmed zero coding sessions. No credentials or downloads were required. These RPC timings do not measure full helper reaping or microphone latency.
- New controller tests failed on baseline for release/tail capture, preroll transition and premature completion. Composer tests failed on baseline for keyboard Cancel, writing focus and hidden suggestion acceptance. Native startup-ID coverage also failed on baseline. The daemon test caught the public admission bypass missed by the earlier service-only regression.

The full suite uses `NODE_OPTIONS=--no-experimental-webstorage` for the documented Node/jsdom storage incompatibility. The shared Cargo target directory is a build cache only; application source and commits are isolated in this task's worktree.

## Playwright validation

An isolated Chrome for Testing browser exercised the running app. Voice API calls were forwarded to a fresh real daemon with the installed model; a public WAV supplied a virtual MediaStream. Production VoiceCapture, AudioWorklet, resampler, controller, hooks and UI ran unchanged. Other app data used the existing browser-preview fixtures. No signed-in browser or physical microphone was used.

Passed: settings microphone capture/finalization, removal disabled during capture, keyboard Cancel, selected-span insertion with surrounding text preserved, Send blocked during recording, Enter-to-finish, writing focus restored, early pointer release with immediate capture teardown, silent-take feedback, consecutive cancellation, keyboard Space release after focus moved, simulated permission-denial recovery, reduced-motion spinner behavior, single-surface previews, and Graphite/Paper rendering including the 420 px minimum-width layout. Ten opened virtual captures were released; no provider error events or native coding sessions were created.

Screenshots:

- [Recording in Graphite](voice-flow-polish/recording-graphite.png)
- [Voice settings in Paper](voice-flow-polish/settings-paper.png)
- [Voice settings at 420 px](voice-flow-polish/settings-420.png)

## Remaining acceptance and confidence

**Observed accuracy gap:** two complete browser-streamed takes preserved the reference words but inserted an extra “all” after “nightfall.” The raw PCM helper/daemon fixtures matched every reference word. The browser result demonstrates an accuracy limitation along the streamed audio path; its cause is not established by these checks and this PR does not claim to fix it.

Physical microphone recording, actual macOS permission prompts, signed packaged-app WKWebView behavior, and broader accent/noise accuracy still require desktop acceptance. Keep the PR in draft until these checks and the observed accuracy gap are reviewed. Experimental Codex realtime remains disabled; this change uses the existing local English engine.

## Manual acceptance

1. Run `bun install`, `bun run prepare:voice-helper`, and `bun run tauri dev` on this branch.
2. Open Settings → Voice. Verify setup disclosure before installing; with an installation ready, use Try your microphone, speak a short English sentence, finish, review the text, and clear it. Check that no chat or turn is created.
3. Dictate into a new chat and into a selected span of an existing draft. Surrounding text must survive, provisional words must appear once, and no message may send automatically.
4. Hold and release while the model loads, including a take shorter than one capture frame. The microphone indicator must turn off on release; already spoken words must finalize.
5. Cancel startup and immediately start another take. Repeat several times, also cancelling while finishing. No stale words or releasing-engine error should reach the replacement take.
6. Test keyboard Space hold with focus movement, Enter to finish, Tab to Cancel then Enter, and Escape. After completion, Enter in the writing surface sends only reviewed text. Tab during recording must not accept a hidden suggestion.
7. Deny microphone permission, disconnect the input, try silence, and hide/switch the draft while recording. Verify useful recovery, released capture and an intact original draft.
8. Check Voice settings and both composers in Graphite and Paper, at narrow widths and with reduced motion. Confirm readable status, wrapping errors, keyboard focus indicators and stable composer growth.

# fix/voice-flow-polish: Test contract

## Functional Behavior
- Releasing push-to-talk closes microphone capture immediately, including while the engine loads. Flush and retain the tail of short takes; never record beyond release.
- Preserve bounded startup audio while it drains after readiness. Enforce the live queue bound on additional backlog without rejecting previously admitted audio.
- Deliver every captured frame in order before finalizing. Give completion its own deadline after audio delivery; keep startup, flush and append deadlines bounded.
- Never commit previews, late frames, cancelled speech or speech belonging to a different draft. Silence leaves the draft intact with an actionable notice.
- Make click-to-toggle, pointer hold, keyboard hold, Enter-to-finish, Escape-to-cancel and keyboard activation of Cancel work reliably without sending.
- Restore writing focus after dictation; starting and finishing remain cancellable. Show provisional speech in one writing surface with concise status and real input activity.
- Voice settings provide a compact, explicit setup flow and a microphone test with the same production capture and local speech controller. No automatic downloads or sends.
- Prevent stale setup polling from overwriting newer install/remove results; expose load errors with retry and prevent duplicate mutations.
- Preserve Tailwind v4 tokens, app chrome, keyboard accessibility, reduced-motion support and existing non-voice composer behavior.

## Unit Tests
- Controller regressions reproduce early release/tail flush, preroll-to-live draining, delivery vs finish deadlines, stale generations, no-speech completion and cancellation.
- Capture regressions verify permission/device errors, cleanup and audio activity from PCM.
- Hook/composer regressions cover production integration, focus, keyboard hold/release, button activation and inline preview without draft mutation.
- Voice settings regressions cover initial failure/retry, polling ordering, install/remove transitions and microphone test lifecycle.

## Integration / Functional Tests
- Run focused Vitest voice, capture, hook, composer and settings suites.
- Run bun run build and bun run test on the final branch with the repository's documented Node storage workaround if needed.

## Smoke Tests
- Compile the native helper; verify the build includes a self-contained capture worklet.
- Exercise the production helper against installed speech artifacts and a known speech fixture if artifacts are present; report unavailable artifacts explicitly.

## E2E Tests
- Test rendered composer and settings states in a real approved browser when available, including keyboard/pointer controls, light/dark themes and narrow layouts.
- Human microphone quality and signed packaged-app permission acceptance require an interactive desktop; clearly distinguish exercised paths from unverified ones. Do not claim all voice behavior works from mocked tests.

## Manual Tests
- Set up Voice, test the microphone, dictate into a new and existing draft, hold/release during startup, make consecutive takes, cancel during startup/finish, switch chats and send only after reviewing.
- Record actual verification and limitations in testing/fix-voice-flow-polish-results.md. Open a draft PR if real microphone acceptance remains unverified.

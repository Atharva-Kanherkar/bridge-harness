# Voice dictation review contract

## Functional Behavior
- Preserve explicit local dictation and guarded experimental Codex dictation.
- Keep previews separate from drafts; commit only finalized text to the unchanged owner.
- Capture, cancellation, timeout and helper cleanup must release resources.
- Setup/removal must refresh composer availability and prevent concurrent engine mutation.
- Merge current main without losing composer, settings, protocol or packaging changes.

## Unit Tests
- Voice controller, capture, resampler, hook, settings and composer regressions pass.
- Native voice service, helper supervision, downloads and capability gates pass.
- Add regression coverage for confirmed findings before fixing them.

## Integration / Functional Tests
- Run `bun run build` and `bun run test` on the merged branch.
- Protocol generated artifacts match the Rust definitions and handshake version.

## Smoke Tests
- Compile the native voice helper and verify production worklet packaging.

## E2E Tests
- Live microphone verification requires an interactive desktop microphone; report any unexercised path explicitly.

## Manual Tests
- Review stop/cancel ownership, final insertion, setup refresh and helper termination paths.

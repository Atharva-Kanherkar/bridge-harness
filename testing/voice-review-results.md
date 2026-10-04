# Voice dictation review results

Reviewed on 2026-10-04 against `testing/voice-review.md`.

## Changes
- Integrated current main, preserving composer and Settings changes and moving the voice protocol to 1.21.
- Resumed suspended audio contexts and refreshed composer availability after immediate setup completion.
- Preserved empty hypothesis retractions and prevented helper cleanup from blocking on queued frames or pipe writes.
- Serialized model setup/removal with recording admission and prepared the real helper for desktop development.
- Disabled experimental Codex dictation because its reusable thread identity cannot safely correlate successive recordings.

## Validation
- `bun run build`: passed.
- `NODE_OPTIONS=--no-experimental-webstorage bun run test`: passed at `c04803e5`, including 3,175 frontend tests and the Rust workspace. Core Rust reported 2,950 passed and 16 ignored; live integration cases remain ignored.
- Focused voice checks: 83 frontend tests and 28 native tests passed.
- Desktop development launcher: all three checks passed.
- Native helper compiled, and the production AudioWorklet asset was verified self-contained.
- Main's release-only 0.8.0 bump (`7a34250d`) was then merged cleanly. The production build and release consistency suite passed again after that version update; no application source changed in it.

Node 26.3.0 exposes a global web-storage implementation that conflicts with this Vitest/jsdom setup. The scoped Node option above lets the tests use jsdom storage.

## Remaining acceptance checks
Live microphone permission/capture and real-model transcription were not exercised in this review. Authenticated Codex end-to-end validation was not run, and that transport stays disabled. Keep the existing PR in draft until the packaged microphone acceptance checks are completed. Linux packaging requires CI; the older run failed in linuxdeploy.

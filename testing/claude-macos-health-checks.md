# claude/macos-health-checks — Test Contract

Two macOS-specific environment conditions make Bridge look broken while nothing
in Bridge is: a registered project or workspace living inside a TCC-protected
folder (`~/Desktop`, `~/Documents`, `~/Downloads`), and a running binary that is
only ad-hoc signed, so every rebuild invalidates the file-access grants macOS
keyed to the old signature. The health system detects both and explains the
repeated-permission-prompt symptom, pointing at the "macOS file access prompts"
section of `README.md`.

## Functional Behavior

- `health/health` gains a `warnings` array. It is always present (empty when
  nothing is wrong) and existing fields keep their wire names, so protocol 0.6+
  clients are unaffected.
- Each warning carries a stable kebab-case `id`, a human `title`, an actionable
  `detail` that names the repeated-prompt symptom and points to the
  "macOS file access prompts" section of `README.md`, and the offending `paths`
  (empty for the signing warning).
- A registered project path or workspace path inside `~/Desktop`, `~/Documents`,
  or `~/Downloads` produces one `macos-tcc-protected-path` warning aggregating
  every offending path, deduplicated.
- A path merely *named* like a protected folder (`~/DesktopBackup`,
  `~/Code/Documents`) does not warn; the boundary is component-wise under the
  home directory.
- On macOS, `codesign` output for the running executable showing an ad-hoc
  signature or no signature at all produces one `macos-adhoc-signature`
  warning. A stable identity (an `Authority=` chain or a real
  `TeamIdentifier=`) produces none, and unparseable output stays silent rather
  than warning speculatively.
- On non-macOS hosts both checks are inert: `warnings` is empty.
- The app surfaces the warnings in the health banner area with title, detail,
  and offending paths.

## Unit Tests

- Rust (`bridge_core::health`): the TCC boundary predicate accepts descendants
  and the folder itself, rejects siblings, prefix-named folders, relative
  paths, and paths outside home.
- Rust: the codesign parser classifies ad-hoc, linker-signed (also ad-hoc),
  Developer ID, unsigned, and garbage fixtures.
- Rust: warning assembly aggregates and dedupes offending paths, orders
  warnings deterministically, emits nothing when clean, and both warning texts
  mention the repeated-prompt symptom and the README section by name.
- Rust (`bridge-protocol`): `HealthResult` round-trips with a populated
  `warnings` array; a document without `warnings` still deserializes (older
  daemon).
- Rust (`bridge-core` mirror): `api::Health` with a populated warning mirrors
  `wire::HealthResult` exactly.
- Frontend: the health-warnings component renders each warning's title,
  detail, and paths, and renders nothing for an empty list.

## Integration / Functional Tests

- Regenerate protocol schemas and `src/protocol/generated/protocol.ts`;
  `cargo test -p bridge-protocol` proves no drift.
- `bun run check` passes against the regenerated declarations.

## Smoke Tests

- `bun run build` and `bun run test` are green.

## E2E Tests

N/A — the checks read local state and `codesign` output; there is no
multi-process journey beyond the existing health round-trip.

## Manual / cURL Tests

- On a Mac: register a project under `~/Documents`, run a `bun run tauri dev`
  build, and confirm both warnings appear in-app and their guidance matches
  the README section.

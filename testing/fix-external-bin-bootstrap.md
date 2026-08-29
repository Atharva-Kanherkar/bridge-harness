# fix/external-bin-bootstrap — Test Contract

Issue #347. Locked before implementation.

## The shape of the problem

`tauri.conf.json` declares two external binaries:

```json
"externalBin": ["binaries/bridge-browser-host", "binaries/bridged"]
```

`src-tauri/build.rs` stages a placeholder for exactly one of them, hardcoded:

```rust
let path = std::path::PathBuf::from("binaries").join(format!("bridge-browser-host-{target}"));
```

`tauri_build::build()` validates every declared entry, so any cargo invocation that
runs the bridge-deck build script on a checkout that has never been built fails with

```
resource path `binaries/bridged-aarch64-apple-darwin` doesn't exist
```

The prepare scripts each create only their own placeholder, and `beforeDevCommand`
runs `prepare:browser-host:dev` first — which itself invokes cargo, which runs the
build script, before `prepare:daemon:dev` has created the `bridged` placeholder. So
`bun run tauri dev` cannot bootstrap a fresh clone, and neither can a bare
`cargo build --bin bridge-browser-host`.

The fix is for build.rs to read the declared list rather than restate one member of
it. That closes the standalone cargo path too, and means a third sidecar cannot
reintroduce the failure.

## Functional Behavior

### The declared list is the source of truth

- build.rs parses `bundle.externalBin` out of `tauri.conf.json` and stages a
  placeholder for **every** entry that has no file at `<name>-<target>`.
- The list is read, never restated: adding a third entry to `tauri.conf.json`
  requires no build.rs change.
- An entry that already exists on disk is left exactly as it is. Staging must never
  truncate a real sidecar that a prepare script already built.
- Placeholders are created with owner-only permissions on unix, as before.

### Failure is legible, not silent

- A `tauri.conf.json` that cannot be read or parsed, or whose `externalBin` is
  absent or not an array, leaves the build script staging nothing and lets
  `tauri_build::build()` report the real configuration error. build.rs never
  invents an entry and never panics ahead of Tauri's own diagnostics.
- An entry that is not a string is skipped rather than stringified.
- build.rs emits `cargo:rerun-if-changed=tauri.conf.json` so that editing the
  declared list re-runs staging.

### Path handling

- An entry keeps its declared parent directory: `binaries/bridged` stages
  `binaries/bridged-<target>`, and the parent directory is created if absent.
- The target triple comes from `TARGET`, as before. With no `TARGET` in the
  environment the script stages nothing rather than guessing a triple.

## Regression Test

`src-tauri/tests/external_bin_staging.rs`, an integration test of the shell crate,
so it runs after the build script that it is asserting on.

- Reads `bundle.externalBin` from the crate's own `tauri.conf.json`.
- Asserts the list is non-empty and contains more than one entry, so the test cannot
  pass vacuously if the declaration is ever emptied.
- For every entry, asserts a file exists at `src-tauri/<entry>-<target>`, where
  target is the triple the test itself was compiled for.
- Fails on the base commit — `binaries/bridged-<target>` is absent on a checkout
  whose daemon prepare script has not run — and passes after the fix.

## Out of Scope

- Reordering `beforeDevCommand`. It would unblock `tauri dev` alone while leaving
  standalone cargo invocations broken, and re-break on the next added entry.
- Making the prepare scripts stage each other's placeholders. The build script is
  the one place that already runs for every affected invocation.
- Any change to what the prepare scripts build or where they copy it.

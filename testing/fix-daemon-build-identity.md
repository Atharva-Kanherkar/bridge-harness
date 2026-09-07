# fix/daemon-build-identity — Test Contract

The field failure that made two merged PRs look like no-ops: the desktop app
attaches to any live `bridged` daemon that completes the handshake, with no
check that the daemon is running the same code. A daemon started at 14:36
served an app rebuilt at 17:30 for the rest of the afternoon — every Rust fix
merged in between sat on disk unused, while the frontend (served fresh by the
webview) visibly updated. New frontend over a stale backend is worse than
either alone: it makes fixes look merged and broken.

`ServerInfo.version` cannot catch this: it is the workspace version, a
constant `0.1.0` on every dev build.

Locked before implementation.

## Functional Behavior

- **Build identity.** `bridge_core::binary::file_identity(path)` — FNV-1a-64
  over the file's bytes, hex-encoded. Hand-rolled and fully deterministic
  across processes and toolchains (a `DefaultHasher` is not guaranteed either).
  `self_identity()` reads `std::env::current_exe()`; `bridged` computes it
  **once at startup**, before any rebuild can replace the file at that path.
- **The handshake carries it.** `HandshakeResponse` gains
  `buildId: Option<String>` (`#[serde(default, skip_serializing_if)]`), filled
  by `bridged` after `negotiate` succeeds. Optional so older daemons and other
  clients interoperate; protocol artifacts regenerated, drift test green.
- **The launcher enforces it.** `daemon_host::Launcher::ensure`, on a
  successful attach, compares the daemon's reported build id with the identity
  of the binary the launcher would spawn:
  - equal → attach, as today;
  - different, or absent (a pre-buildId daemon is stale by definition) →
    log loudly, stop the daemon, spawn the launcher's binary, attach to that.
  - The launcher without a binary of its own (attach-only) accepts any daemon:
    it has nothing better to offer.
- **Stopping a daemon it did not spawn.** The launcher SIGTERMs the pid in the
  data directory's lease file — only when the recorded owner `kind` is
  `daemon`; an `embedded` owner is an app and is never signalled. `bridged`
  already drains gracefully on SIGTERM. New
  `ownership::current_holder(data_dir)` reads the holder identity without
  taking the lock.
- **No restart wars.** One replacement attempt per `ensure` call: after the
  launcher spawns its own binary, whatever that daemon reports is accepted
  (mismatch there is logged, not re-fought). Two concurrently running apps of
  different builds sharing one data dir will each restart toward their own
  build; that configuration is already unsupported (documented, logged, not
  defended).

## Unit Tests

- `bridge-core binary`: `file_identity` is stable for identical content across
  separate reads, differs when one byte differs, and errors (not panics) on a
  missing path.
- `bridge-core ownership`: `current_holder` reads the holder of a held lease
  without releasing or corrupting it; returns `None` for an unheld directory.
- `bridged`: the handshake response carries the configured build id; a config
  without one (tests, embedded reuse) omits the field.
- `daemon_host`: the staleness decision is a pure function
  (`daemon_is_stale(ours, theirs)`) — table-tested: equal ids fresh, differing
  ids stale, absent daemon id stale, launcher without a binary never stale.
- `daemon_host` (process test, following the crate's existing fake-binary
  tests): attaching to a live daemon reporting a different build id terminates
  it and serves from a fresh spawn of the launcher's binary.

## Integration / Smoke

- `bun run check`, `bun run test`, `bun run build` all green.
- Protocol artifacts regenerated; `cargo test -p bridge-protocol` (drift) green.

## E2E

N/A — desktop app.

## Manual Tests (reviewer)

1. Run the app (spawns daemon A). Rebuild `bridged` (any code change).
   Relaunch the app → console logs the stale daemon replacement; the new
   daemon's pid differs; behavior matches the new build immediately.
2. Kill the daemon mid-session → supervisor respawns it (unchanged behavior).
3. `bridge exec --json` (bridge-client without a daemon binary) still attaches
   to whatever daemon is serving.

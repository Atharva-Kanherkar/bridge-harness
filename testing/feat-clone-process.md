# feat/clone-process — test contract

Locked before implementation. Part 1 of the browser-clones epic: a throwaway
browser process and its lifecycle in `bridge-core`, macOS only. A clone is a
Chrome or Brave process on a temporary profile that lives on a RAM disk, is
driven over `--remote-debugging-pipe`, can be handed session-only cookies, and
is destroyed (process killed, volume ejected) or recovered after a crash. Out of
scope on purpose: where cookies come from, any leak guard, any UI, any wire
method.

## Shape of the thing

`src-tauri/bridge-core/src/browser_clone.rs` defines `CloneSupervisor`,
separate from `BrowserBridgeSupervisor` (which supervises the user's own
attached tab). It owns:

- `spawn_clone()` — mount a RAM disk under a Bridge-owned temp dir, launch the
  browser on a profile inside it, wait for it to answer over the pipe, return a
  `CloneInfo` or a typed `CloneError` within `ready_timeout`.
- `load_session(clone_id, Vec<CookieSpec>)` — inject cookies with CDP
  `Storage.setCookies` and no expiry, so they are session cookies: readable
  in-page, never written to the profile's `Cookies` database. CDP's input type
  has no `session` field, so "session: true" is achieved by omitting `expires`
  and is *proven* by reading the cookies back and requiring `session: true`.
- `destroy(clone_id)` — kill the browser's process group, reap it, eject the
  volume, remove the mount point, drop the ledger record.
- `sweep_orphans()` — at core boot, before the core serves, kill any recorded
  clone process that survived a crash and eject its volume.

The ledger (`<data_dir>/browser-clones.json`) records only `pid` and `mount`
per clone. Never cookies, never a session id, never a URL.

Wiring: `BridgeCore` gains `browser_clones: Arc<CloneSupervisor>`; `boot()` runs
`sweep_orphans()` before returning. Everything is behind
`#[cfg(target_os = "macos")]`, including the `BridgeCore` field.

## 1. Launch arguments — `browser_clone::tests`

| # | Behaviour | Assertion |
|---|---|---|
| 1.1 | The debugging transport is the pipe | args contain `--remote-debugging-pipe` and `--user-data-dir=<profile>` |
| 1.2 | A TCP debugging port is never requested | no arg starts with `--remote-debugging-port`, and none is `--remote-debugging-address` |
| 1.3 | The guard refuses a port flag before any launch | `validate_launch_args` returns an error for `--remote-debugging-port=9222` and for a bare `--remote-debugging-port` |
| 1.4 | Headless is opt-in | `--headless=new` appears only when `CloneConfig.headless` is true |
| 1.5 | No keychain prompt, no first-run UI | args contain `--use-mock-keychain` and `--no-first-run` |
| 1.6 | The HTTP cache cannot fill the RAM disk | args carry `--disk-cache-size` below half the default volume |

## 2. CDP over the pipe — `browser_clone::tests`

| # | Behaviour | Assertion |
|---|---|---|
| 2.1 | Requests are NUL-terminated JSON | a fake browser reading the pipe sees `{"id","method","params"}` terminated by `\0`; the result is returned |
| 2.2 | A CDP error becomes a typed error | an `error` response yields `CloneError::Cdp` carrying the method and message |
| 2.3 | A silent browser times out | `call` returns `CloneError::Timeout` after the deadline, and the pending slot is freed |
| 2.4 | A dead browser fails pending calls | EOF on the pipe yields `CloneError::BrowserExited`, for in-flight and later calls |
| 2.5 | Frames are bounded | `read_frame` rejects a frame larger than the limit instead of buffering it |

## 3. Session cookies — `browser_clone::tests`

| # | Behaviour | Assertion |
|---|---|---|
| 3.1 | Cookies are sent as session cookies | the `Storage.setCookies` params have camelCase keys (`httpOnly`, `sameSite`) and contain no `expires` |
| 3.2 | Values never reach `Debug` output | `format!("{:?}", cookie)` does not contain the value |
| 3.3 | Invalid cookies fail before the wire | empty name, empty domain, relative path, `SameSite=None` without `secure` each yield `InvalidCookie` and send nothing |
| 3.4 | Injection is verified | after `Storage.setCookies`, `Storage.getCookies` is called and every cookie must read back with `session: true` |
| 3.5 | A persistent read-back fails closed | a fake browser answering `session: false` yields `CookieNotSession` |
| 3.6 | A dropped cookie fails closed | a cookie missing from the read-back yields `CookieRejected` |
| 3.7 | Unknown clone | `load_session("nope", ...)` yields `UnknownClone` |

## 4. Lifecycle — `browser_clone::tests` (fake browser script + directory-backed volume)

A perl script stands in for the browser: it speaks the pipe protocol on fd 3/4
and records the argv it actually received. The volume backend is a plain
directory, so these run without `hdiutil`.

| # | Behaviour | Assertion |
|---|---|---|
| 4.1 | `spawn_clone` returns a ready clone | `CloneInfo` has a live pid, a mount under the mounts dir and the product string from `Browser.getVersion` |
| 4.2 | The real child got the pipe and no port | the argv the child recorded contains `--remote-debugging-pipe` and no `--remote-debugging-port` |
| 4.3 | Not ready in time is a typed, cleaned-up error | a silent browser with a 300 ms budget yields `NotReady`, returns within 5 s, leaves no process, no mount, no ledger record |
| 4.4 | Early exit is a typed, cleaned-up error | a browser that exits at once yields `BrowserExited`, no mount, no ledger record |
| 4.5 | No browser is a typed error | with no candidates, `spawn_clone` yields `NoBrowser` and creates nothing |
| 4.6 | `destroy` kills and ejects | after `destroy`: the pid is gone, the mount path does not exist, the ledger is empty; a second `destroy` yields `UnknownClone` |
| 4.7 | `destroy` reaches helper processes | a helper the browser forked in its process group is dead after `destroy` |
| 4.8 | The ledger holds only pid + mount | the JSON keys of every record are exactly `pid` and `mount`, and the file does not contain a cookie value loaded into the clone |
| 4.9 | Dropping the supervisor cleans up | a live clone is destroyed when the last `Arc<CloneSupervisor>` drops |
| 4.10 | A failed volume create leaves nothing | a backend that fails `create` yields a `Volume` error and no ledger record |
| 4.11 | The browser inherits no descriptor beyond the pipe | a descriptor this process holds *without* close-on-exec (placed at fd >= 200) is reported closed by the child; mutation-checked: removing the scrub makes this fail |

## 5. Orphan sweep — `browser_clone::tests`

A "crash" is simulated by writing the ledger and starting a sacrificial process
whose command line carries the mount path, then sweeping from a fresh supervisor.

| # | Behaviour | Assertion |
|---|---|---|
| 5.1 | A recorded orphan is killed and its volume ejected | the process is dead, the mount path is gone, the ledger is empty, the report lists both |
| 5.2 | A recycled pid is left alone | a live unrelated process recorded under the mount is *not* killed (its command line does not mention the mount), while the mount is still ejected and the record cleared |
| 5.3 | A volume-only record (`pid: null`) is ejected | mount gone, record cleared, nothing killed |
| 5.4 | A tampered record cannot reach outside the mounts dir | a record whose mount is elsewhere is dropped, that directory is untouched, and nothing is ejected |
| 5.5 | A failed eject keeps the record | with a backend whose eject fails, the record stays for the next boot and the report lists the failure |
| 5.6 | No ledger, no work | a missing ledger yields an empty report and creates neither a ledger nor a mounts dir |
| 5.7 | Live clones of this process are not swept | a running clone's record is skipped |
| 5.8 | Core boot sweeps before returning | `runtime::tests` — `BridgeCore::boot` on a data dir holding a stale record kills the orphan, removes its mount and empties the ledger |

## 6. Real browser — `browser_clone::tests::live` (env-gated)

Runs only when `BRIDGE_CLONE_LIVE=1`; skips (prints why, passes) when no
Chrome, Brave or Chrome for Testing is installed, or the variable is unset.

| # | Behaviour | Assertion |
|---|---|---|
| 6.1 | A real clone starts on a real, invisible RAM disk | `spawn_clone` returns; the live process' `ps` command line has `--remote-debugging-pipe`, no `--remote-debugging-port`, and its profile under the mount; the mount table shows the volume `nobrowse,nosuid,nodev` at the Bridge-owned path and nothing under `/Volumes` |
| 6.2 | The session cookie is readable in-page | after `load_session`, `document.cookie` on a local page contains the cookie |
| 6.3 | While the clone runs, the session cookie is not in the on-disk `Cookies` file | the bytes of `<profile>/Default/Cookies` do not contain the session cookie's value during operation |
| 6.4 | `destroy` is the boundary: RAM disk and profile are gone | `destroy` SIGKILLs the process group (no graceful flush) then ejects; the mount path and `profile_dir` no longer exist and `hdiutil info` no longer lists the volume |
| 6.5 | No browser helper outlives `destroy` | no process's command line still names the mount (renderer, GPU and network service died with the browser) |
| 6.6 | Same checks on every engine | `BRIDGE_CLONE_BROWSER` points the live test at Brave or Chrome for Testing; all three pass |

**Finding (verified against real Chrome 154, 2026-09-29):** session cookies are kept in memory *while the browser runs* but modern Chromium flushes them to the profile `Cookies` database on shutdown (for session restore). So the disk guarantee does **not** rest on cookie residence — it rests on the RAM disk. `destroy` kills the whole process group (so nothing is flushed on a graceful exit) and ejects the volume; the profile and anything flushed into it die with the RAM disk and never reach the real disk. The module doc and epic #749 wording were corrected to say this.

## Review findings folded in

Found while reviewing the first draft against a real browser; each has a test above.

- **Teardown sent SIGTERM first.** `adapters::terminate_process_group` terminates gracefully, and a graceful Chromium exit is exactly when session cookies get flushed into the profile. Clones now `killpg(SIGKILL)` at once and wait with a zombie-aware check (6.4).
- **Inherited descriptors.** Nothing stopped a library's non-close-on-exec handle (SQLite, PTY, socket) from reaching the browser. The child now marks every descriptor above 4 close-on-exec before exec, which keeps std's exec-error pipe working (4.11).
- **Finder flash.** `diskutil erasevolume` mounted the new volume under `/Volumes` before it was moved, and the final mount was browsable. It is now formatted with `newfs_hfs` without mounting and mounted `nobrowse,nosuid,nodev`, root `0700` (6.1).
- **PATH-resolved tools.** `hdiutil`, `diskutil`, `newfs_hfs` and `ps` are run by absolute path.
- **Cache size.** A 256 MiB RAM disk with an uncapped HTTP cache fills on a media site; the cache is capped at 64 MiB (1.6).

## 7. Repo gates

- `bun run check` (tsc -b + cargo check --workspace) is green; the crate still
  compiles on non-macOS (Linux CI runs `cargo check` and `cargo test` over the
  workspace, where this module does not exist).
- `cargo test --manifest-path src-tauri/Cargo.toml -p bridge-core` is green.
- `bun run build` is green.
- No protocol artifact changes: no wire method is added.

## Manual

```text
BRIDGE_CLONE_LIVE=1 cargo test --manifest-path src-tauri/Cargo.toml -p bridge-core \
  --lib browser_clone::tests::live -- --nocapture
hdiutil info | grep -c BridgeClone     # 0 after the run: nothing leaked
```

Explicitly **not** changed: `BrowserBridgeSupervisor` and the attached-tab
extension flow; the embedded Tauri webview browser; the protocol and every
generated artifact; the frontend.

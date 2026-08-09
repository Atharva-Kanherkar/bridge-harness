# feat/acp-registry-sync — Test Contract

First PR for #156 (ACP marketplace backend), section 2: **registry sync**. Everything else
in #156 — `AcpAdapter`, the installer, auth — is out of scope here and lands separately.

## Scope boundary

**In:** catalog types, tolerant parsing, on-disk cache, conditional HTTP fetch, staleness.

**Out, deliberately:**
- No `CoreEvent` variant and no notification. That requires the protocol `NotificationName`
  registry, which is #157 (RPC layer). This PR exposes a `refresh` entry point; #157 schedules
  it and publishes the event.
- No installing, no launching, no auth. Reading the catalog only.
- No mirroring or re-hosting. Bridge reads the upstream index and stores a local cache of it.

## Functional Behavior

Source of truth: `https://cdn.agentclientprotocol.com/registry/v1/latest/registry.json`
(Apache-2.0, public CDN, no auth). Top-level shape is `{ version, agents: [...], extensions: [] }`.

1. **Parse is tolerant, not strict.** A malformed or unrecognized agent entry is *skipped and
   counted*, never fatal. One bad entry must not void the catalog. (Same lesson as #144, where
   one bad row fails a whole replay page.)
2. **Optional fields are genuinely optional.** Verified against live data: `sha256` is absent on
   real binary entries (Devin has none), and `icon` / `website` are inconsistent across entries.
   The published FORMAT.md and the live index do not perfectly agree — the parser follows the
   data, not the doc.
3. **Three distribution types**, each a single-key object under `distribution`:
   - `npx` → `{ package }`, optional `args`
   - `uvx` → `{ package }`, optional `args`
   - `binary` → map of platform target → `{ archive, cmd, args?, sha256?, env? }`
4. **Six platform targets**: `darwin-aarch64`, `darwin-x86_64`, `linux-aarch64`, `linux-x86_64`,
   `windows-aarch64`, `windows-x86_64`. An unknown target string is skipped, not fatal.
5. **Conditional fetch.** Send `If-None-Match` when a cached ETag exists. `304 Not Modified`
   reuses the cache without reparsing.
6. **Offline serves last-good cache, marked stale.** A fetch failure with a cache present is not
   an error — it returns the cached index flagged stale. A fetch failure with no cache is an error.
7. **Cache write is atomic.** Write to a temp file and rename, so an interrupted write cannot
   leave a corrupt cache that poisons the next start.
8. **A corrupt cache file is recoverable**, not fatal — it is discarded and treated as absent.
9. **Never blocks.** No network call happens implicitly; callers drive `refresh` explicitly.

## Unit Tests

Module under test: `bridge_core::acp_registry`.

- `parses_the_live_index_fixture` — the checked-in fixture parses, yielding the expected agent
  count with zero skips.
- `parses_npx_uvx_and_binary_distributions` — one agent of each type; `uvx` args survive; binary
  platform map is keyed correctly.
- `binary_entry_without_sha256_is_valid` — Devin-shaped entry parses with `sha256: None`.
- `entry_missing_optional_icon_and_website_is_valid` — minion-code-shaped entry parses.
- `skips_malformed_entry_and_keeps_the_rest` — an index of 3 agents where one lacks a required
  field yields 2 agents and 1 recorded skip naming the offending id/index.
- `skips_unknown_platform_target_without_failing_the_entry` — an entry with one bogus platform
  key keeps its valid platforms.
- `skips_unknown_distribution_type_as_one_entry` — a `distribution` Bridge doesn't understand
  skips that agent, counted, rest intact.
- `rejects_index_with_no_agents_key` — a document without `agents` is an error, not silent empty.
- `platform_target_for_current_host_resolves` — the running platform maps to a known target.
- `binary_build_for_missing_platform_is_none` — an agent with no build for the host reports none
  rather than panicking or picking a wrong arch.
- `cache_round_trips` — write then read returns an identical index plus its ETag.
- `corrupt_cache_is_discarded_not_fatal` — garbage bytes on disk read as "no cache".
- `cache_write_is_atomic` — no partial file is observable under an interrupted write; a temp
  artifact does not survive a successful write.
- `stale_cache_is_served_when_the_fetch_fails` — fetch error + cache present → cached index,
  `stale = true`.
- `fetch_failure_without_cache_is_an_error` — fetch error + no cache → `BridgeError`.
- `not_modified_reuses_cache_without_reparsing` — a 304 path returns the cached index and does
  not rewrite the cache file.

## Integration / Functional Tests

- `refresh_populates_an_empty_cache_dir` — against a local HTTP server (not the live CDN),
  a first refresh writes a cache file and returns a non-stale index.
- `second_refresh_sends_if_none_match` — the recorded request carries `If-None-Match` matching
  the stored ETag, and a 304 response leaves the cache file's mtime unchanged.

**No test hits the live CDN.** Network tests use a local `tiny_http` server (already a
`bridge-core` dependency) so CI is deterministic and offline-safe.

## Smoke Tests

- `cargo build -p bridge-core` succeeds.
- `cargo test -p bridge-core acp_registry` passes.
- `cargo clippy -p bridge-core` reports no new warnings for the new module.

## E2E Tests

N/A for this PR — there is no user-facing surface until #157 (RPC) and #158 (UI). The
end-to-end "browse → install → sign in → run a turn" journey is #158's acceptance.

## Manual Tests

A one-shot check against the real index, run by hand (not in CI):

```bash
curl -s https://cdn.agentclientprotocol.com/registry/v1/latest/registry.json | head -c 400
```

Expected: a document beginning with `{"version":"1.0.0","agents":[`. Confirms the fixture still
matches the upstream shape. If upstream changes shape, `parses_the_live_index_fixture` is the
test that should be updated, deliberately, in its own commit.

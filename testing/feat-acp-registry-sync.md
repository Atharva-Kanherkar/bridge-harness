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
2. **Optional fields are genuinely optional.** Measured over the captured fixture (38 agents):
   `id`, `name`, `version`, `description`, `authors`, `license`, `icon`, `distribution` present
   on all 38; `repository` on 32; `website` on 31. `sha256` on only 48 of 90 binary builds.
   The published FORMAT.md and the live index do not agree — FORMAT.md lists `repository` and
   `website` as required, and the data disagrees. **The parser follows the data, not the doc.**
3. **`distribution` is a set, not a choice.** An agent may offer more than one method —
   verified: `kilo` and `sigit` each ship both `binary` and `npx`. The parser models every
   method the entry offers; *selecting* one is the installer's job (#156 §3), not this PR's.
   - `npx` → `{ package }`, optional `args`, optional `env`
   - `uvx` → `{ package }`, optional `args`, optional `env`
   - `binary` → map of platform target → `{ archive, cmd, args?, sha256?, env? }`
4. **Six platform targets**: `darwin-aarch64`, `darwin-x86_64`, `linux-aarch64`, `linux-x86_64`,
   `windows-aarch64`, `windows-x86_64`. An unknown target string is skipped, not fatal.
   Coverage is uneven and "no build for your platform" is a common state, not an edge case —
   `windows-aarch64` appears on only 8 of the 17 binary-distributed agents.
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

- `parses_the_live_index_fixture` — the checked-in 38-agent fixture parses with zero skips.
- `parses_npx_uvx_and_binary_distributions` — one agent of each type; `uvx` args survive; binary
  platform map is keyed correctly.
- `agent_offering_both_npx_and_binary_keeps_both` — `kilo`/`sigit` shape; neither method is
  silently dropped by an enum that can only hold one.
- `binary_entry_without_sha256_is_valid` — Devin-shaped entry parses with `sha256: None`.
- `entry_missing_optional_repository_and_website_is_valid` — parses with both `None`.
- `npx_entry_with_env_is_valid` — `env` is carried on npx/uvx, not only on binary builds.
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
- Atomic write, split into the two halves that are actually observable:
  `successful_write_leaves_no_temp_artifact` and `a_failed_write_leaves_the_previous_cache_intact`.
  The third property — no torn read when the process dies mid-write — is a guarantee of
  `rename(2)`, not something a unit test can assert, so it is stated in the code rather than
  pretended in a test.
- `stale_cache_is_served_when_the_fetch_fails` — fetch error + cache present → cached index,
  `stale = true`.
- `fetch_failure_without_cache_is_an_error` — fetch error + no cache → `BridgeError`.
- `second_refresh_sends_if_none_match_and_a_304_reuses_the_cache` — a 304 returns the cached
  catalog and leaves the cache file **byte-identical** (asserted on contents, not mtime, which
  is coarse and filesystem-dependent).
  **A 304 does reparse the cached document, deliberately.** The cache stores upstream's verbatim
  bytes rather than our parse of it, so that a later parser improvement re-understands
  previously-skipped entries without a network round trip. Reparsing 48 KB is not a cost worth
  trading that for.

## Integration / Functional Tests

- `refresh_populates_an_empty_cache_dir` — against a local HTTP server (not the live CDN),
  a first refresh writes a cache file and returns a non-stale index.
- `second_refresh_sends_if_none_match_and_a_304_reuses_the_cache` — the recorded request carries
  `If-None-Match` matching the stored ETag; the cold-cache request carries none.
- `an_upstream_error_status_falls_back_instead_of_caching_it` — a 5xx degrades to the cache and
  does not overwrite it.
- `an_unparseable_response_does_not_overwrite_a_good_cache` — the response is parsed *before* the
  cache is replaced, so a bad upstream publish cannot cost the user their last-good catalog on
  top of the failed refresh.
- `a_cache_recorded_against_a_different_url_is_not_reused` — a cache whose `sourceUrl` does not
  match is neither served nor used to supply a revalidation header.

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

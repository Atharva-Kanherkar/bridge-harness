# feat/claude-fable-5-2 Test Contract

Claude Fable 5.2 (`claude-fable-5-2`) is out. The Agent SDK that ships it is
0.3.322 (Claude Code 2.1.322). Claude models are discovered live from the
pinned SDK, so support is a pin bump plus prices. The same SDK also moves the
`haiku` alias to Claude Haiku 5.5 (`claude-haiku-5-5`), so Haiku 5.5 must be
priced in the same change or every Haiku turn reports `unpriced`.

## Functional Behavior

- The Agent SDK pin is `0.3.322` in `sidecar/claude-agent/package.json`,
  `runtimes/claude/package.json`, both `package-lock.json` files, `bun.lock`,
  and `managed_runtime::CLAUDE_SDK_VERSION`. No `0.3.284` pin remains outside
  test fixtures.
- With the 0.3.322 SDK the sidecar catalog (`{"catalog":true}`) lists
  `fable -> claude-fable-5-2 "Fable 5.2"` and `haiku -> claude-haiku-5-5
  "Haiku 5.5"`. `claude-fable-5-1`, `claude-fable-5` and
  `claude-haiku-4-5-20251001` stay selectable.
- Bridge parses those rows to ids `claude-fable-5-2` (label "Fable 5.2") and
  `claude-haiku-5-5` (label "Haiku 5.5"), each with the five effort levels
  low, medium, high, xhigh, max.
- Tier inference: Fable 5.2 is Strong, Haiku 5.5 is Fast.
- `usage_rates.json` prices, in micro-USD per million tokens:
  - `claude-fable-5-2`: input 8,000,000, output 40,000,000, cacheRead 200,000,
    cacheWrite 10,000,000 (Anthropic pricing page: $8 / $40, hits $0.20,
    5m writes $10).
  - `claude-haiku-5-5`: input 100,000, output 500,000, cacheRead 10,000,
    cacheWrite 125,000 (base tier, prompts up to 100K tokens).
- Rates are regenerated through `usage_pricing::project_litellm_document`, not
  hand-written. Every rate key in the previous snapshot is still present
  (retired models stay priced for history).
- The browser mock's managed Claude runtime reports `pinnedVersion: "0.3.322"`.
- Known limit, unchanged: pricing is base tier only, so Haiku 5.5 prompts over
  100K tokens are priced at the lower tier (documented in `usage_pricing.rs`).

## Unit Tests

- `adapters::tests::the_strongest_and_fastest_models_each_provider_ships_infer_their_tier`
  covers `("claude-fable-5-2", "Fable 5.2") -> Strong` and
  `("claude-haiku-5-5", "Haiku 5.5") -> Fast`.
- `claude_adapter::catalogue_tests::the_fable_5_2_catalog_resolves_aliases_to_new_releases`
  (new): the real 0.3.322 rows for `default`, `fable`, `haiku` and
  `claude-fable-5-1` parse to the ids, labels and effort ladders above, and
  Fable 5.1 stays a distinct entry.
- `usage_pricing` bundled-rate test: `lookup_rate("claude-fable-5-2[1m]")` is
  8,000,000 input / 40,000,000 output / 200,000 cache read; one million input
  tokens on `claude-fable-5-2` price at 8,000,000 micro-USD;
  `lookup_rate("claude-haiku-5-5")` output is 500,000.
- Existing `managed_runtime` tests that compare against `CLAUDE_SDK_VERSION`
  still pass.

## Integration / Functional Tests

- `cargo test -p bridge-core` for `adapters::`, `claude_adapter::`,
  `usage_pricing::`, `managed_runtime::` passes.
- `node --test` sidecar suite passes on the new lockfile.
- A script compares old and new `usage_rates.json`: no key removed, the two new
  keys present with the values above.

## Smoke Tests

- Clean `npm ci` of `runtimes/claude` installs `@anthropic-ai/claude-agent-sdk`
  0.3.322, and `node sidecar/claude-agent/index.mjs '{"catalog":true}'` against
  that install lists Fable 5.2 and Haiku 5.5.
- `claude --model fable -p "Reply with OK" --output-format json` on Claude Code
  2.1.322 reports `modelUsage` for `claude-fable-5-2`.

## E2E Tests

N/A. No UI code changes: the composer picker renders the live catalog. Covered
by the manual steps below.

## Manual / cURL Tests

1. Build the app from this branch. In Settings, Harnesses, Claude shows an
   Update to 0.3.322 on an install that has an older managed runtime.
2. After updating, the composer model picker lists "Fable 5.2" under Strong and
   "Haiku 5.5" under Fast.
3. A one-line turn on Fable 5.2 completes and the usage view prices it (not
   "unpriced").

## Gates

`bun run check`, `bun run build`, and `bun run test` green, or any failure shown
to also fail on untouched `origin/main`.

# Menu Bar

The Menu Bar is an independent product surface inside Bridge. Its presentation
is native SwiftUI hosted in AppKit menu rows. Bridge's Rust backend remains the
only authority for authentication, provider collection, history, pricing and
persistence. The Codex vertical slice is the first implementation.

## Ownership

| Module | Owns |
| --- | --- |
| `src-tauri/bridge-menu-bar/` | Native alpha-mask template icon, status item, menu, SwiftUI presentation, formatting |
| `src-tauri/src/menu_bar/` | Desktop lifecycle, main-thread dispatch, presentation refresh and action queue |
| `src/features/menu-bar/` | General → Menu Bar preferences and their view state |
| `bridge-core/src/codex_adapter/account.rs` | Bounded account/limit queries through Bridge's Codex runtime |
| `bridge-core/src/usage_overview.rs` | Versioned quota snapshot, collection coalescing, aggregation over the existing ledger |
| `bridge-core/src/menu_bar.rs` | Centralized preference persistence |
| `bridge-protocol/src/messages/usage_overview.rs` | Contract consumed by Swift and the existing desktop presentation adapter |

The native package imports no Tauri, credentials, provider SDK, database, or
Bridge UI. It accepts bounded UTF-8 JSON and returns action IDs through a C ABI.
All AppKit operations stay on the main thread. Swift compiles into a static
library using Xcode, targeting macOS 12 on both arm64 and x86_64. No separate
application or provider daemon is embedded by this package.

Settings compose Bridge's ordinary Tailwind v4 settings primitives. Menu
presentation imports neither UsageWidget nor MeterReadings. Main-window
integration is limited to startup, opening Settings/Bridge, and a data adapter
that preserves the existing quota presentation.

## Data contract

Protocol 1.8 adds `usage/get_usage_overview`, `usage/refresh_usage_overview`,
`menu_bar/get_menu_bar_settings` and `menu_bar/save_menu_bar_settings`.
Both snapshots and settings carry `schemaVersion: 1`. Older daemons fail the
handshake before a menu attempts to use missing methods.

Numeric metrics carry independent value, source and status fields:

- Sources: reported, measured, estimated. Missing observations have no source.
- Status: current, stale, unavailable.
- A reported zero is a real numeric zero. Missing values remain null.
- Passed reset times, failed reads and old observations become stale. A reset
  never proves that an account has used zero since resetting.
- Unpriced records make the corresponding cost unavailable, including mixed
  priced/unpriced totals. Tokens still count. Model-priced amounts are estimates.
- Reasoning tokens are part of output and are not added twice.
- Quota percentages are never converted into an invented token budget.
- Reported window duration identifies session and weekly quotas, including
  accounts whose only weekly limit occupies the provider's primary slot.
  Automatic status-item selection prefers a current session, then a current
  weekly limit. An explicit selection stays unavailable when that window is
  missing.
- Costs and token totals cover records on this Mac, potentially across accounts;
  the account heading identifies the quota account, not a billing attribution
  for the device-wide history.

Account refresh starts a bounded read-only Codex app-server probe, using the
same executable/credential selection as Bridge sessions. It starts no chat or
turn. Account identity is checked before and after the limit read; a change
rejects the result. The probe is killed and reaped on success, failure or timeout.
Public quota observations are stored in the existing configuration store;
credentials never cross the UI contract.

Refreshes are serialized and coalesced by the backend. The native host schedules
requests according to saved preferences and renders completion truthfully.
Snapshot-only reads re-evaluate expiry and ledger data without provider queries.
Codex history uses the existing bounded incremental importer and deduplication.
The menu's preference controls its own scheduled requests. The visible main
window retains its existing polling cadence and can also update the shared
observation; hiding or disabling the menu does not disable main-window usage.

## Cutover and roadmap

The native menu is the default macOS surface. `BRIDGE_LEGACY_METER=1` retains the
previous meter internally for parity checks; native initialization failures also
fall back to it. Keep that internal route until native appearance, keyboard,
focus, signing and supported-OS checks are complete. Its retirement must preserve
the existing in-app reading components while removing menu-specific sharing.

1. Codex: template icon; used/remaining and spend modes; session/weekly windows;
   reset countdowns; account/plan; model/token/cost breakdown; Refresh and
   Settings; General → Menu Bar preferences.
2. Claude: account/source selection, quota semantics, costs, fixtures and parity.
3. Cursor: credential/session sources, credits and token-availability semantics.
4. OpenCode: provider identity, local usage and cost coverage.
5. Additional CodexBar providers as independent central-backend adapters.

Hooks and user-authored plugins are a separate future project. They require
their own lifecycle, authority and execution contracts.

## Reference

Visual organization and native behavior are inspired by
[CodexBar](https://github.com/steipete/CodexBar), reviewed at
`7fdc17636f161ab410d8a6a0e8f45b6a595cf8d2`. This feature does not import CodexBar's
Swift application, provider authentication stack, or updater.

See [validation notes](menu-bar-validation.md) for completed checks and the
remaining native parity gates before retiring the legacy meter.

## Overview and customization

Protocol 1.10 adds independent `quotaDisplayMode` (defaults to used),
`openToOverview`, `iconStyle`, a bounded `statusLayout`, and `showHistory`.
Existing icon-text preferences survive migration; quota bars no longer inherit
the icon-text display mode. A fully consumed allowance is a full used bar, with
remaining percentage below. Sub-1% values retain precision rather than becoming
zero. Quota bars have fixed guide breaks at 50% and 75% of their displayed
width, using CodexBar's punched slot and neutral center stripe. These positions
stay fixed in both used and remaining modes. All stale/unavailable protections
still apply.

Overview is the first native tab and the default opening surface. It shows only
provider headers, quota bars, reset countdowns, freshness and connection errors.
Provider tabs contain billing amounts, compact Today and Last 30 days token/cost
summaries, and sparse daily history with selectable per-day totals. Detailed
model breakdowns stay in their submenu. Missing days and unpriced costs
stay unknown. Codex rolling limits use their reported duration (`5-hour`); named
additional pools retain provider labels. Pace is an elapsed-time estimate,
explicitly distinct from a separate quota or credit inventory.

General → Menu Bar includes a status layout editor inspired by CodexBar's token
compositor. Users add and reorder icon, provider, used/remaining percentages,
5-hour/weekly percentages, reset, cost, spaces and separators across two lines.
Presets and JSON copy/paste use the same bounded, non-executable token contract
(two lines, twelve items per line, one icon). A standard display remains
available. While a custom layout is active, the standard display selector says
Custom layout. Choosing a standard display clears that layout immediately;
Use standard display in the editor restores the saved standard choice.
The Bridge logo and quota-meter icon are both macOS alpha templates.
Refresh interval is also available directly in the native menu. Open Bridge and
Menu Bar Settings remain the navigation actions; there is no CodexBar app,
updater, animation system, notification system, or iCloud dependency.

Daily history keeps its metric and selected day independently for each provider,
including when switching tabs or reopening the menu. Metric changes reuse the
loaded snapshot within a fixed-height chart; native menu measurements are deferred
and coalesced outside SwiftUI updates. The backend caches the shared history
aggregate until database contents, local date, or timezone changes. Snapshot time,
quota expiry, and freshness are still computed on every presentation update.

Protocol 1.11 separates user-initiated provider refresh from background refresh.
When default Claude Code credentials are unavailable or expired, a manual Refresh
can read `/usage` through the installed Claude CLI in Bridge's private probe
directory. The probe disables tools, hooks, plugins, and MCP, has a bounded
capture deadline, and never answers login, trust, or permission prompts. Explicit
OAuth tokens and custom Claude configuration directories remain authoritative;
they are never replaced by another CLI account. Background collection does not
launch this fallback. CLI observations are labelled `Claude CLI`; absent account,
plan and reset information remain unavailable.

## Favorite providers and overflow

Protocol 1.12 persists `pinnedProviders`: up to three unique provider IDs in
display order, defaulting to Codex, Claude, and Cursor. General → Menu Bar lets
users choose each position. Selecting an existing favorite swaps its position;
choosing None removes it. The backend validates the list and older clients do
not pair with a daemon that would discard the preference.

Overview stays fixed beside a horizontally scrollable provider strip. Three
providers fit at once; arrow buttons and horizontal scrolling reveal additional
providers without increasing the 350-point menu width. Favorites lead the list,
followed by other enabled providers. The currently supported collectors remain
Codex, Claude, Cursor, and OpenCode; scroll capacity does not imply new adapters.
Navigation arrows sit at opposite edges with matching 6-point outer gutters and
4-point gaps beside the tab group; three equal provider slots fit between them.
The menu card scrolls vertically without a visible scrollbar. Provider changes
use a brief layer opacity fade after the deferred layout update, respecting
macOS Reduce Motion without animating menu geometry or replacing the open menu.

The status item's usage, custom layout, and template meter always belong to the
first favorite. Detail-tab selection is independent. With no favorites, the
first enabled provider supplies status; a disconnected first favorite remains
explicitly disconnected rather than silently showing another account's usage.

Favorites and account connections are independent. A favorite remains visible
while disconnected, with a Settings action. It never renders cached quota or
account history until its account usage is enabled. Pinning, selecting, and
scrolling do not enable collectors. Existing disabled choices survive migration.

Claude model-specific weekly limits are distinct from its all-model weekly
quota. The API's `limits[]` records with `group: weekly` and `kind: weekly_scoped`
provide the model identity, display name, used percentage, and reset. Following
CodexBar, `is_active: false` does not discard a reported Fable allowance. Stable
model IDs prevent duplicates; all-model totals and malformed entries are excluded
from these scoped rows. A Fable-only account can show its reported limit even if
the ordinary five-hour or weekly fields are absent. The manual CLI fallback reads `Current week
(<model>)` sections, including Fable, with strict section/redraw boundaries.
Only reported limits are shown; a missing Fable row is not manufactured from
Opus usage or treated as zero.

Enabled Cursor accounts also read the optional Grok Bot included allowance from
`/api/dashboard/get-sand-usage-status`, using the existing Cursor session. This
adds a separate Grok Bot quota and its own reset/duration when Cursor reports an
included allowance. The request has a five-second timeout; failure or an absent
allowance leaves the main Cursor usage available. Missing quota is never inferred
from Grok model token history.

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

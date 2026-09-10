# Menu Bar provider expansion

Scope: Codex, Claude, Cursor, and OpenCode. Provider presentation stays in the native
Menu Bar package; account collection, pricing, history, and persistence remain
in bridge-core. The existing Codex snapshot API remains a compatibility view
for the main window. A grouped versioned snapshot supplies the native switcher.

## Provider sources

- Codex: existing bounded account/rate-limit probe and local usage ledger.
- Claude: Claude Code OAuth credentials, read-only usage/profile endpoints,
  session/weekly/scoped limits, and the existing local history ledger.
- Cursor: read-only Cursor desktop authentication, dashboard usage-summary,
  billing-cycle limits, and reported plan/on-demand amounts. Recorded Bridge
  tokens remain a separate local ledger; missing Cursor history is not zero.
- OpenCode: existing local history importer for models/tokens/cost. An optional
  connected Zen web account supplies rolling/weekly quota or monthly spend and
  prepaid balance. Local model API keys do not imply a Zen subscription.

The provider switcher and settings choose the displayed provider independently
from enabling collection. Disabled providers do not start scheduled probes.
Freshness and collection failures are tracked per provider. Failed identity
reads clear account labels while retaining historical values as stale.

Cursor percentage fields already use percent units. Cents convert to USD;
OpenCode monthlyUsage/balance use fixed-point 1e8 USD units while monthlyLimit
is in whole USD. Missing limits never become a fabricated remaining percentage.

## Authentication and availability

New providers are disabled by default when migrating existing preferences.
Enable them individually in General → Menu Bar. Cursor uses its desktop session;
Bridge does not scrape browser cookies. Claude reads its explicit OAuth token,
configured credential file, or the Claude Code Keychain item without background
authentication prompts. Expired credentials require sign-in again.

Connect OpenCode opens a separate HTTPS sign-in window with no Bridge IPC
capabilities. Only first-party auth cookies from an opened workspace are saved
in macOS Keychain by the active backend (embedded or daemon). This keeps
Keychain ownership with the process that refreshes the session. The authenticated
local RPC only writes the session; it provides no credential-read method. API keys are never treated as Zen session cookies. The
workspace ID can be overridden in settings; it is not a credential. Dashboard
server-function IDs can change upstream and return an unavailable state.

Cursor local history import, Claude credential refresh, OpenCode Go, browser
cookie import, hooks and user plugins are outside this provider slice.

## Reference

Reviewed CodexBar at `7fdc17636f161ab410d8a6a0e8f45b6a595cf8d2`:

- [Claude OAuth usage](https://github.com/steipete/CodexBar/blob/7fdc17636f161ab410d8a6a0e8f45b6a595cf8d2/Sources/CodexBarCore/Providers/Claude/ClaudeOAuth/ClaudeOAuthUsageFetcher.swift)
- [Cursor authentication](https://github.com/steipete/CodexBar/blob/7fdc17636f161ab410d8a6a0e8f45b6a595cf8d2/Sources/CodexBarCore/Providers/Cursor/CursorAppAuth.swift)
- [Cursor usage](https://github.com/steipete/CodexBar/blob/7fdc17636f161ab410d8a6a0e8f45b6a595cf8d2/Sources/CodexBarCore/Providers/Cursor/CursorStatusProbe.swift)
- [OpenCode fetcher](https://github.com/steipete/CodexBar/blob/7fdc17636f161ab410d8a6a0e8f45b6a595cf8d2/Sources/CodexBarCore/Providers/OpenCode/OpenCodeUsageFetcher.swift)
- [OpenCode billing units](https://github.com/steipete/CodexBar/blob/7fdc17636f161ab410d8a6a0e8f45b6a595cf8d2/Sources/CodexBarCore/Providers/OpenCode/OpenCodeZenBillingParser.swift)

Provider transport and parsing were adapted from the referenced MIT-licensed
implementation. Its [license notice](third-party/CodexBar-LICENSE.txt) is retained.

Implementation validation is recorded in `menu-bar-validation.md`.

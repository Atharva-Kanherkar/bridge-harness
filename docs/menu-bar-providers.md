# Menu Bar provider expansion

Scope: Codex, Claude, Cursor, and OpenCode. Provider presentation stays in the native
Menu Bar package; account collection, pricing, history, and persistence remain
in bridge-core. The existing Codex snapshot API remains a compatibility view
for the main window. A grouped versioned snapshot supplies the native switcher.

## Provider sources

- Codex: existing bounded account/rate-limit probe and local usage ledger.
- Claude: a no-turn Claude Agent SDK probe asks Claude Code for its account and
  structured 5-hour/weekly/model-scoped limits. Claude owns credential lookup and
  renewal. The existing local history ledger stays separate. Explicit OAuth
  profiles and older SDKs retain the legacy read-only usage/profile reader; a
  manual Refresh can use the installed CLI's `/usage` as a compatibility fallback.
- Cursor: read-only Cursor desktop authentication, dashboard usage-summary,
  billing-cycle limits, reported plan/on-demand amounts, and a bounded dashboard
  event window for daily model/token details. Dashboard account history remains
  separate from the device-local ledger and the main Usage screen.
- OpenCode: existing local history importer for models/tokens/cost. An optional
  connected Zen web account supplies rolling/weekly quota or monthly spend and
  prepaid balance. Local model API keys do not imply a Zen subscription.

The provider switcher and settings choose the displayed provider independently
from enabling collection. Disabled providers do not start scheduled probes.
Freshness and collection failures are tracked per provider. Failed identity
reads cannot reuse another account's limits. Claude quota failures clear the
unverified account snapshot; local token history remains independent.

Cursor percentage fields already use percent units. Cents convert to USD;
OpenCode monthlyUsage/balance use fixed-point 1e8 USD units while monthlyLimit
is in whole USD. Missing limits never become a fabricated remaining percentage.

## Authentication and availability

New providers are disabled by default when migrating existing preferences.
Enable them individually in General → Menu Bar. Cursor uses its desktop session;
Bridge does not scrape browser cookies. Claude's default profile uses an isolated
Agent SDK control request, with no user prompt, hooks, tools, plugins, MCP servers,
or persisted session. `skipBehaviors: true` also skips transcript scanning.
Initialization and usage each have a 15-second deadline; the Rust parent caps
the entire process at 35 seconds and terminates its process group. No credential
material leaves Claude Code. Only account metadata and quota windows cross the
sidecar boundary. Explicit OAuth tokens/configuration directories remain
authoritative. A missing or incompatible SDK falls back to the legacy reader;
SDK authentication/network failures cannot silently select another credential.

An explicit Refresh can probe Claude Code with tools, hooks, plugins and MCP
disabled. It waits for the CLI's normal prompt before sending only `/usage`,
aborts authentication/trust/permission prompts, and terminates/reaps its process
on completion, timeout or Bridge shutdown. Scheduled refreshes do not launch
this fallback. CLI observations identify their source and omit unavailable
identity/reset metadata. Bridge neither rewrites nor mints Claude credentials.

Cursor history uses the same verified desktop session as its quota request.
Complete, bounded pagination is required before totals are published. Exact
adjacent-page duplicates are reconciled only against the provider's event count.
`tokenUsage.totalCents` is provider-reported API-rate value, distinct from
`chargedCents` deducted from the plan; the UI keeps account billing separate.
Unpriced/invalid records cannot turn a partial sum into a complete cost. Account
changes cannot reuse another account's history, and missing history stays
unavailable. The local Cursor importer remains Unsupported because its local
stores do not report token counts.

Connect OpenCode opens a separate HTTPS sign-in window with no Bridge IPC
capabilities. Only first-party auth cookies from an opened workspace are saved
in macOS Keychain by the active backend (embedded or daemon). This keeps
Keychain ownership with the process that refreshes the session. The authenticated
local RPC only writes the session; it provides no credential-read method. API keys are never treated as Zen session cookies. The
workspace ID can be overridden in settings; it is not a credential. Dashboard
server-function IDs can change upstream and return an unavailable state.

Cursor local history import, Bridge-owned Claude OAuth refresh, OpenCode Go, browser
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

The dashboard history follow-up was reviewed against the user's local CodexBar
revision `928166f899471bbdcb72210641cdec91324d0154`, especially
`Sources/CodexBarCore/Providers/Cursor/CursorUsageEventsFetcher.swift`. Claude
terminal readiness and bounded collection were checked against
`Sources/CodexBarCore/Providers/Claude/ClaudeStatusProbe.swift` and the installed
CLI's screen-reader output.

Implementation validation is recorded in `menu-bar-validation.md`.

The default Claude SDK collector follows [T3 Code's capability probe](https://github.com/pingdotgg/t3code/blob/main/apps/server/src/provider/Layers/ClaudeProvider.ts).
Bridge's pinned SDK 0.3.261 exposes an experimental structured usage method;
runtime checks and fixture tests cover missing methods and changed response shapes.
The deployed default-profile path was checked against Claude Code 2.1.276.

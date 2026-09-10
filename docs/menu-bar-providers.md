# Menu Bar provider expansion

Scope: Codex, Cursor, and OpenCode. Provider presentation stays in the native
Menu Bar package; account collection, pricing, history, and persistence remain
in bridge-core. The existing Codex snapshot API remains a compatibility view
for the main window. A grouped versioned snapshot supplies the native switcher.

## Provider sources

- Codex: existing bounded account/rate-limit probe and local usage ledger.
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

## Reference

Reviewed CodexBar at `7fdc17636f161ab410d8a6a0e8f45b6a595cf8d2`:

- [Cursor authentication](https://github.com/steipete/CodexBar/blob/7fdc17636f161ab410d8a6a0e8f45b6a595cf8d2/Sources/CodexBarCore/Providers/Cursor/CursorAppAuth.swift)
- [Cursor usage](https://github.com/steipete/CodexBar/blob/7fdc17636f161ab410d8a6a0e8f45b6a595cf8d2/Sources/CodexBarCore/Providers/Cursor/CursorStatusProbe.swift)
- [OpenCode fetcher](https://github.com/steipete/CodexBar/blob/7fdc17636f161ab410d8a6a0e8f45b6a595cf8d2/Sources/CodexBarCore/Providers/OpenCode/OpenCodeUsageFetcher.swift)
- [OpenCode billing units](https://github.com/steipete/CodexBar/blob/7fdc17636f161ab410d8a6a0e8f45b6a595cf8d2/Sources/CodexBarCore/Providers/OpenCode/OpenCodeZenBillingParser.swift)

Implementation and validation are in progress.

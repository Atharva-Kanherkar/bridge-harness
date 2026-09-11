# CodexBar architecture and Bridge adaptation

Reviewed the user's local CodexBar checkout at
`928166f899471bbdcb72210641cdec91324d0154` on 2026-09-11. The earlier provider
adapter review remains pinned separately in [provider sources](menu-bar-providers.md).

## How the relevant pieces work

| CodexBar layer | Responsibility | Bridge counterpart |
| --- | --- | --- |
| `CodexBarCore`, provider descriptors and fetch plans | Resolve an enabled provider's source, fetch bounded account data, and return typed usage with source/account provenance | `bridge-core::provider_usage` and the versioned usage snapshots |
| `UsageStore` | Collect in the background, retain per-provider state, coordinate refreshes, publish usage and optional enrichments | `bridge-core::usage_overview` and the native host worker |
| `SettingsStore` | Persist provider choices, presentation preferences and refresh cadence | Central backend menu settings, edited through General → Menu Bar |
| `StatusItemController` and menu extensions | Own status items, provider selection, menu tracking, invalidation and hosted content | `bridge-menu-bar` Swift package and `src/menu_bar` host |
| `MenuCardView` and menu descriptors | Present each provider's account, limits, resets and supplemental costs | Independent `MenuCard`, formatting and model submenus |
| `IconRenderer` | Draw an 18-point alpha-mask image and mark it as an AppKit template | Bridge app-mark geometry with a transparent background and `isTemplate = true` |

Its application, CLI and widgets consume a shared core. Provider transport does
not belong in a menu view. Account quotas and local token-cost scans are distinct
inputs; source errors and unavailable values retain their meaning.

## Open-menu switching

The packaged Bridge check exposed a selected Claude segment above a Cursor card.
The worker had delivered updates through Tauri's normal main-thread event queue,
which can wait while AppKit owns the nested `NSMenu` tracking loop.

CodexBar addresses this explicitly in
[`ProviderSwitcherTrackingRunLoopScheduler`](https://github.com/steipete/CodexBar/blob/928166f899471bbdcb72210641cdec91324d0154/Sources/CodexBar/StatusItemController%2BProviderSwitcher.swift):

1. Queue the operation in both `eventTracking` and `default` run-loop modes.
2. Share a one-shot operation between those blocks, so either mode can deliver
   it and the other becomes a no-op.
3. Wake the main run loop. Avoid recursively pumping the tracking loop.
4. Reconcile the selected provider's content and close obsolete child menus.

Bridge adapts that scheduler in `MenuRunLoop.swift`. The Rust host now uses it
for presentation delivery while retaining its existing Objective-C and Rust
exception boundary. The owned callback context is released once. Provider
changes refresh the model submenu, and opening that submenu reads the current
snapshot instead of retaining another provider's breakdown.

The native regression check exercises delivery during tracking, normal-mode
fallback, and exactly-once behavior with either mode first. The fixture, quota
semantics and template-icon checks run alongside it.

## Compatibility and scope

This CodexBar checkout targets Swift 6.2 and macOS 14. Bridge retains its macOS 12
target by adapting the AppKit/Core Foundation mechanism in its existing Swift
layer. Its backend continues to own authentication, provider collection,
pricing and persistence. The main Bridge conversation/usage components are
outside the native presentation package.

The reused implementation retains the [CodexBar MIT notice](third-party/CodexBar-LICENSE.txt).
The local CodexBar checkout is a source reference and remains unchanged.

Additional references inspected: `docs/architecture.md`, `docs/ui.md`,
`docs/refresh-loop.md`, `ProviderFetchPlan.swift`,
`StatusItemController+ProviderNavigation.swift`,
`StatusItemController+MenuRefreshScheduling.swift`,
`StatusItemController+MenuTracking.swift`, and
`Tests/CodexBarTests/StatusMenuSwitcherTrackingTests.swift`.

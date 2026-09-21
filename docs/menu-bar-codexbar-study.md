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
snapshot instead of retaining another provider's breakdown. If that child is
already open, Bridge defers structural changes until its next opening. AppKit's
`cancelTracking()` ends the entire menu session even when called on a child, so
Bridge does not use it to reconcile an in-flight provider update.

The native regression check exercises delivery during tracking, normal-mode
fallback, and exactly-once behavior with either mode first. A separate native
menu regression exercises multiple updates while the breakdown is open and
checks that reopening uses the latest provider and visibility preferences.
The fixture, quota semantics and template-icon checks run alongside it.

## Native card sizing, appearance, and accessibility

The follow-up comparison at the same CodexBar revision identified a viewport
bug: Bridge resized the SwiftUI document after a tracked update but left the
enclosing menu row at the height measured when it opened. A short loading card
could therefore retain a short viewport after its full usage arrived.

`CostHistoryMenuScrollView.swift` and
`StatusItemController+MenuPresentation.swift` make the measured viewport an
explicit intrinsic/fitting size, then update the frame, invalidate sizing and
tile the scroll view. Bridge adapts that boundary in `MenuCardScrollView.swift`,
remeasuring every snapshot because usage can change at the same width. It caps
the viewport on the current screen, preserves and clamps distance from the top
for both coordinate orientations, and resets to the top for a new provider.
The scroll view keeps a transparent background and an overlay vertical scroller
only when needed. The hosted item stays attached while tracking.

`StatusItemController+MenuAppearance.swift` preserves the exact effective
appearance object, including accessibility attributes. Bridge pins that object
across the root and model submenus. Following `MenuCardMenuItem`, its hosted row
has a blank fallback title and suppresses AppKit's parallel selection highlight.

`MenuBarLayoutRenderedTitle` and `applyMenuBarLayoutContent` use a common
rendered payload for visible text and VoiceOver. Bridge's `MenuStatus` similarly
supplies the title, tooltip, provider/window/value description, and explicit
stale/unavailable wording. Local cost freshness remains metric-specific:
`usage_overview.rs` and CodexBar's `MenuCardView+Costs.swift` both separate local
token-cost snapshots from account quota errors. An expired account session
does not make freshly computed local usage stale.

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

## Overview, quota direction, and status layout

The follow-up Sol source review used CodexBar's menu-card/overview presentation,
`MenuBarLayout.swift`, `MenuBarLayoutRenderer.swift`, `MenuBarLayoutEditor.swift`,
`StatusItemController+MenuBarLayout.swift`, and `CodexBarCore/UsagePace.swift`.
Bridge adapts the independent used/remaining preference and the native token
compositor. Its bounded layout supports two lines, provider/window percentages,
reset, cost, space, separator and icon tokens, with presets and JSON copy/paste.
Stacked or inline images and text become a single AppKit alpha template so
highlighted and accessibility appearances recolor the complete status item.

Bridge's Overview deliberately contains only quota and freshness information.
Daily token/cost history and per-model details belong to provider tabs. Cursor
preserves its distinct total, Cursor and third-party lanes and billing amounts.
Codex limits use reported durations and named pools. The pace estimate compares
elapsed window time with usage; reserve wording describes that estimate, not an
additional pool of tokens. Bridge does not import CodexBar's expression language,
notifications, iCloud, plugins, hooks or updater.

Claude manual recovery follows `ClaudeStatusProbe.swift` and the TTY runner's
bounded `/usage` capture pattern. Bridge uses its existing Rust PTY dependency,
keeps provider collection in core, and refuses prompt consent or account
substitution. The probe runs only for a user-initiated refresh when default
credentials cannot be read; background refresh retains the direct collection
path. CLI-only snapshots explicitly identify their source and do not invent
account names, plans or reset times.
## Provider colors and text switcher · 2026-09-12

The native menu now adapts CodexBar's text-only switcher from
`StatusItemController+SwitcherViews.swift` and `ProviderSwitcherButtons.swift`
at `928166f`: 30-point buttons, 6-point corners, system accent selection,
secondary unselected labels, uniform widths measured in both toggle states,
and a minimum one-point gap. Its 16-point outer grid relaxes to 10 or 6 only
when needed for the full five-tab row. Bridge's fixed provider order is
Overview, Codex, Claude, Cursor, OpenCode; disabled providers remain hidden.
The host owns callbacks and accessibility, while the provider snapshot remains
the only data authority. The adaptation includes the existing MIT notice.

`ProviderStyle.swift` uses the exact descriptor palette: Codex #49A3B0,
Claude #CC7C5E, Cursor #00BFA5, OpenCode #3B82F6. Quota track/fill drawing is
adapted from `UsageProgressBar.swift` and `MenuHighlightStyle.swift`, using a
single static Canvas and a six-point rounded bar. Detail charts use the same
provider accents. Status-item artwork remains an alpha template for macOS
tinting. Bridge retains used/remaining settings, fractional precision, and
unknown/stale semantics; no CodexBar account store or collector is imported.

The native regression suite checks the real AppKit button layout, visible
labels, equal widths, non-overlap, Cursor selection, stable frames across
selection, and changed provider lists. Synthetic production-card renders were
inspected in light and dark appearances with all providers enabled, including
Cursor's full used bars. These use fixture data and are not live account reads.

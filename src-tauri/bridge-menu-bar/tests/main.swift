import AppKit
import Foundation
import SwiftUI

func check(_ condition: @autoclosure () -> Bool, _ message: String) {
    if !condition() { fatalError(message) }
}

let zero = Metric(value: 0, source: "reported", status: "current")
check(quotaPercentLabel(0.36) == "0.36%" && quotaPercentLabel(99.64) == "99.64%", "Fractional Cursor usage must not look empty or exhausted")
check(quotaPercentLabel(0.001) == "<0.01%", "A small positive percentage must not become reported zero")
check(countLabel(zero) == "0", "Reported zero must remain zero")
check(countLabel(Metric(value: 27_933_293, source: "measured", status: "current")) == "27,933,293", "Large token counts must be readable")
check(compactCountLabel(Metric(value: 27_933_293, source: "measured", status: "current")) == "28M", "Summary token counts stay compact")
check(moneyLabel(zero) == "$0.00", "A reported zero cost is valid")
check(countLabel(.unavailable) == "Unavailable", "Unknown tokens must not become zero")
check(moneyLabel(.unavailable) == "Unavailable", "Unpriced usage must not become $0")
check(moneyLabel(Metric(value: 2_300_000, source: "estimated", status: "current")) == "≈$2.30", "Estimated costs need a qualifier")
let compactPeriod = UsagePeriod(
    tokens: Metric(value: 18_400, source: "measured", status: "current"),
    costMicrousd: Metric(value: 1_240_000, source: "estimated", status: "current"),
    models: [])
check(usageSummaryValues(compactPeriod, showTokens: true, showCost: true) == "18K tokens · ≈$1.24",
      "A summary combines labeled tokens and spend without repeated rows")
check(usageSummaryValues(compactPeriod, showTokens: true, showCost: false) == "18K tokens",
      "Token-only summaries retain their unit")
let unavailablePeriod = UsagePeriod(tokens: .unavailable, costMicrousd: .unavailable, models: [])
check(usageSummaryValues(unavailablePeriod, showTokens: true, showCost: true) == "Tokens unavailable · Cost unavailable",
      "Unavailable summary values identify which metric is missing")
check(Metric(value: 42, source: "reported", status: "stale").current == nil, "Stale quota cannot drive a current percentage")
check(countdown(3_700, now: Date(timeIntervalSince1970: 100)) == "1h 0m", "Reset countdown uses seconds")
check(moneyLabel(Metric(value: 0, source: "reported", status: "stale")) == "$0.00 · stale", "Historical costs retain freshness")

// Rust round-trips this same fixture against its generated wire contract.
let fixture = try JSONDecoder().decode(Presentation.self, from: Data(contentsOf: URL(fileURLWithPath: CommandLine.arguments[1])))
check(fixture.settings.schemaVersion == 1 && fixture.settings.displayMode == "remaining", "Settings wire names must agree")
check(fixture.selectedUsage?.windows[0].usedPercent.current == 0, "Reported zero survives Rust to Swift")
check(fixture.selectedUsage?.windows[1].usedPercent.current == nil, "Stale usage stays historical")
check(moneyLabel(fixture.selectedUsage!.today.costMicrousd) == "≈$1.20", "Estimated cost survives Rust to Swift")
check(moneyLabel(fixture.selectedUsage!.month.costMicrousd) == "Unavailable", "Unpriced totals stay unavailable")
check(fixture.selectedUsage?.today.models[0].totalTokens.current == 60, "Model token fields must agree")
var available = fixture.selectedUsage!
available.windows[1].usedPercent = Metric(value: 58, source: "reported", status: "current")
check(available.menuWindow("auto", now: 100)?.id == "session", "Automatic prefers a current session")
available.windows[0].usedPercent = .unavailable
check(available.menuWindow("auto", now: 100)?.id == "weekly", "Automatic supports weekly-only accounts")
check(available.menuWindow("session", now: 100)?.usedPercent.current == nil, "Explicit selection must not silently switch windows")

var switched = fixture
switched.settings.cursorEnabled = true
switched.settings.selectedProvider = "cursor"
check(switched.selectedUsage?.provider == "cursor", "The selected provider owns the card and status item")
check(switched.selectedUsage?.menuWindow("auto", now: 100)?.usedPercent.current == 0.36, "Cursor percentages must not be multiplied by 100")
check(moneyLabel(switched.selectedUsage!.accountMetrics![0].value) == "$1.00", "Account money shares micro-USD units")
switched.settings.cursorEnabled = false
check(switched.settings.activeProvider == "cursor" && switched.selectedUsage == nil,
      "A pinned disconnected selection stays visible without exposing cached usage")
switched.settings.codexEnabled = false
check(switched.selectedUsage == nil, "All providers disabled hides cached usage")
switched.settings.claudeEnabled = true
switched.settings.selectedProvider = "claude"
check(switched.selectedUsage?.provider == "claude", "Claude can be the only enabled provider")
switched.settings.claudeEnabled = false
switched.settings.opencodeEnabled = true
switched.settings.selectedProvider = "opencode"
check(switched.selectedUsage?.provider == "opencode", "OpenCode can be the only enabled provider")

// The spoken status must describe the same provider/value as the visible text,
// including real zero, missing quota, stale observations and estimated prices.
var statusPresentation = fixture
statusPresentation.usage!.providers[0].observedAt = 100
statusPresentation.usage!.providers[0].windows[0].resetsAt = 3_700
statusPresentation.settings.quotaWindow = "session"
var status = MenuStatus(statusPresentation, now: 100)
check(status.title == "100%" && status.accessibilityTitle.contains("Codex, 5-hour, 100% remaining"),
      "Zero used must be announced as all quota remaining")
statusPresentation.settings.displayMode = "used"
status = MenuStatus(statusPresentation, now: 100)
check(status.title == "0%" && status.accessibilityTitle.contains("0% used"), "Used mode must speak the visible percentage")
status = MenuStatus(statusPresentation, now: 700)
check(status.title == "—" && status.accessibilityTitle.contains("stale"), "Old quota must not be spoken as current")
statusPresentation.usage!.providers[0].windows[0].usedPercent = .unavailable
status = MenuStatus(statusPresentation, now: 100)
check(status.title == "—" && status.accessibilityTitle.contains("unavailable"), "Missing quota must not be spoken as zero or stale")
statusPresentation.settings.displayMode = "cost"
status = MenuStatus(statusPresentation, now: 100)
check(status.title == "≈$1.20" && status.accessibilityTitle.contains("estimated cost $1.20"), "VoiceOver must explain estimated prices")
statusPresentation.usage!.providers[0].error = "Account session expired"
statusPresentation.usage!.providers[0].observedAt = nil
status = MenuStatus(statusPresentation, now: 100)
check(status.title == "≈$1.20" && !status.accessibilityTitle.contains("stale"), "An account auth failure must not taint freshly computed local cost")
statusPresentation.usage!.providers[0].today.costMicrousd.status = "stale"
status = MenuStatus(statusPresentation, now: 100)
check(status.title == "≈$1.20 · stale" && status.accessibilityTitle.contains("estimated cost $1.20 · stale"), "Metric-level stale cost must retain both qualifiers")
statusPresentation.usage!.providers[0].today.costMicrousd = .unavailable
status = MenuStatus(statusPresentation, now: 100)
check(status.title == "—" && status.accessibilityTitle.contains("cost unavailable"), "Unknown prices must not be spoken as free")
statusPresentation.settings.cursorEnabled = true
statusPresentation.settings.selectedProvider = "cursor"
statusPresentation.settings.displayMode = "icon"
check(MenuStatus(statusPresentation, now: 100).accessibilityTitle == "Bridge usage menu, Cursor", "Icon-only status must still identify the selected provider")
statusPresentation.settings.codexEnabled = false
statusPresentation.settings.cursorEnabled = false
check(MenuStatus(statusPresentation, now: 100).title == "" && MenuStatus(statusPresentation, now: 100).accessibilityTitle.contains("disconnected"),
      "A disabled pinned provider must not leave old status data or connectivity text")

// A short loading card must expand with its data while the same menu row stays
// attached. Clamp real scroll origins when content shrinks or the screen changes.
final class MeasuredMenuDocument: NSView {
    var measuredHeight: CGFloat = 80
    var usesFlippedCoordinates = true
    override var isFlipped: Bool { usesFlippedCoordinates }
    override var fittingSize: NSSize { NSSize(width: 350, height: measuredHeight) }
}
let document = MeasuredMenuDocument(frame: .zero)
let scroll = MenuCardScrollView(document: document, width: 350, maximumHeight: 300)
check(scroll.frame.height == 80 && scroll.intrinsicContentSize.height == 80 && !scroll.hasVerticalScroller,
      "Short content must not reserve a scroll gutter or oversized native row")
document.measuredHeight = 720
scroll.updateSize(maximumHeight: 300)
check(scroll.frame.height == 300 && scroll.fittingSize.height == 300 && document.frame.height == 720 && scroll.hasVerticalScroller,
      "A loaded card must expand both the document and the bounded menu viewport")
scroll.contentView.scroll(to: NSPoint(x: 0, y: 150))
document.measuredHeight = 900
scroll.updateSize(maximumHeight: 300)
check(scroll.contentView.bounds.minY == 150, "Same-provider updates must preserve the reader's offset")
document.measuredHeight = 350
scroll.updateSize(maximumHeight: 300)
check(scroll.contentView.bounds.minY == 50, "Shrinking content must clamp a previous deep scroll position")
document.measuredHeight = 100
scroll.updateSize(maximumHeight: 300)
check(scroll.frame.height == 100 && scroll.contentView.bounds.minY == 0 && !scroll.hasVerticalScroller,
      "A tall-to-short update must shrink the native row and clear scrolling")
document.measuredHeight = 720
scroll.updateSize(maximumHeight: 160)
scroll.contentView.scroll(to: NSPoint(x: 0, y: 200))
scroll.updateSize(maximumHeight: 120, resetScroll: true)
check(scroll.frame.height == 120 && scroll.contentView.bounds.minY == 0, "A new provider must start at the top within the current screen cap")
let unflipped = MeasuredMenuDocument(frame: .zero)
unflipped.usesFlippedCoordinates = false
unflipped.measuredHeight = 720
let unflippedScroll = MenuCardScrollView(document: unflipped, width: 350, maximumHeight: 300)
check(unflippedScroll.contentView.bounds.minY == 420, "Unflipped documents must also start at the visual top")
unflippedScroll.contentView.scroll(to: NSPoint(x: 0, y: 270))
unflipped.measuredHeight = 900
unflippedScroll.updateSize(maximumHeight: 300)
check(unflippedScroll.contentView.bounds.minY == 450, "Unflipped documents must preserve distance from the visual top")

// Exercise the production SwiftUI document too: a same-width observed update
// must be measured in this delivery, not after closing and reopening the menu.
let hostedState = MenuState()
hostedState.showingOverview = false
let hostedDocument = NSHostingView(rootView: MenuCard(state: hostedState))
let hostedScroll = MenuCardScrollView(document: hostedDocument, width: 350, maximumHeight: 1_000)
let loadingHeight = hostedScroll.frame.height
hostedState.presentation = fixture
hostedScroll.updateSize(maximumHeight: 1_000)
check(hostedScroll.frame.height > loadingHeight + 100, "Loading-to-data must resize the real hosted SwiftUI card immediately")
hostedState.presentation = Presentation(settings: .initial, usage: nil, refreshing: true, error: nil)
hostedScroll.updateSize(maximumHeight: 1_000)
check(hostedScroll.frame.height < loadingHeight + 50, "Removing provider data must shrink the real hosted SwiftUI card immediately")
var trackedCardMeasured = false
MenuRunLoop.schedule {
    hostedState.presentation = fixture
    hostedScroll.updateSize(maximumHeight: 1_000)
    trackedCardMeasured = hostedScroll.frame.height > loadingHeight + 100
}
CFRunLoopRunInMode(CFRunLoopMode(RunLoop.Mode.eventTracking.rawValue as CFString), 0.1, true)
check(trackedCardMeasured, "The same snapshot delivery must resize real SwiftUI content in menu tracking mode")

// History interaction belongs to the provider surface and survives replacement
// snapshots and hosted-card reconstruction without affecting another provider.
hostedState.setHistoryMetric("cost", for: "codex")
hostedState.setHistoryDay("2026-09-08", for: "codex")
hostedState.presentation = fixture
check(hostedState.historySelection(for: "codex") == MenuState.HistorySelection(day: "2026-09-08", metric: "cost"),
      "A provider history selection must survive a new snapshot")
check(hostedState.historySelection(for: "cursor") == MenuState.HistorySelection(),
      "History selection must not leak across providers")
let reconstructedHistoryCard = NSHostingView(rootView: MenuCard(state: hostedState))
reconstructedHistoryCard.layoutSubtreeIfNeeded()
check(hostedState.historySelection(for: "codex").metric == "cost",
      "Rebuilding the hosted card must preserve its provider-local metric")
let missingHistoryDay = DailyUsageView(usage: fixture.usage!.providers[0], showCost: true, showTokens: true,
    selectedDay: .constant("1900-01-01"), chartMetric: .constant("tokens"))
check(missingHistoryDay.selected == nil, "An expired day selection must never substitute the whole month's totals")

let coalescedDocument = MeasuredMenuDocument(frame: .zero)
let coalescedScroll = MenuCardScrollView(document: coalescedDocument, width: 350, maximumHeight: 300)
coalescedDocument.measuredHeight = 500
coalescedScroll.scheduleUpdateSize(maximumHeight: 260)
coalescedScroll.scheduleUpdateSize(maximumHeight: 180, resetScroll: true)
check(coalescedScroll.frame.height == 80, "A scheduled resize must not reenter the current SwiftUI update")
CFRunLoopRunInMode(CFRunLoopMode(RunLoop.Mode.eventTracking.rawValue as CFString), 0.1, true)
check(coalescedScroll.frame.height == 180 && coalescedScroll.hasVerticalScroller,
      "Coalesced menu-tracking resize applies the latest bound once")

let appearanceRoot = NSMenu()
let appearanceChild = NSMenu()
let appearanceItem = NSMenuItem(title: "Details", action: nil, keyEquivalent: "")
appearanceItem.submenu = appearanceChild
appearanceRoot.addItem(appearanceItem)
for name in [NSAppearance.Name.aqua, .darkAqua, .accessibilityHighContrastAqua, .accessibilityHighContrastDarkAqua] {
    let appearance = NSAppearance(named: name)!
    MenuAppearance.pin(appearanceRoot, to: appearance)
    check(appearanceRoot.appearance === appearance && appearanceChild.appearance === appearance,
          "Root and child menus must preserve the exact appearance, including accessibility attributes")
}

// A provider snapshot arriving while the breakdown is open must preserve the
// tracked menu and its model children. Exercise native menu objects without
// creating a status item, displaying a menu, or reading a live provider.
var breakdownPresentation = fixture
breakdownPresentation.settings.cursorEnabled = true
breakdownPresentation.settings.opencodeEnabled = true
for index in breakdownPresentation.usage!.providers.indices {
    let provider = breakdownPresentation.usage!.providers[index].provider
    breakdownPresentation.usage!.providers[index].today.models[0].model = "\(provider)-model"
}
let breakdownState = MenuState()
breakdownState.showingOverview = false
breakdownState.presentation = breakdownPresentation
let breakdown = ModelBreakdownMenu(state: breakdownState)
let parentMenu = NSMenu()
parentMenu.addItem(breakdown.item)
let trackedMenu = breakdown.menu
let trackedModel = trackedMenu.items[1]
let trackedValues = trackedModel.submenu!
check(trackedModel.title == "codex-model", "The breakdown initially uses the selected provider")
breakdown.menuWillOpen(trackedMenu)
breakdownState.presentation.settings.selectedProvider = "cursor"
breakdown.update()
check(breakdown.item.submenu === trackedMenu && trackedMenu.items[1] === trackedModel,
      "A provider switch must preserve the tracked breakdown and its model rows")
check(trackedModel.submenu === trackedValues && trackedModel.title == "codex-model",
      "The currently open model submenu must remain intact")
breakdownState.presentation.settings.selectedProvider = "opencode"
breakdownState.presentation.settings.showCost = false
breakdown.update()
breakdown.menuDidClose(trackedMenu)
check(trackedMenu.items[1] === trackedModel, "The close callback must not mutate menu structure")
breakdown.menuNeedsUpdate(trackedMenu)
check(parentMenu.items[0] === breakdown.item && breakdown.item.submenu === trackedMenu,
      "Refreshing the child must preserve the parent menu's structure")
check(trackedMenu.items[1].title == "opencode-model", "Reopening must read the latest provider, skipping superseded snapshots")
check(trackedMenu.items[1].submenu!.items.count == 4, "Reopening must apply the latest cost visibility preference")

// An open NSMenu must receive Rust snapshots without waiting for dismissal.
// The fallback block must not run the operation twice or release its context twice.
var deliveries = 0
MenuRunLoop.schedule { deliveries += 1 }
CFRunLoopRunInMode(CFRunLoopMode(RunLoop.Mode.eventTracking.rawValue as CFString), 0.1, true)
check(deliveries == 1, "Snapshot delivery must run in the menu tracking loop")
CFRunLoopRunInMode(.defaultMode, 0.1, true)
check(deliveries == 1, "The default-mode fallback must not repeat a tracking delivery")
MenuRunLoop.schedule { deliveries += 1 }
CFRunLoopRunInMode(.defaultMode, 0.1, true)
check(deliveries == 2, "A closed menu must still receive snapshots")
CFRunLoopRunInMode(CFRunLoopMode(RunLoop.Mode.eventTracking.rawValue as CFString), 0.1, true)
check(deliveries == 2, "Opening a menu later must not replay an old snapshot")

let callbackCount = UnsafeMutablePointer<Int>.allocate(capacity: 1)
callbackCount.initialize(to: 0)
let queued = DispatchSemaphore(value: 0)
Thread {
    scheduleMenuBarTask({ context in
        check(Thread.isMainThread, "The worker callback must execute on AppKit's main thread")
        context.assumingMemoryBound(to: Int.self).pointee += 1
    }, UnsafeMutableRawPointer(callbackCount))
    queued.signal()
}.start()
check(queued.wait(timeout: .now() + 2) == .success, "A worker must enqueue without waiting for the main thread")
CFRunLoopRunInMode(CFRunLoopMode(RunLoop.Mode.eventTracking.rawValue as CFString), 0.1, true)
CFRunLoopRunInMode(.defaultMode, 0.1, true)
check(callbackCount.pointee == 1, "The C ABI context must be delivered once from a worker")
callbackCount.deinitialize(count: 1)
callbackCount.deallocate()

// Used-first bars are independent of status-item text and must distinguish
// exhausted quota, real zero, missing observations, and stale provider data.
var quotaUsage = fixture.selectedUsage!
quotaUsage.observedAt = 100
quotaUsage.error = nil
var quotaWindow = quotaUsage.windows[0]
quotaWindow.resetsAt = 9_100
quotaWindow.windowMinutes = 300
quotaWindow.usedPercent = Metric(value: 100, source: "reported", status: "current")
var quotaDisplay = QuotaDisplay(quotaWindow, usage: quotaUsage, mode: "used", now: Date(timeIntervalSince1970: 100), failed: false)
check(quotaDisplay.fill == 1 && quotaDisplay.valueLabel == "100% used" && quotaDisplay.counterpart == "0% left", "Exhausted quota must fill the entire used bar")
quotaDisplay = QuotaDisplay(quotaWindow, usage: quotaUsage, mode: "remaining", now: Date(timeIntervalSince1970: 100), failed: false)
check(quotaDisplay.fill == 0 && quotaDisplay.valueLabel == "0% left", "Remaining mode deliberately empties an exhausted bar")
quotaWindow.usedPercent = zero
quotaDisplay = QuotaDisplay(quotaWindow, usage: quotaUsage, mode: "used", now: Date(timeIntervalSince1970: 100), failed: false)
check(quotaDisplay.fill == 0 && quotaDisplay.counterpart == "100% left", "Reported zero stays an observed empty used bar")
quotaWindow.usedPercent = .unavailable
check(QuotaDisplay(quotaWindow, usage: quotaUsage, mode: "used", now: Date(timeIntervalSince1970: 100), failed: false).fill == nil, "Unavailable must not masquerade as zero")
quotaWindow.usedPercent = Metric(value: 42, source: "reported", status: "current")
check(QuotaDisplay(quotaWindow, usage: quotaUsage, mode: "used", now: Date(timeIntervalSince1970: 800), failed: false).valueLabel == "Stale", "Old values stay stale independently of bar direction")
quotaUsage.error = "Account unavailable"
check(QuotaDisplay(quotaWindow, usage: quotaUsage, mode: "used", now: Date(timeIntervalSince1970: 100), failed: false).fill == nil, "Provider errors must suppress current-looking quota")
check(quotaPace(quotaWindow, used: 25, now: Date(timeIntervalSince1970: 100)) == "25% in reserve", "Reserve is elapsed-time allowance less usage, not credits")
check(quotaPace(quotaWindow, used: 75, now: Date(timeIntervalSince1970: 100)) == "25% in deficit", "Ahead-of-time consumption is a pace deficit")
check(quotaPace(quotaWindow, used: nil, now: Date(timeIntervalSince1970: 100)) == nil, "Unknown or stale values must have no forecast")
check(quotaPace(quotaWindow, used: 100, now: Date(timeIntervalSince1970: 100)) == nil, "An exhausted quota must not suggest reserve")

var layoutPresentation = fixture
layoutPresentation.usage!.providers[0].observedAt = 100
layoutPresentation.usage!.providers[0].windows[0] = quotaWindow
layoutPresentation.usage!.providers[0].windows[1].usedPercent = Metric(value: 74, source: "reported", status: "current")
layoutPresentation.usage!.providers[0].windows[1].resetsAt = 172_800
layoutPresentation.settings.statusLayout = [["icon", "space", "fiveHourUsed"], ["weeklyRemaining"]]
var statusLayout = StatusLayout(layoutPresentation, now: 100)
check(statusLayout.visibleText == "\u{fffc} 5h 42%\n7d 26%", "Two-line layouts preserve order and spaces")
check(statusLayout.accessibilityTitle.contains("42% used") && statusLayout.accessibilityTitle.contains("26% remaining"), "Custom visible percentages must retain spoken semantics")
check(statusLayout.attributedTitle(icon: MenuController.templateIcon()).attribute(.attachment, at: 0, effectiveRange: nil) != nil, "The icon token must render an actual template attachment")
statusLayout = StatusLayout(layoutPresentation, now: 900)
check(statusLayout.visibleText.contains("5h —") && statusLayout.accessibilityTitle.contains("stale"), "Custom layouts must not present stale quota as current")
layoutPresentation.settings.statusLayout = [["todayCost"]]
layoutPresentation.usage!.providers[0].error = "Expired account"
check(StatusLayout(layoutPresentation, now: 100).visibleText == "≈$1.20", "Account failures must not invalidate local ledger cost in a layout")
layoutPresentation.settings.statusLayout = [["space", "dot"]]
check(!StatusLayout(layoutPresentation, now: 100).hasContent, "A whitespace-only layout must fall back to a visible status item")

let overviewState = MenuState()
overviewState.presentation = fixture
let overviewBreakdown = ModelBreakdownMenu(state: overviewState)
check(overviewBreakdown.item.isHidden, "Overview must omit cost/model breakdown actions")
overviewState.showingOverview = false
overviewBreakdown.update()
check(!overviewBreakdown.item.isHidden, "Provider details retain the model breakdown")
let visibleBeforeTracking = overviewBreakdown.item.isHidden
let childBeforeTracking = overviewBreakdown.menu.items[1]
overviewBreakdown.parentTracking = true
overviewState.showingOverview = true
overviewBreakdown.update()
check(overviewBreakdown.item.isHidden == visibleBeforeTracking && !overviewBreakdown.item.isEnabled, "Switching to Overview must not remove a root row during tracking")
check(overviewBreakdown.menu.items[1] === childBeforeTracking, "Root tracking must also defer submenu reconstruction")
overviewBreakdown.parentTracking = false
overviewBreakdown.update()
check(overviewBreakdown.item.isHidden, "The next closed-menu update can remove the unused breakdown row")
layoutPresentation.settings.statusLayout = [["icon", "space", "used"]]
let leadingLayout = StatusLayout(layoutPresentation, now: 100)
check(leadingLayout.hasLeadingIcon && !leadingLayout.requiresTemplateImage, "Single-line leading icons stay in AppKit's native image slot")
check(leadingLayout.attributedTitle(icon: MenuController.templateIcon(), omitLeadingIcon: true).attribute(.attachment, at: 0, effectiveRange: nil) == nil, "A leading icon must not be duplicated as a text attachment")
layoutPresentation.settings.statusLayout = [["icon", "space", "fiveHourUsed"], ["weeklyUsed"]]
let stackedImage = StatusLayout(layoutPresentation, now: 100).templateImage(icon: MenuController.templateIcon())
check(stackedImage.isTemplate && stackedImage.size.height <= 22, "Stacked text and inline icons must tint as one status-bar-sized alpha mask")
let meterImage = MenuController.meterIcon(layoutPresentation)
check(meterImage.isTemplate && meterImage.size == NSSize(width: 18, height: 18), "Quota icon customization must preserve the template contract")

let image = MenuController.templateIcon()
check(image.isTemplate, "Menu icon must be a system template")
check(image.size == NSSize(width: 18, height: 18), "Menu icon uses point dimensions")
let representation = NSBitmapImageRep(data: image.tiffRepresentation!)!
var clear = 0
var ink = 0
for y in 0..<representation.pixelsHigh {
    for x in 0..<representation.pixelsWide {
        let color = representation.colorAt(x: x, y: y)!.usingColorSpace(.deviceRGB)!
        if color.alphaComponent == 0 { clear += 1 }
        else {
            ink += 1
            check(color.redComponent == color.greenComponent && color.greenComponent == color.blueComponent, "Template must have no colored pixels")
        }
    }
}
check(clear > 0 && ink > 0, "Icon must contain an alpha mask and visible ink")
check(representation.colorAt(x: 0, y: 0)!.alphaComponent == 0, "Icon background must be transparent")

// Favorites remain bounded while the independent native strip handles a future
// provider catalog without inventing adapters or expanding the menu card.
var allProviders = fixture.settings
allProviders.claudeEnabled = true
allProviders.cursorEnabled = true
allProviders.opencodeEnabled = true
check(allProviders.enabledProviders == ["codex", "claude", "cursor", "opencode"],
      "Cursor appears immediately after Claude in both switcher and Overview")
allProviders.pinnedProviders = ["cursor", "codex", "cursor", "claude", "opencode"]
check(allProviders.normalizedPinnedProviders == ["cursor", "codex", "claude"], "Favorites are unique and capped at three")
check(allProviders.visibleProviders == ["cursor", "codex", "claude", "opencode"], "Favorites lead enabled overflow providers")
var legacySettings = fixture.settings
legacySettings.pinnedProviders = nil
check(legacySettings.visibleProviders.prefix(3) == ["codex", "claude", "cursor"], "Old payloads show the default favorite three including Cursor")
var emptyFavorites = fixture.settings
emptyFavorites.pinnedProviders = []
emptyFavorites.codexEnabled = false
check(emptyFavorites.visibleProviders.isEmpty && emptyFavorites.activeProvider == nil, "Empty favorites and disabled providers leave Overview as the only surface")

var pickedProvider = ""
let manyProviders = (1...69).map { "provider-\($0)" }
let switcher = ProviderSwitcherView(providers: manyProviders, selection: "overview") { pickedProvider = $0 }
switcher.layoutSubtreeIfNeeded()
check(switcher.frame.width == 350 && switcher.intrinsicContentSize.width == 350, "Overflow never expands the 350pt widget")
check(switcher.buttons.count == 70 && switcher.buttons[0].title == "Overview", "Overview stays fixed ahead of all 69 synthetic names")
check(switcher.buttons.dropFirst().allSatisfy { $0.frame.width >= ProviderSwitcherView.minimumProviderWidth }, "Provider titles retain a comfortable minimum width")
let longNameSwitcher = ProviderSwitcherView(
    providers: ["codex", "claude", "cursor", "a-provider-name-that-must-never-widen-the-default-slots"],
    selection: "overview") { _ in }
longNameSwitcher.layoutSubtreeIfNeeded()
let defaultProviderFrames = Array(longNameSwitcher.buttons.dropFirst().prefix(3)).map(\.frame)
check(defaultProviderFrames.count == 3 && defaultProviderFrames.last!.maxX <= longNameSwitcher.testViewportWidth,
      "Codex, Claude, and Cursor fit fully before overflow arrows")
check(Set(defaultProviderFrames.map(\.width)).count == 1 && longNameSwitcher.buttons.last?.toolTip != nil,
      "A later long provider truncates with a tooltip instead of widening the three default slots")
check(switcher.testMaximumOffset > 0 && !switcher.testArrowState.previous && switcher.testArrowState.next, "Overflow starts clamped with only the forward arrow enabled")
switcher.testScrollBackward()
check(switcher.testScrollOffset == 0, "The back arrow is a no-op at the beginning")
for _ in 0..<100 { switcher.testScrollForward() }
check(switcher.testScrollOffset == switcher.testMaximumOffset && switcher.testArrowState.previous && !switcher.testArrowState.next, "Repeated arrows reach and clamp at the end")
switcher.testScrollForward()
check(switcher.testScrollOffset == switcher.testMaximumOffset, "The forward arrow is a no-op at the end")
switcher.update(providers: manyProviders, selection: "provider-69") { pickedProvider = $0 }
check(switcher.testScrollOffset == switcher.testMaximumOffset, "Selecting an offscreen provider reveals the final segment")
let preservedOffset = switcher.testMaximumOffset / 2
switcher.testSetScrollOffset(preservedOffset)
switcher.testScrollByWheelDelta(12)
check(abs(switcher.testScrollOffset - (preservedOffset - 12)) < 0.5, "A positive native wheel delta scrolls toward the leading providers")
switcher.testScrollByWheelDelta(-12)
check(abs(switcher.testScrollOffset - preservedOffset) < 0.5, "A negative native wheel delta scrolls toward later providers")
switcher.update(providers: manyProviders, selection: "provider-69") { pickedProvider = $0 }
check(abs(switcher.testScrollOffset - preservedOffset) < 0.5, "Repeated polling snapshots preserve manual horizontal scroll")
switcher.buttons[69].performClick(nil)
check(pickedProvider == "provider-69", "Synthetic overflow selection dispatches the provider ID")

var disconnected = fixture
disconnected.settings.pinnedProviders = ["cursor"]
disconnected.settings.selectedProvider = "cursor"
disconnected.settings.cursorEnabled = false
check(disconnected.selectedUsage == nil, "A hostile cached disconnected provider exposes no quota, account, cost, or token model")
check(MenuStatus(disconnected, now: 100).accessibilityTitle.contains("disconnected"), "Disconnected status is explicit")
disconnected.settings.statusLayout = [["todayCost", "space", "used"]]
check(StatusLayout(disconnected, now: 100).usage == nil && StatusLayout(disconnected, now: 100).visibleText.contains("—")
      && StatusLayout(disconnected, now: 100).accessibilityTitle.contains("disconnected"),
      "Custom status layouts consume no cached disconnected data and announce connectivity")
let disconnectedState = MenuState()
var settingsActions = 0
disconnectedState.openSettings = { settingsActions += 1 }
disconnectedState.openSettings()
check(settingsActions == 1, "The disconnected card settings action is independently wired")
try renderMenuCardFixtures(fixture)
print("Menu Bar Swift checks passed: wire fixture, disabled-data isolation, favorite defaults, 69-provider bounded overflow, selection reveal, scroll preservation, countdowns, status accessibility, viewport resizing, appearance, submenu tracking, template mask")

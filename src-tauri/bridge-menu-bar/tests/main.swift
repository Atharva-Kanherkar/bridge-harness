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
check(moneyLabel(zero) == "$0.00", "A reported zero cost is valid")
check(countLabel(.unavailable) == "Unavailable", "Unknown tokens must not become zero")
check(moneyLabel(.unavailable) == "Unavailable", "Unpriced usage must not become $0")
check(moneyLabel(Metric(value: 2_300_000, source: "estimated", status: "current")) == "≈$2.30", "Estimated costs need a qualifier")
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
check(switched.selectedUsage?.provider == "codex", "Disabling the selected provider falls back to an enabled provider")
switched.settings.codexEnabled = false
check(switched.selectedUsage == nil, "All providers disabled hides usage")
switched.settings.claudeEnabled = true
check(switched.selectedUsage?.provider == "claude", "Claude can be the only enabled provider")
switched.settings.claudeEnabled = false
switched.settings.opencodeEnabled = true
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
check(MenuStatus(statusPresentation, now: 100).accessibilityTitle.contains("no providers enabled"), "Disabled providers must not leave old accessibility text")

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

// The actual AppKit buttons must fit the five-tab menu, retain their frames
// across selection, and dispatch Cursor's ID rather than a display index.
var allProviders = fixture.settings
allProviders.claudeEnabled = true
allProviders.cursorEnabled = true
allProviders.opencodeEnabled = true
check(allProviders.enabledProviders == ["codex", "claude", "cursor", "opencode"],
      "Cursor appears immediately after Claude in both switcher and Overview")
var pickedProvider = ""
let switcher = ProviderSwitcherView(providers: allProviders.enabledProviders, selection: "overview") { pickedProvider = $0 }
switcher.layoutSubtreeIfNeeded()
check(switcher.buttons.map(\.title) == ["Overview", "Codex", "Claude", "Cursor", "OpenCode"], "All five titles remain visible")
let initialFrames = switcher.buttons.map(\.frame)
for (index, button) in switcher.buttons.enumerated() {
    let textWidth = (button.title as NSString).size(withAttributes: [.font: button.font!]).width
    check(button.frame.width >= textWidth + 8, "Each provider title has breathing room")
    check(button.frame.minX >= 0 && button.frame.maxX <= switcher.bounds.width, "Provider tabs stay within the menu")
    if index > 0 {
        check(button.frame.minX - switcher.buttons[index - 1].frame.maxX >= 1, "Provider tabs never overlap")
        check(button.frame.width == switcher.buttons[index - 1].frame.width, "Provider tabs have uniform widths")
    }
}
switcher.buttons[3].performClick(nil)
check(pickedProvider == "cursor" && switcher.buttons.filter { $0.state == .on }.map(\.title) == ["Cursor"],
      "Clicking Cursor selects exactly its provider")
switcher.update(providers: allProviders.enabledProviders, selection: "cursor") { pickedProvider = $0 }
switcher.layoutSubtreeIfNeeded()
check(switcher.buttons.map(\.frame) == initialFrames, "Selecting a provider must not shift the row")
switcher.update(providers: ["codex", "cursor"], selection: "cursor") { pickedProvider = $0 }
switcher.layoutSubtreeIfNeeded()
switcher.buttons[1].performClick(nil)
check(pickedProvider == "codex", "Toggling provider visibility must not leave stale selection callbacks")
try renderMenuCardFixtures(fixture)
print("Menu Bar Swift checks passed: wire fixture, semantics, countdowns, dynamic status accessibility, viewport resizing and scroll clamping, appearance propagation, submenu tracking deferral, tracking-loop delivery, template flag, alpha mask, monochrome pixels")

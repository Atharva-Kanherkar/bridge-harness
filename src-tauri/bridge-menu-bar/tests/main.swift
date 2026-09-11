import AppKit
import Foundation

func check(_ condition: @autoclosure () -> Bool, _ message: String) {
    if !condition() { fatalError(message) }
}

let zero = Metric(value: 0, source: "reported", status: "current")
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
print("Menu Bar Swift checks passed: wire fixture, semantics, countdowns, submenu tracking deferral, tracking-loop delivery, template flag, alpha mask, monochrome pixels")

import AppKit
import SwiftUI

private final class FixtureCanvas: NSView {
    override var isOpaque: Bool { true }
    override func draw(_ dirtyRect: NSRect) {
        NSColor.windowBackgroundColor.setFill()
        dirtyRect.fill()
    }
}

// Optional synthetic card renders, never screenshots of the user's desktop.
// Adapted from CodexBar MenuLayoutScreenshotRenderTests.pngDataWithWindow at
// 928166f. Copyright (c) 2026 Peter Steinberger; see the bundled MIT notice.
func renderMenuCardFixtures(_ fixture: Presentation) throws {
    guard let destination = ProcessInfo.processInfo.environment["BRIDGE_MENU_BAR_RENDER_DIR"] else { return }
    let output = URL(fileURLWithPath: destination, isDirectory: true)
    try FileManager.default.createDirectory(at: output, withIntermediateDirectories: true)
    _ = NSApplication.shared
    let now = Int64(Date().timeIntervalSince1970)
    var snapshot = fixture
    snapshot.settings.claudeEnabled = true
    snapshot.settings.cursorEnabled = true
    snapshot.settings.opencodeEnabled = true
    snapshot.usage!.providers[0].observedAt = now
    snapshot.usage!.providers[0].account = "person@example.com"
    snapshot.usage!.providers[0].windows[0].usedPercent = Metric(value: 42, source: "reported", status: "current")
    snapshot.usage!.providers[0].windows[0].resetsAt = now + 7_200
    snapshot.usage!.providers[0].windows[1].usedPercent = Metric(value: 58, source: "reported", status: "current")
    snapshot.usage!.providers[0].windows[1].resetsAt = now + 172_800
    for index in snapshot.usage!.providers.indices {
        snapshot.usage!.providers[index].observedAt = now
        for window in snapshot.usage!.providers[index].windows.indices {
            snapshot.usage!.providers[index].windows[window].resetsAt = now + 172_800
            snapshot.usage!.providers[index].windows[window].usedPercent.status = "current"
        }
    }
    snapshot.usage!.providers[0].windows[0].resetsAt = now + 7_200
    let cursor = snapshot.usage!.providers.firstIndex { $0.provider == "cursor" }!
    snapshot.usage!.providers[cursor].windows = [
        QuotaWindow(id: "total", label: "Total", usedPercent: Metric(value: 100, source: "reported", status: "current"), resetsAt: now + 259_200, windowMinutes: nil),
        QuotaWindow(id: "cursor", label: "Cursor", usedPercent: Metric(value: 62, source: "reported", status: "current"), resetsAt: now + 259_200, windowMinutes: nil),
        QuotaWindow(id: "third-party", label: "Third Party", usedPercent: Metric(value: 100, source: "reported", status: "current"), resetsAt: now + 259_200, windowMinutes: nil),
        QuotaWindow(id: "cursor-grok-bot", label: "Grok Bot", usedPercent: Metric(value: 35, source: "reported", status: "current"), resetsAt: now + 25_200, windowMinutes: 10080),
    ]
    let claude = snapshot.usage!.providers.firstIndex { $0.provider == "claude" }!
    snapshot.usage!.providers[claude].windows.append(
        QuotaWindow(id: "weekly", label: "Weekly", usedPercent: Metric(value: 62, source: "reported", status: "current"), resetsAt: now + 172_800, windowMinutes: 10080))
    snapshot.usage!.providers[claude].windows.append(
        QuotaWindow(id: "claude-weekly-scoped-claude-fable", label: "Weekly · Fable only", usedPercent: Metric(value: 37.5, source: "reported", status: "current"), resetsAt: now + 172_800, windowMinutes: 10080))
    var days: [UsageDay] = []
    for index in 1...14 {
        var period = snapshot.usage!.providers[0].today
        period.tokens.value = Double(index * 81_321)
        period.costMicrousd.value = index == 8 ? nil : Double(index * 930_000)
        period.costMicrousd.status = index == 8 ? "unavailable" : "current"
        days.append(UsageDay(day: String(format: "2026-09-%02d", index), usage: period))
    }
    snapshot.usage!.providers[0].daily = days
    var missingCostSnapshot = snapshot
    for index in missingCostSnapshot.usage!.providers.indices {
        missingCostSnapshot.usage!.providers[index].today.costMicrousd = .unavailable
        missingCostSnapshot.usage!.providers[index].month.costMicrousd = .unavailable
        missingCostSnapshot.usage!.providers[index].daily = missingCostSnapshot.usage!.providers[index].daily?.map { day in
            var day = day
            day.usage.costMicrousd = .unavailable
            return day
        }
    }
    var manifest = "Synthetic production MenuCard on a plain window background; native NSMenu chrome is not rendered.\n"
    let appearances: [(String, NSAppearance.Name)] = [
        ("light", .aqua), ("dark", .darkAqua),
        ("high-contrast-light", .accessibilityHighContrastAqua),
        ("high-contrast-dark", .accessibilityHighContrastDarkAqua),
    ]
    for (name, appearanceName) in appearances {
      for surface in ["overview", "overview-default", "codex", "codex-day", "claude", "cursor", "cursor-disconnected"] {
        for (sizeName, maximumHeight) in [("full", CGFloat(1_000)), ("compact", CGFloat(280))] {
            let appearance = NSAppearance(named: appearanceName)!
            let state = MenuState()
            state.presentation = snapshot
            state.showingOverview = surface.hasPrefix("overview")
            if surface == "overview-default" {
                state.presentation.settings.pinnedProviders = nil
                state.presentation.settings.claudeEnabled = false
                state.presentation.settings.cursorEnabled = false
                state.presentation.settings.opencodeEnabled = false
            } else if surface == "cursor-disconnected" {
                state.presentation.settings.selectedProvider = "cursor"
                state.presentation.settings.cursorEnabled = false
            } else if surface == "codex-day" {
                state.presentation.settings.selectedProvider = "codex"
                state.setHistoryDay("2026-09-14", for: "codex")
            } else if !state.showingOverview {
                state.presentation.settings.selectedProvider = surface
            }
            let hosting = NSHostingView(rootView: MenuCard(state: state))
            hosting.appearance = appearance
            let scroll = MenuCardScrollView(document: hosting, width: 350, maximumHeight: maximumHeight)
            let canvas = FixtureCanvas(frame: scroll.frame)
            canvas.appearance = appearance
            canvas.addSubview(scroll)
            let window = NSWindow(contentRect: canvas.frame, styleMask: [.borderless], backing: .buffered, defer: false)
            window.isReleasedWhenClosed = false
            window.appearance = appearance
            window.contentView = canvas
            defer { window.contentView = nil; window.close() }
            window.layoutIfNeeded()
            hosting.layoutSubtreeIfNeeded()
            RunLoop.current.run(until: Date().addingTimeInterval(0.1))
            scroll.updateSize(maximumHeight: maximumHeight, resetScroll: true)
            canvas.setFrameSize(scroll.fittingSize)
            window.setContentSize(canvas.frame.size)
            canvas.layoutSubtreeIfNeeded()
            check(!window.isVisible, "Fixture render windows must never be shown")
            let bitmap = canvas.bitmapImageRepForCachingDisplay(in: canvas.bounds)!
            canvas.cacheDisplay(in: canvas.bounds, to: bitmap)
            let file = "menu-\(surface)-\(name)-\(sizeName).png"
            try bitmap.representation(using: .png, properties: [:])!.write(to: output.appendingPathComponent(file))
            manifest += "\(file): \(Int(canvas.frame.width))×\(Int(canvas.frame.height))pt, scroller=\(scroll.hasVerticalScroller)\n"
            if scroll.hasVerticalScroller {
                let bottom = max(0, hosting.frame.height - scroll.contentView.bounds.height)
                scroll.contentView.scroll(to: NSPoint(x: 0, y: hosting.isFlipped ? bottom : 0))
                scroll.reflectScrolledClipView(scroll.contentView)
                canvas.cacheDisplay(in: canvas.bounds, to: bitmap)
                let bottomFile = "menu-\(surface)-\(name)-\(sizeName)-bottom.png"
                try bitmap.representation(using: .png, properties: [:])!.write(to: output.appendingPathComponent(bottomFile))
                manifest += "\(bottomFile): scrolled to the final content row\n"
            }
        }
      }
      let overflow = ProviderSwitcherView(providers: (1...69).map { "provider-\($0)" }, selection: "provider-69") { _ in }
      overflow.appearance = NSAppearance(named: appearanceName)!
      let overflowCanvas = FixtureCanvas(frame: overflow.frame)
      overflowCanvas.appearance = overflow.appearance
      overflowCanvas.addSubview(overflow)
      let overflowWindow = NSWindow(contentRect: overflowCanvas.frame, styleMask: [.borderless], backing: .buffered, defer: false)
      overflowWindow.isReleasedWhenClosed = false
      overflowWindow.appearance = overflow.appearance
      overflowWindow.contentView = overflowCanvas
      overflow.layoutSubtreeIfNeeded()
      check(!overflowWindow.isVisible, "Overflow fixture window must never be shown")
      let overflowBitmap = overflowCanvas.bitmapImageRepForCachingDisplay(in: overflowCanvas.bounds)!
      overflowCanvas.cacheDisplay(in: overflowCanvas.bounds, to: overflowBitmap)
      let overflowFile = "menu-provider-overflow-\(name).png"
      try overflowBitmap.representation(using: .png, properties: [:])!.write(to: output.appendingPathComponent(overflowFile))
      manifest += "\(overflowFile): 350×30pt, selected final provider, Overview fixed\n"
      overflowWindow.contentView = nil
      overflowWindow.close()

      let longOverflow = ProviderSwitcherView(
          providers: ["codex", "claude", "cursor", "a-provider-name-that-must-truncate-without-widening"],
          selection: "overview") { _ in }
      longOverflow.appearance = NSAppearance(named: appearanceName)!
      let longCanvas = FixtureCanvas(frame: longOverflow.frame)
      longCanvas.appearance = longOverflow.appearance
      longCanvas.addSubview(longOverflow)
      let longWindow = NSWindow(contentRect: longCanvas.frame, styleMask: [.borderless], backing: .buffered, defer: false)
      longWindow.isReleasedWhenClosed = false
      longWindow.appearance = longOverflow.appearance
      longWindow.contentView = longCanvas
      longOverflow.layoutSubtreeIfNeeded()
      check(!longWindow.isVisible, "Long-name overflow fixture window must never be shown")
      let longBitmap = longCanvas.bitmapImageRepForCachingDisplay(in: longCanvas.bounds)!
      longCanvas.cacheDisplay(in: longCanvas.bounds, to: longBitmap)
      let longFile = "menu-provider-default-three-long-overflow-\(name).png"
      try longBitmap.representation(using: .png, properties: [:])!.write(to: output.appendingPathComponent(longFile))
      manifest += "\(longFile): 350×30pt, full default three before long-name overflow\n"
      longWindow.contentView = nil
      longWindow.close()

      let missingState = MenuState()
      missingState.showingOverview = false
      missingState.presentation = missingCostSnapshot
      missingState.setHistoryMetric("cost", for: "codex")
      let missingHosting = NSHostingView(rootView: MenuCard(state: missingState))
      missingHosting.appearance = NSAppearance(named: appearanceName)!
      let missingScroll = MenuCardScrollView(document: missingHosting, width: 350, maximumHeight: 1_000)
      let missingCanvas = FixtureCanvas(frame: missingScroll.frame)
      missingCanvas.appearance = missingHosting.appearance
      missingCanvas.addSubview(missingScroll)
      missingCanvas.layoutSubtreeIfNeeded()
      let missingBitmap = missingCanvas.bitmapImageRepForCachingDisplay(in: missingCanvas.bounds)!
      missingCanvas.cacheDisplay(in: missingCanvas.bounds, to: missingBitmap)
      let missingFile = "menu-cost-history-unavailable-\(name).png"
      try missingBitmap.representation(using: .png, properties: [:])!.write(to: output.appendingPathComponent(missingFile))
      manifest += "\(missingFile): explicit unavailable state with stable history height\n"
    }
    try manifest.write(to: output.appendingPathComponent("README.txt"), atomically: true, encoding: .utf8)
    print("Synthetic menu card renders written to \(output.path)")
}

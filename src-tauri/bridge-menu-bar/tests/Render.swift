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
    var manifest = "Synthetic production MenuCard on a plain window background; native NSMenu chrome is not rendered.\n"
    let appearances: [(String, NSAppearance.Name)] = [
        ("light", .aqua), ("dark", .darkAqua),
        ("high-contrast-light", .accessibilityHighContrastAqua),
        ("high-contrast-dark", .accessibilityHighContrastDarkAqua),
    ]
    for (name, appearanceName) in appearances {
        for (sizeName, maximumHeight) in [("full", CGFloat(1_000)), ("compact", CGFloat(280))] {
            let appearance = NSAppearance(named: appearanceName)!
            let state = MenuState()
            state.presentation = snapshot
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
            let file = "menu-card-\(name)-\(sizeName).png"
            try bitmap.representation(using: .png, properties: [:])!.write(to: output.appendingPathComponent(file))
            manifest += "\(file): \(Int(canvas.frame.width))×\(Int(canvas.frame.height))pt, scroller=\(scroll.hasVerticalScroller)\n"
            if scroll.hasVerticalScroller {
                let bottom = max(0, hosting.frame.height - scroll.contentView.bounds.height)
                scroll.contentView.scroll(to: NSPoint(x: 0, y: hosting.isFlipped ? bottom : 0))
                scroll.reflectScrolledClipView(scroll.contentView)
                canvas.cacheDisplay(in: canvas.bounds, to: bitmap)
                let bottomFile = "menu-card-\(name)-\(sizeName)-bottom.png"
                try bitmap.representation(using: .png, properties: [:])!.write(to: output.appendingPathComponent(bottomFile))
                manifest += "\(bottomFile): scrolled to the final content row\n"
            }
        }
    }
    try manifest.write(to: output.appendingPathComponent("README.txt"), atomically: true, encoding: .utf8)
    print("Synthetic menu card renders written to \(output.path)")
}

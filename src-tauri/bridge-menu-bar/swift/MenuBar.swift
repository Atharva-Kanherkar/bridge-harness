import AppKit
import SwiftUI

// Owns native presentation only. AppKit retains the status item until destroy;
// Rust supplies data and handles all outward actions through the callback.
private var controller: MenuController?

final class MenuController: NSObject, NSMenuDelegate {
    let state = MenuState()
    let item: NSStatusItem
    let menu = NSMenu()
    let callback: @convention(c) (Int32) -> Void
    let card = NSMenuItem()
    let refresh = NSMenuItem(title: "Refresh usage", action: #selector(refreshUsage), keyEquivalent: "r")
    let breakdown = NSMenuItem(title: "Model & token breakdown", action: nil, keyEquivalent: "")
    var tracking = false

    init(callback: @escaping @convention(c) (Int32) -> Void) {
        self.callback = callback
        item = NSStatusBar.system.statusItem(withLength: NSStatusItem.variableLength)
        super.init()
        item.isVisible = false
        item.autosaveName = "BridgeMenuBar"
        item.button?.image = Self.templateIcon()
        item.button?.imagePosition = .imageLeading
        item.button?.toolTip = "Bridge usage"
        item.button?.setAccessibilityLabel("Bridge usage menu")
        menu.delegate = self
        menu.autoenablesItems = false
        menu.addItem(card)
        menu.addItem(breakdown)
        menu.addItem(.separator())
        refresh.target = self
        menu.addItem(refresh)
        addAction("Menu Bar Settings…", #selector(openSettings), key: ",")
        addAction("Open Bridge", #selector(openBridge))
        menu.addItem(.separator())
        addAction("Quit Bridge", #selector(quit), key: "q")
        item.menu = menu
        rebuildCard()
    }

    static func templateIcon() -> NSImage {
        let image = NSImage(size: NSSize(width: 18, height: 18), flipped: false) { _ in
            NSColor.black.setStroke()
            let path = NSBezierPath()
            path.lineWidth = 1.6
            path.lineCapStyle = .round
            path.lineJoinStyle = .round
            path.move(to: NSPoint(x: 2, y: 4))
            path.line(to: NSPoint(x: 2, y: 13))
            path.curve(to: NSPoint(x: 16, y: 13), controlPoint1: NSPoint(x: 5, y: 6), controlPoint2: NSPoint(x: 13, y: 6))
            path.line(to: NSPoint(x: 16, y: 4))
            path.move(to: NSPoint(x: 1, y: 4))
            path.line(to: NSPoint(x: 17, y: 4))
            path.move(to: NSPoint(x: 6, y: 4))
            path.line(to: NSPoint(x: 6, y: 9))
            path.move(to: NSPoint(x: 12, y: 4))
            path.line(to: NSPoint(x: 12, y: 9))
            path.stroke()
            return true
        }
        image.isTemplate = true
        return image
    }

    func addAction(_ title: String, _ action: Selector, key: String = "") {
        let row = NSMenuItem(title: title, action: action, keyEquivalent: key)
        row.target = self
        menu.addItem(row)
    }

    func rebuildCard() {
        let view = NSHostingView(rootView: MenuCard(state: state))
        let height = view.fittingSize.height
        view.frame = NSRect(x: 0, y: 0, width: 350, height: height)
        // The menu stays inside even a small display; every row remains
        // reachable when data grows or the user enables all detail sections.
        let scroll = NSScrollView(frame: NSRect(x: 0, y: 0, width: 350,
            height: min(height, max(180, (item.button?.window?.screen?.visibleFrame.height ?? 800) - 210))))
        scroll.drawsBackground = false
        scroll.hasVerticalScroller = true
        scroll.autohidesScrollers = true
        scroll.documentView = view
        scroll.contentView.scroll(to: NSPoint(x: 0, y: view.isFlipped ? 0 : max(0, height - scroll.contentView.bounds.height)))
        scroll.reflectScrolledClipView(scroll.contentView)
        card.view = scroll
        let details = NSMenu()
        let usage = state.presentation.usage
        let showCost = state.presentation.settings.showCost
        for (title, period) in [("Today", usage?.today), ("Last 30 days", usage?.month)] {
            details.addItem(withTitle: title, action: nil, keyEquivalent: "").isEnabled = false
            if let period = period, !period.models.isEmpty {
                for model in period.models {
                    let row = NSMenuItem(title: model.model, action: nil, keyEquivalent: "")
                    let values = NSMenu()
                    for (label, metric) in [("Total tokens", model.totalTokens), ("Input", model.inputTokens),
                        ("Output", model.outputTokens), ("Cache", model.cacheTokens)] {
                        values.addItem(withTitle: "\(label): \(countLabel(metric))", action: nil, keyEquivalent: "").isEnabled = false
                    }
                    if showCost { values.addItem(withTitle: "Cost: \(moneyLabel(model.costMicrousd))", action: nil, keyEquivalent: "").isEnabled = false }
                    row.submenu = values
                    details.addItem(row)
                }
            } else {
                details.addItem(withTitle: "No recorded usage", action: nil, keyEquivalent: "").isEnabled = false
            }
            if title == "Today" { details.addItem(.separator()) }
        }
        breakdown.submenu = details
        breakdown.isHidden = !state.presentation.settings.codexEnabled || !state.presentation.settings.showTokens
    }

    func update(_ presentation: Presentation) {
        state.presentation = presentation
        item.isVisible = presentation.settings.enabled || tracking
        refresh.isEnabled = presentation.settings.codexEnabled && !presentation.refreshing
        refresh.title = presentation.refreshing ? "Refreshing usage…" : "Refresh usage"
        let settings = presentation.settings
        let usage = presentation.usage
        var title = ""
        if settings.codexEnabled {
            if settings.displayMode == "cost" {
                title = usage.map { moneyLabel($0.today.costMicrousd) } ?? "—"
                if title == "Unavailable" { title = "—" }
            } else if settings.displayMode != "icon" {
                let window = usage?.windows.first { $0.id == settings.quotaWindow }
                let now = Int64(Date().timeIntervalSince1970)
                let fresh = presentation.error == nil && usage?.observedAt.map { now - $0 < 600 && now >= $0 } == true
                    && (window?.resetsAt.map { $0 > now } ?? true)
                let quota = fresh ? window?.usedPercent.current : nil
                title = quota.map { String(format: "%.0f%%", settings.displayMode == "used" ? $0 : max(0, 100 - $0)) } ?? "—"
            }
        }
        item.button?.title = title.isEmpty ? "" : " \(title)"
        item.button?.font = NSFont.monospacedDigitSystemFont(ofSize: 12, weight: .medium)
        item.button?.toolTip = title.isEmpty ? "Bridge usage" : "Bridge · Codex · \(title) \(settings.displayMode)"
        // Avoid structural changes during menu tracking; data updates in place.
        if !tracking { rebuildCard() }
        else if let scroll = card.view as? NSScrollView, let view = scroll.documentView as? NSHostingView<MenuCard> {
            view.layoutSubtreeIfNeeded()
            view.setFrameSize(NSSize(width: 350, height: view.fittingSize.height))
        }
    }

    func menuWillOpen(_ menu: NSMenu) {
        rebuildCard()
        tracking = true
        callback(4)
    }
    func menuDidClose(_ menu: NSMenu) { tracking = false; item.isVisible = state.presentation.settings.enabled }
    @objc func refreshUsage() { callback(1) }
    @objc func openSettings() { callback(2) }
    @objc func openBridge() { callback(3) }
    @objc func quit() { callback(5) }
    func destroy() { menu.cancelTracking(); NSStatusBar.system.removeStatusItem(item) }
}

@_cdecl("bridge_menu_bar_create")
public func createMenuBar(_ callback: @escaping @convention(c) (Int32) -> Void) -> Bool {
    guard Thread.isMainThread else { return false }
    if controller == nil { controller = MenuController(callback: callback) }
    return true
}

@_cdecl("bridge_menu_bar_update")
public func updateMenuBar(_ bytes: UnsafePointer<UInt8>, _ count: Int) -> Bool {
    guard Thread.isMainThread, let controller = controller, count >= 0, count <= 1_048_576,
          let value = try? JSONDecoder().decode(Presentation.self, from: Data(bytes: bytes, count: count)),
          value.settings.schemaVersion == 1, value.usage == nil || value.usage?.schemaVersion == 1 else { return false }
    controller.update(value)
    return true
}

@_cdecl("bridge_menu_bar_show")
public func showMenuBar() {
    guard Thread.isMainThread else { return }
    // An explicit Open Meter action can temporarily reveal a hidden item;
    // dismissal restores the saved visibility preference.
    controller?.item.isVisible = true
    controller?.item.button?.performClick(nil)
}

@_cdecl("bridge_menu_bar_destroy")
public func destroyMenuBar() {
    guard Thread.isMainThread else { return }
    controller?.destroy()
    controller = nil
}

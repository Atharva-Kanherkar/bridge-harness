import AppKit
import SwiftUI

// Owns native presentation only. AppKit retains the status item until destroy;
// Rust supplies data and handles all outward actions through the callback.
private var controller: MenuController?

final class ModelBreakdownMenu: NSObject, NSMenuDelegate {
    let item = NSMenuItem(title: "Model & token breakdown", action: nil, keyEquivalent: "")
    let menu = NSMenu()
    let state: MenuState
    private var tracking = false

    init(state: MenuState) {
        self.state = state
        super.init()
        menu.delegate = self
        item.submenu = menu
        update()
    }

    func update() {
        // Cancelling a child NSMenu also ends its parent's tracking session.
        // Keep the open child intact; menuNeedsUpdate reads the latest state on
        // its next open. No menu structure changes from open/close callbacks.
        guard !tracking else { return }
        item.isHidden = state.presentation.settings.enabledProviders.isEmpty || !state.presentation.settings.showTokens
        menu.removeAllItems()
        let usage = state.presentation.selectedUsage
        let showCost = state.presentation.settings.showCost
        for (title, period) in [("Today", usage?.today), ("Last 30 days", usage?.month)] {
            menu.addItem(withTitle: title, action: nil, keyEquivalent: "").isEnabled = false
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
                    menu.addItem(row)
                }
            } else {
                menu.addItem(withTitle: "No recorded usage", action: nil, keyEquivalent: "").isEnabled = false
            }
            if title == "Today" { menu.addItem(.separator()) }
        }
    }

    func menuNeedsUpdate(_ menu: NSMenu) {
        if menu === self.menu { update() }
    }

    func menuWillOpen(_ menu: NSMenu) {
        if menu === self.menu { tracking = true }
    }

    func menuDidClose(_ menu: NSMenu) {
        if menu === self.menu { tracking = false }
    }
}

final class MenuController: NSObject, NSMenuDelegate {
    let state = MenuState()
    let item: NSStatusItem
    let menu = NSMenu()
    let callback: @convention(c) (Int32) -> Void
    let card = NSMenuItem()
    let refresh = NSMenuItem(title: "Refresh usage", action: #selector(refreshUsage), keyEquivalent: "r")
    lazy var breakdown = ModelBreakdownMenu(state: state)
    var tracking = false

    init(callback: @escaping @convention(c) (Int32) -> Void) {
        self.callback = callback
        item = NSStatusBar.system.statusItem(withLength: NSStatusItem.variableLength)
        super.init()
        state.selectProvider = { [weak self] id in
            guard let index = ["codex", "claude", "cursor", "opencode"].firstIndex(of: id) else { return }
            self?.callback(Int32(100 + index))
        }
        item.isVisible = false
        item.autosaveName = "BridgeMenuBar"
        item.button?.image = Self.templateIcon()
        item.button?.imagePosition = .imageLeading
        item.button?.toolTip = "Bridge usage"
        item.button?.setAccessibilityLabel("Bridge usage menu")
        menu.delegate = self
        menu.autoenablesItems = false
        menu.addItem(card)
        menu.addItem(breakdown.item)
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
        let image = NSImage(size: NSSize(width: 18, height: 18), flipped: true) { _ in
            // Foreground rectangles from assets/bridge-icon.svg, with its
            // background omitted. Center the original proportions at 16pt wide.
            let scale: CGFloat = 16 / 640
            let top = (18 - 408 * scale) / 2
            NSColor.black.setFill()
            for rect in [
                NSRect(x: 264, y: 400, width: 96, height: 312),
                NSRect(x: 664, y: 400, width: 96, height: 312),
                NSRect(x: 192, y: 304, width: 640, height: 104),
            ] {
                NSRect(x: 1 + (rect.minX - 192) * scale,
                    y: top + (rect.minY - 304) * scale,
                    width: rect.width * scale, height: rect.height * scale).fill()
            }
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
        breakdown.update()
    }

    func update(_ presentation: Presentation) {
        let providerChanged = state.presentation.settings.activeProvider != presentation.settings.activeProvider
        state.presentation = presentation
        item.isVisible = presentation.settings.enabled || tracking
        refresh.isEnabled = !presentation.settings.enabledProviders.isEmpty && !presentation.refreshing
        refresh.title = presentation.refreshing ? "Refreshing usage…" : "Refresh usage"
        let settings = presentation.settings
        let usage = presentation.selectedUsage
        var title = ""
        if !settings.enabledProviders.isEmpty {
            if settings.displayMode == "cost" {
                title = usage.map { moneyLabel($0.today.costMicrousd) } ?? "—"
                if title == "Unavailable" { title = "—" }
            } else if settings.displayMode != "icon" {
                let now = Int64(Date().timeIntervalSince1970)
                let window = usage?.menuWindow(settings.quotaWindow, now: now)
                let fresh = presentation.error == nil && usage?.observedAt.map { now - $0 < 600 && now >= $0 } == true
                    && (window?.resetsAt.map { $0 > now } ?? true)
                let quota = fresh ? window?.usedPercent.current : nil
                title = quota.map { String(format: "%.0f%%", settings.displayMode == "used" ? $0 : max(0, 100 - $0)) } ?? "—"
            }
        }
        item.button?.title = title.isEmpty ? "" : " \(title)"
        item.button?.font = NSFont.monospacedDigitSystemFont(ofSize: 12, weight: .medium)
        let windowLabel = usage?.menuWindow(settings.quotaWindow, now: Int64(Date().timeIntervalSince1970))?.label ?? "Quota"
        let metricLabel = settings.displayMode == "cost" ? "Today" : windowLabel
        item.button?.toolTip = title.isEmpty ? "Bridge usage" : "Bridge · \(providerName(settings.activeProvider ?? "")) · \(metricLabel) · \(title) \(settings.displayMode)"
        // Avoid structural changes during menu tracking; data updates in place.
        if !tracking { rebuildCard() }
        else if let scroll = card.view as? NSScrollView, let view = scroll.documentView as? NSHostingView<MenuCard> {
            if providerChanged {
                breakdown.update()
            }
            view.layoutSubtreeIfNeeded()
            view.setFrameSize(NSSize(width: 350, height: view.fittingSize.height))
        }
    }

    func menuWillOpen(_ menu: NSMenu) {
        guard menu === self.menu else { return }
        rebuildCard()
        tracking = true
        callback(4)
    }
    func menuDidClose(_ menu: NSMenu) {
        guard menu === self.menu else { return }
        tracking = false
        item.isVisible = state.presentation.settings.enabled
    }
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
          value.settings.schemaVersion == 1, value.usage == nil || value.usage?.schemaVersion == 1,
          value.usage?.providers.allSatisfy({ $0.schemaVersion == 1 }) ?? true else { return false }
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

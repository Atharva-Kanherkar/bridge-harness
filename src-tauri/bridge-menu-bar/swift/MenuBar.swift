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
    var parentTracking = false

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
        item.isEnabled = !state.showingOverview && state.presentation.selectedUsage != nil && state.presentation.settings.showTokens
        guard !parentTracking else { return }
        item.isHidden = state.showingOverview || state.presentation.selectedUsage == nil || !state.presentation.settings.showTokens
        rebuildContents()
    }

    private func rebuildContents() {
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
        if menu === self.menu && !tracking { rebuildContents() }
    }

    func menuWillOpen(_ menu: NSMenu) {
        MenuAppearance.pin(menu)
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
    // A detached custom view must never reveal NSMenuItem's default title.
    let card = MenuCardItem(title: "", action: nil, keyEquivalent: "")
    let refresh = NSMenuItem(title: "Refresh usage", action: #selector(refreshUsage), keyEquivalent: "r")
    let refreshOptions = NSMenu(title: "Refresh interval")
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
        state.openSettings = { [weak self] in self?.callback(2) }
        state.surfaceChanged = { [weak self] in
            guard let self = self else { return }
            self.breakdown.update()
            (self.card.view as? MenuCardScrollView)?.updateSize(maximumHeight: self.cardMaximumHeight, resetScroll: true)
        }
        state.contentChanged = { [weak self] in
            guard let self = self else { return }
            (self.card.view as? MenuCardScrollView)?.updateSize(maximumHeight: self.cardMaximumHeight)
        }
        item.isVisible = false
        item.autosaveName = "BridgeMenuBar"
        item.button?.image = Self.templateIcon()
        item.button?.imagePosition = .imageLeading
        item.button?.toolTip = "Bridge usage"
        item.button?.setAccessibilityTitle("Bridge usage menu")
        menu.delegate = self
        menu.autoenablesItems = false
        menu.addItem(card)
        menu.addItem(breakdown.item)
        menu.addItem(.separator())
        refresh.target = self
        menu.addItem(refresh)
        let interval = NSMenuItem(title: "Refresh interval", action: nil, keyEquivalent: "")
        for (index, title) in ["Manually", "Every minute", "Every 5 minutes", "Every 15 minutes", "Every 30 minutes"].enumerated() {
            let row = NSMenuItem(title: title, action: #selector(changeRefreshInterval(_:)), keyEquivalent: "")
            row.tag = 200 + index
            row.target = self
            refreshOptions.addItem(row)
        }
        interval.submenu = refreshOptions
        menu.addItem(interval)
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

    static func meterIcon(_ presentation: Presentation) -> NSImage {
        let image = NSImage(size: NSSize(width: 18, height: 18), flipped: true) { _ in
            let usage = presentation.selectedUsage
            for index in 0..<2 {
                let rect = NSRect(x: 1, y: 3 + index * 7, width: 16, height: 5)
                NSColor.black.withAlphaComponent(0.25).setFill()
                NSBezierPath(roundedRect: rect, xRadius: 1, yRadius: 1).fill()
                NSColor.black.setFill()
                let window = usage?.windows.dropFirst(index).first
                if let usage = usage, let window = window,
                   let fill = QuotaDisplay(window, usage: usage, mode: presentation.settings.quotaDisplayMode ?? "used", now: Date(), failed: presentation.error != nil).fill {
                    NSBezierPath(roundedRect: NSRect(x: rect.minX, y: rect.minY, width: rect.width * fill, height: rect.height), xRadius: 1, yRadius: 1).fill()
                } else {
                    NSRect(x: 8, y: rect.minY + 2, width: 2, height: 1).fill()
                }
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
        card.view = MenuCardScrollView(document: view, width: 350, maximumHeight: cardMaximumHeight)
        breakdown.update()
    }

    var cardMaximumHeight: CGFloat {
        MenuCardScrollView.maximumHeight(on: card.view?.window?.screen ?? item.button?.window?.screen ?? NSScreen.main)
    }

    func update(_ presentation: Presentation) {
        let providerChanged = state.presentation.settings.activeProvider != presentation.settings.activeProvider
        let usageAvailabilityChanged = (state.presentation.selectedUsage == nil) != (presentation.selectedUsage == nil)
        state.presentation = presentation
        item.isVisible = presentation.settings.enabled || tracking
        refresh.isEnabled = !presentation.settings.enabledProviders.isEmpty && !presentation.refreshing
        refresh.title = presentation.refreshing ? "Refreshing usage…" : "Refresh usage"
        let status = MenuStatus(presentation, now: Int64(Date().timeIntervalSince1970))
        let icon = presentation.settings.iconStyle == "meter" ? Self.meterIcon(presentation) : Self.templateIcon()
        let layout = StatusLayout(presentation, now: Int64(Date().timeIntervalSince1970))
        if layout.custom && layout.hasContent {
            if layout.requiresTemplateImage {
                item.button?.attributedTitle = NSAttributedString(string: "")
                item.button?.image = layout.templateImage(icon: icon)
                item.button?.imagePosition = .imageOnly
            } else {
                item.button?.image = layout.hasLeadingIcon ? icon : nil
                item.button?.attributedTitle = layout.attributedTitle(icon: icon, omitLeadingIcon: true)
                item.button?.imagePosition = layout.hasLeadingIcon ? .imageLeading : .noImage
            }
        } else {
            item.button?.image = icon
            item.button?.imagePosition = .imageLeading
            item.button?.attributedTitle = NSAttributedString(string: "")
            item.button?.title = status.title.isEmpty ? "" : " \(status.title)"
        }
        item.button?.font = NSFont.monospacedDigitSystemFont(ofSize: 12, weight: .medium)
        let accessible = layout.custom && layout.hasContent ? layout.accessibilityTitle : status.accessibilityTitle
        item.button?.toolTip = accessible
        item.button?.setAccessibilityTitle(accessible)
        if !tracking {
            for (index, seconds) in [0, 60, 300, 900, 1800].enumerated() {
                refreshOptions.items[index].state = presentation.settings.refreshSeconds == seconds ? .on : .off
            }
        }
        // Avoid structural changes during menu tracking; data updates in place.
        if !tracking { rebuildCard() }
        else if let scroll = card.view as? MenuCardScrollView {
            if providerChanged || usageAvailabilityChanged {
                breakdown.update()
            }
            scroll.updateSize(maximumHeight: cardMaximumHeight, resetScroll: providerChanged)
        }
    }

    func menuWillOpen(_ menu: NSMenu) {
        MenuAppearance.pin(menu)
        guard menu === self.menu else { return }
        state.showingOverview = state.presentation.settings.openToOverview ?? true
        rebuildCard()
        tracking = true
        breakdown.parentTracking = true
        callback(4)
    }
    func menuDidClose(_ menu: NSMenu) {
        guard menu === self.menu else { return }
        tracking = false
        breakdown.parentTracking = false
        item.isVisible = state.presentation.settings.enabled
    }
    @objc func refreshUsage() { callback(1) }
    @objc func changeRefreshInterval(_ sender: NSMenuItem) { callback(Int32(sender.tag)) }
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
    // Thirty days of model-level history across four providers can exceed the
    // old 1 MiB quota-only snapshot. Keep an explicit bound for the expanded UI.
    guard Thread.isMainThread, let controller = controller, count >= 0, count <= 8_388_608,
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

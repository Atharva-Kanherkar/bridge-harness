import AppKit

extension Presentation {
    func forMenuProvider(_ provider: String?) -> Presentation {
        guard let provider = provider else { return self }
        var result = self
        result.settings.selectedProvider = provider
        if !result.settings.visibleProviders.contains(provider) {
            result.settings.pinnedProviders = result.settings.normalizedPinnedProviders + [provider]
        }
        return result
    }

    func forStatusProvider(_ provider: String?) -> Presentation {
        guard let provider = provider else { return self }
        var result = self
        result.settings.pinnedProviders = [provider]
        result.settings.statusLayout = []
        return result
    }
}

// CodexBar-style stable per-provider status items. All instances consume the
// same backend snapshot; switching a detail tab never changes an icon's owner.
final class MenuBarController {
    let combined: MenuController
    private(set) var providers: [String: MenuController] = [:]
    private var latest: Presentation?
    private let callback: @convention(c) (Int32) -> Void
    private var reconciling = false
    private var destroyed = false

    init(callback: @escaping @convention(c) (Int32) -> Void) {
        self.callback = callback
        combined = MenuController(callback: callback)
        combined.didClose = { [weak self] in self?.scheduleReconcile() }
    }

    static func providerIDs(_ settings: MenuSettings) -> [String] {
        guard settings.enabled, settings.separateProviderIcons ?? false else { return [] }
        return settings.visibleProviders.filter { settings.isProviderEnabled($0) }
    }

    func update(_ presentation: Presentation) {
        guard !destroyed else { return }
        latest = presentation
        // Creation/configuration may synchronously re-enter. This guard is set
        // before constructing any item, so a provider cannot be vended twice.
        guard !reconciling else { return }
        reconciling = true
        defer { reconciling = false }
        let ids = Self.providerIDs(presentation.settings)
        let tracking = combined.tracking || providers.values.contains { $0.tracking }
        if !tracking {
            for id in Array(providers.keys) where !ids.contains(id) {
                providers.removeValue(forKey: id)?.destroy()
            }
            for id in ids where providers[id] == nil {
                let item = MenuController(statusProvider: id, callback: callback)
                providers[id] = item
                item.didClose = { [weak self] in self?.scheduleReconcile() }
            }
        }
        // Keep topology/visibility stable until every tracking menu has closed.
        combined.update(presentation, visible: tracking ? combined.visibilityRequested : presentation.settings.enabled && ids.isEmpty)
        for (id, item) in providers {
            item.update(presentation, visible: tracking ? item.visibilityRequested : ids.contains(id))
        }
    }

    private func scheduleReconcile() {
        // Do not release the delegate while AppKit is still returning from close.
        DispatchQueue.main.async { [weak self] in
            guard let self = self, let latest = self.latest else { return }
            self.update(latest)
        }
    }

    func show() {
        let first = latest.flatMap { Self.providerIDs($0.settings).first }.flatMap { providers[$0] }
        let target = first ?? combined
        target.item.isVisible = true
        target.item.button?.performClick(nil)
    }

    func destroy() {
        destroyed = true
        combined.destroy()
        providers.values.forEach { $0.destroy() }
        providers.removeAll()
    }
}

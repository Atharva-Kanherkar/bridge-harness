import Foundation

// One payload drives visible text, the tooltip and VoiceOver, following
// CodexBar's MenuBarLayoutRenderedTitle / applyMenuBarLayoutContent boundary.
struct MenuStatus {
    let title: String
    let accessibilityTitle: String

    init(_ presentation: Presentation, now: Int64) {
        let settings = presentation.settings
        guard let provider = settings.statusProvider else {
            title = ""
            accessibilityTitle = "Bridge usage menu, no providers enabled"
            return
        }
        let prefix = "Bridge usage menu, \(providerName(provider))"
        guard settings.isProviderEnabled(provider) else {
            title = settings.displayMode == "icon" ? "" : "—"
            accessibilityTitle = "\(prefix), disconnected"
            return
        }
        let usage = presentation.statusUsage
        if settings.displayMode == "icon" {
            title = ""
            accessibilityTitle = prefix
        } else if settings.displayMode == "cost" {
            let metric = usage?.today.costMicrousd ?? .unavailable
            let amount = moneyLabel(metric)
            title = amount == "Unavailable" ? "—" : amount
            if amount == "Unavailable" {
                accessibilityTitle = "\(prefix), today's cost unavailable"
            } else {
                let estimate = metric.source == "estimated" ? "estimated " : ""
                accessibilityTitle = "\(prefix), today's \(estimate)cost \(amount.replacingOccurrences(of: "≈", with: ""))"
            }
        } else {
            let window = usage?.menuWindow(settings.quotaWindow, now: now)
            let label = window.map { quotaLabel($0, provider: usage?.provider ?? "") } ?? "Quota"
            let fresh = presentation.error == nil && usage?.error == nil
                && usage?.observedAt.map { now >= $0 && now - $0 < 600 } == true
                && (window?.resetsAt.map { $0 > now } ?? true)
            if let used = window?.usedPercent.current, fresh {
                let remaining = settings.displayMode != "used"
                title = quotaPercentLabel(remaining ? max(0, 100 - used) : used)
                accessibilityTitle = "\(prefix), \(label), \(title) \(remaining ? "remaining" : "used")"
            } else {
                title = "—"
                let hasObservation = window?.usedPercent.value != nil && window?.usedPercent.status != "unavailable"
                accessibilityTitle = "\(prefix), \(label) \(hasObservation ? "stale" : "unavailable")"
            }
        }
    }
}

import Foundation

struct Metric: Decodable {
    var value: Double?
    var source: String?
    var status: String
    var current: Double? { status == "current" ? value : nil }
    var qualifier: String? {
        if status == "stale" { return "Stale" }
        if status == "unavailable" { return "Unavailable" }
        return source == "estimated" ? "Estimated" : nil
    }
    static let unavailable = Metric(value: nil, source: nil, status: "unavailable")
}

struct QuotaWindow: Decodable, Identifiable {
    var id: String
    var label: String
    var usedPercent: Metric
    var resetsAt: Int64?
    var windowMinutes: Int64?
}

struct ModelUsage: Decodable, Identifiable {
    var model: String
    var inputTokens: Metric
    var outputTokens: Metric
    var cacheTokens: Metric
    var totalTokens: Metric
    var costMicrousd: Metric
    var id: String { model }
}

struct UsagePeriod: Decodable {
    var tokens: Metric
    var costMicrousd: Metric
    var models: [ModelUsage]
}

struct UsageDay: Decodable, Identifiable {
    var day: String
    var usage: UsagePeriod
    var id: String { day }
}

struct AccountMetric: Decodable, Identifiable {
    var id: String
    var label: String
    var value: Metric
}

struct ProviderOverviews: Decodable {
    var schemaVersion: Int
    var generatedAt: Int64
    var providers: [UsageOverview]
}

struct UsageOverview: Decodable {
    var schemaVersion: Int
    var generatedAt: Int64
    var provider: String
    var account: String?
    var plan: String?
    var observedAt: Int64?
    var quotaSource: String? = nil
    var windows: [QuotaWindow]
    var accountMetrics: [AccountMetric]?
    var today: UsagePeriod
    var month: UsagePeriod
    var daily: [UsageDay]? = nil
    var coverage: String
    var error: String?

    func menuWindow(_ preference: String, now: Int64) -> QuotaWindow? {
        if preference == "fiveHour" { return windows.first { $0.windowMinutes == 300 } }
        if preference != "auto" { return windows.first { $0.id == preference } }
        return windows.first { $0.usedPercent.current != nil && ($0.resetsAt.map { $0 > now } ?? true) }
            ?? windows.first
    }
}

struct MenuSettings: Decodable {
    var schemaVersion: Int
    var enabled: Bool
    var codexEnabled: Bool
    var claudeEnabled: Bool
    var cursorEnabled: Bool
    var opencodeEnabled: Bool
    var selectedProvider: String
    var pinnedProviders: [String]? = nil
    var opencodeWorkspace: String?
    var enabledProviders: [String] {
        [("codex", codexEnabled), ("claude", claudeEnabled), ("cursor", cursorEnabled), ("opencode", opencodeEnabled)].filter { $0.1 }.map { $0.0 }
    }
    var normalizedPinnedProviders: [String] {
        let requested = pinnedProviders ?? ["codex", "claude", "cursor"]
        return requested.reduce(into: [String]()) { result, provider in
            if !provider.isEmpty && !result.contains(provider) { result.append(provider) }
        }
    }
    var visibleProviders: [String] {
        normalizedPinnedProviders
    }
    // Local detail selection for a separate icon; never changes saved favorites.
    var menuProviderOverride: String? = nil
    var activeProvider: String? { menuProviderOverride ?? (visibleProviders.contains(selectedProvider) ? selectedProvider : visibleProviders.first) }
    // The status item belongs to the first favorite, independently of which
    // detail tab is open. A disconnected favorite must not expose cached data
    // or silently substitute another account; no favorites uses the first enabled.
    var statusProvider: String? { visibleProviders.first ?? enabledProviders.first }
    func isProviderEnabled(_ provider: String) -> Bool { enabledProviders.contains(provider) }
    var displayMode: String
    var quotaDisplayMode: String? = nil
    var openToOverview: Bool? = nil
    var iconStyle: String? = nil
    var statusLayout: [[String]]? = nil
    var quotaWindow: String
    var showAccount: Bool
    var showTokens: Bool
    var showCost: Bool
    var showHistory: Bool? = nil
    var showOverviewSummary: Bool? = nil
    var separateProviderIcons: Bool? = nil
    var refreshSeconds: UInt64
    static let initial = MenuSettings(schemaVersion: 1, enabled: true, codexEnabled: true, claudeEnabled: false, cursorEnabled: false, opencodeEnabled: false, selectedProvider: "codex", opencodeWorkspace: nil,
        displayMode: "used", quotaWindow: "auto", showAccount: true,
        showTokens: true, showCost: true, refreshSeconds: 300)
}

struct Presentation: Decodable {
    var settings: MenuSettings
    var usage: ProviderOverviews?
    var selectedUsage: UsageOverview? {
        guard let provider = settings.activeProvider, settings.isProviderEnabled(provider) else { return nil }
        return usage?.providers.first { $0.provider == provider }
    }
    var statusUsage: UsageOverview? {
        guard let provider = settings.statusProvider, settings.isProviderEnabled(provider) else { return nil }
        return usage?.providers.first { $0.provider == provider }
    }
    var refreshing: Bool
    var error: String?
}

final class MenuState: ObservableObject {
    struct HistorySelection: Equatable {
        var day = "all"
        var metric = "tokens"
    }

    var selectProvider: (String) -> Void = { _ in }
    var openSettings: () -> Void = { }
    var surfaceChanged: () -> Void = { }
    var contentChanged: () -> Void = { }
    private var historySelections: [String: HistorySelection] = [:]
    @Published var showingOverview = true
    @Published var presentation = Presentation(settings: .initial, usage: nil, refreshing: false, error: nil)

    func select(_ id: String) {
        let previous = showingOverview ? "overview" : presentation.settings.activeProvider
        guard previous != id else { return }
        showingOverview = id == "overview"
        if !showingOverview { selectProvider(id) }
        // A different provider fades when its snapshot arrives. Only switching
        // Overview to/from the already loaded provider needs an immediate fade.
        if showingOverview || presentation.settings.activeProvider == id { surfaceChanged() }
    }

    func historySelection(for provider: String) -> HistorySelection {
        historySelections[provider] ?? HistorySelection()
    }

    func setHistoryDay(_ day: String, for provider: String) {
        var selection = historySelection(for: provider)
        guard selection.day != day else { return }
        selection.day = day
        objectWillChange.send()
        historySelections[provider] = selection
    }

    func setHistoryMetric(_ metric: String, for provider: String) {
        var selection = historySelection(for: provider)
        guard selection.metric != metric else { return }
        selection.metric = metric
        objectWillChange.send()
        historySelections[provider] = selection
    }
}

// Keep legacy wire IDs for other Bridge consumers, while using the provider's
// actual rolling-window duration in this independent menu presentation.
func quotaLabel(_ window: QuotaWindow, provider: String) -> String {
    if window.id == "session" && window.windowMinutes == 300 { return "5-hour" }
    if provider == "codex" && window.label == "Session" { return "Rolling" }
    if provider == "codex" {
        return window.label.replacingOccurrences(of: "gpt[- ]reserve", with: "GPT Reserve", options: [.regularExpression, .caseInsensitive])
    }
    return window.label
}

// CodexBar's uniform-time pace model: reserve is the gap between elapsed
// allowance and actual use, not extra credits or a separate token allowance.
func quotaPace(_ window: QuotaWindow, used: Double?, now: Date) -> String? {
    guard let used = used, used < 100, let minutes = window.windowMinutes, minutes > 0,
          let reset = window.resetsAt else { return nil }
    let duration = Double(minutes) * 60
    let remaining = Double(reset) - now.timeIntervalSince1970
    guard remaining > 0 && remaining <= duration else { return nil }
    let expected = (duration - remaining) / duration * 100
    guard expected >= 3 else { return nil }
    let delta = max(0, used) - expected
    if abs(delta) <= 2 { return "On pace" }
    return String(format: "%.0f%% in %@", abs(delta), delta < 0 ? "reserve" : "deficit")
}

struct QuotaDisplay {
    let used: Double?
    let fill: Double?
    let valueLabel: String
    let counterpart: String?
    let expired: Bool

    init(_ window: QuotaWindow, usage: UsageOverview, mode: String, now: Date, failed: Bool) {
        expired = window.resetsAt.map { Double($0) <= now.timeIntervalSince1970 } ?? false
        let old = usage.observedAt.map { now.timeIntervalSince1970 - Double($0) >= 600 || Double($0) > now.timeIntervalSince1970 } ?? true
        used = !failed && usage.error == nil && !expired && !old ? window.usedPercent.current : nil
        if let used = used {
            let remaining = max(0, 100 - used)
            let showRemaining = mode == "remaining"
            fill = min(100, max(0, showRemaining ? remaining : used)) / 100
            valueLabel = "\(quotaPercentLabel(showRemaining ? remaining : used)) \(showRemaining ? "left" : "used")"
            counterpart = "\(quotaPercentLabel(showRemaining ? used : remaining)) \(showRemaining ? "used" : "left")"
        } else {
            fill = nil
            let recorded = window.usedPercent.value != nil && window.usedPercent.status != "unavailable"
            valueLabel = recorded ? "Stale" : "Unavailable"
            counterpart = nil
        }
    }
}

func quotaPercentLabel(_ value: Double) -> String {
    if value > 0 && value < 0.01 { return "<0.01%" }
    if value > 99.99 && value < 100 { return ">99.99%" }
    if (value > 0 && value < 1) || (value > 99 && value < 100) { return String(format: "%.2f%%", value) }
    return String(format: "%.0f%%", value)
}

func countLabel(_ metric: Metric) -> String {
    guard let value = metric.value, metric.status != "unavailable" else { return "Unavailable" }
    let format = NumberFormatter()
    format.numberStyle = .decimal
    format.locale = Locale(identifier: "en_US_POSIX")
    format.usesGroupingSeparator = true
    format.groupingSize = 3
    format.secondaryGroupingSize = 3
    format.maximumFractionDigits = 0
    let label = format.string(from: NSNumber(value: value)) ?? "Unavailable"
    return metric.qualifier.map { "\(label) · \($0.lowercased())" } ?? label
}

func compactCountLabel(_ metric: Metric, unit: String? = nil) -> String {
    guard let value = metric.value, metric.status != "unavailable" else { return "Unavailable" }
    let absolute = abs(value)
    let label: String
    if absolute >= 1_000_000_000 {
        label = String(format: absolute >= 10_000_000_000 ? "%.0fB" : "%.1fB", value / 1_000_000_000)
    } else if absolute >= 1_000_000 {
        label = String(format: absolute >= 10_000_000 ? "%.0fM" : "%.1fM", value / 1_000_000)
    } else if absolute >= 1_000 {
        label = String(format: absolute >= 10_000 ? "%.0fK" : "%.1fK", value / 1_000)
    } else {
        label = String(format: "%.0f", value)
    }
    let cleaned = label.replacingOccurrences(of: ".0K", with: "K")
        .replacingOccurrences(of: ".0M", with: "M")
        .replacingOccurrences(of: ".0B", with: "B")
    let labeled = unit.map { "\(cleaned) \($0)" } ?? cleaned
    return metric.qualifier.map { "\(labeled) · \($0.lowercased())" } ?? labeled
}

func usageSummaryValues(_ period: UsagePeriod, showTokens: Bool, showCost: Bool) -> String {
    var values: [String] = []
    if showTokens {
        let tokens = compactCountLabel(period.tokens, unit: "tokens")
        values.append(tokens == "Unavailable" ? "Tokens unavailable" : tokens)
    }
    if showCost {
        let cost = moneyLabel(period.costMicrousd)
        values.append(cost == "Unavailable" ? "Cost unavailable" : cost)
    }
    return values.joined(separator: " · ")
}

func moneyLabel(_ metric: Metric) -> String {
    guard let value = metric.value, metric.status != "unavailable" else { return "Unavailable" }
    let label = String(format: "$%.2f", value / 1_000_000)
    let qualified = metric.source == "estimated" ? "≈\(label)" : label
    return metric.status == "stale" ? "\(qualified) · stale" : qualified
}

func providerName(_ id: String) -> String {
    if let known = ["codex": "Codex", "claude": "Claude", "cursor": "Cursor", "opencode": "OpenCode"][id] { return known }
    let readable = id.replacingOccurrences(of: "-", with: " ").replacingOccurrences(of: "_", with: " ")
    return readable.split(separator: " ").map { $0.prefix(1).uppercased() + $0.dropFirst() }.joined(separator: " ")
}

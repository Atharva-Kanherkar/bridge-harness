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

struct UsageOverview: Decodable {
    var schemaVersion: Int
    var generatedAt: Int64
    var provider: String
    var account: String?
    var plan: String?
    var observedAt: Int64?
    var windows: [QuotaWindow]
    var today: UsagePeriod
    var month: UsagePeriod
    var coverage: String
    var error: String?

    func menuWindow(_ preference: String, now: Int64) -> QuotaWindow? {
        if preference != "auto" { return windows.first { $0.id == preference } }
        return windows.first { $0.usedPercent.current != nil && ($0.resetsAt.map { $0 > now } ?? true) }
            ?? windows.first
    }
}

struct MenuSettings: Decodable {
    var schemaVersion: Int
    var enabled: Bool
    var codexEnabled: Bool
    var displayMode: String
    var quotaWindow: String
    var showAccount: Bool
    var showTokens: Bool
    var showCost: Bool
    var refreshSeconds: UInt64
    static let initial = MenuSettings(schemaVersion: 1, enabled: true, codexEnabled: true,
        displayMode: "remaining", quotaWindow: "auto", showAccount: true,
        showTokens: true, showCost: true, refreshSeconds: 300)
}

struct Presentation: Decodable {
    var settings: MenuSettings
    var usage: UsageOverview?
    var refreshing: Bool
    var error: String?
}

final class MenuState: ObservableObject {
    @Published var presentation = Presentation(settings: .initial, usage: nil, refreshing: false, error: nil)
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

func moneyLabel(_ metric: Metric) -> String {
    guard let value = metric.value, metric.status != "unavailable" else { return "Unavailable" }
    let label = String(format: "$%.2f", value / 1_000_000)
    let qualified = metric.source == "estimated" ? "≈\(label)" : label
    return metric.status == "stale" ? "\(qualified) · stale" : qualified
}

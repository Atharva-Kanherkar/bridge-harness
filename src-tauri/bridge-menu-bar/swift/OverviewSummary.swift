import SwiftUI

// Presentation-only totals over the backend's existing 30-day snapshot. Missing
// prices are excluded from the known subtotal, never converted to free usage.
struct OverviewSummary {
    let providerCount: Int
    let costProviderCount: Int
    let tokenProviderCount: Int
    let cost: Metric
    let tokens: Metric
    let costPartial: Bool
    let tokensPartial: Bool

    init(_ presentation: Presentation) {
        let providers = presentation.settings.enabledProviders
        let periods = providers.map { provider in
            presentation.usage?.providers.first { $0.provider == provider }?.month
        }
        providerCount = providers.count
        let costs = periods.compactMap { $0?.costMicrousd }
        let counts = periods.compactMap { $0?.tokens }
        func known(_ metric: Metric) -> Bool {
            metric.status != "unavailable" && metric.value.map { $0.isFinite && $0 >= 0 } == true
        }
        costProviderCount = costs.filter(known).count
        tokenProviderCount = counts.filter(known).count
        // A partial price can carry a known subtotal even within one provider.
        costPartial = costProviderCount < providers.count || costs.contains { $0.status != "current" }
        tokensPartial = tokenProviderCount < providers.count || counts.contains { $0.status != "current" }
        func sum(_ metrics: [Metric]) -> Metric {
            let available = metrics.filter(known)
            guard !available.isEmpty else { return .unavailable }
            let total = available.reduce(0) { $0 + ($1.value ?? 0) }
            guard total.isFinite else { return .unavailable }
            return Metric(value: total,
                          source: available.contains { $0.source == "estimated" } ? "estimated" : "measured",
                          status: available.contains { $0.status == "stale" } ? "stale" : "current")
        }
        cost = sum(costs)
        tokens = sum(counts)
    }

    var costLabel: String {
        let value = moneyLabel(cost)
        return costPartial && cost.value != nil && !value.hasPrefix("≈") ? "≈\(value)" : value
    }
    var tokenLabel: String {
        let value = compactCountLabel(tokens, unit: "tokens")
        return tokensPartial && tokens.value != nil ? "≈\(value)" : value
    }
}

struct OverviewSummaryCard: View {
    let presentation: Presentation
    var body: some View {
        let summary = OverviewSummary(presentation)
        let settings = presentation.settings
        if summary.providerCount > 0 && (settings.showCost || settings.showTokens) {
            VStack(alignment: .leading, spacing: 8) {
                Text("Usage & Spend · 30d").font(.system(size: 12, weight: .semibold)).foregroundColor(.secondary)
                if settings.showCost {
                    Text(summary.costLabel).font(.system(size: 23, weight: .bold)).monospacedDigit()
                    Text("\(summary.costProviderCount) of \(summary.providerCount) providers have spend")
                        .foregroundColor(.secondary)
                }
                if settings.showTokens { Text(summary.tokenLabel).foregroundColor(.secondary) }
                if settings.showCost && summary.cost.source == "estimated" {
                    Text("Includes estimated API cost").font(.system(size: 10)).foregroundColor(.secondary)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(12)
            .background(Color.accentColor.opacity(0.07))
            .clipShape(RoundedRectangle(cornerRadius: 12))
            .accessibilityElement(children: .combine)
            .accessibilityIdentifier("overview-usage-spend")
        }
    }
}

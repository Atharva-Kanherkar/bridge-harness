import SwiftUI

// CodexBar's history view selects a day to reveal its model breakdown. This
// compact native chart uses SwiftUI primitives to retain macOS 12 support.
struct DailyUsageView: View {
    let usage: UsageOverview
    let showCost: Bool
    let showTokens: Bool
    @Binding var selectedDay: String
    @Binding var chartMetric: String
    var days: [UsageDay] { usage.daily ?? [] }
    var costChart: Bool { showCost && (!showTokens || chartMetric == "cost") }
    var selected: UsagePeriod { days.first { $0.day == selectedDay }?.usage ?? usage.month }
    func metric(_ day: UsageDay) -> Metric { costChart ? day.usage.costMicrousd : day.usage.tokens }
    var chartDays: [UsageDay] { days.filter { metric($0).current != nil } }
    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack {
                Text("Daily history").fontWeight(.semibold)
                Spacer()
                if showCost && showTokens {
                    Picker("History metric", selection: $chartMetric) {
                        Text("Tokens").tag("tokens")
                        Text("Cost").tag("cost")
                    }.pickerStyle(.segmented).frame(width: 130).labelsHidden()
                }
            }
            if days.isEmpty {
                Text("No daily history recorded").font(.system(size: 11)).foregroundColor(.secondary)
                    .frame(height: 52, alignment: .center)
            } else if chartDays.isEmpty {
                Text(costChart ? "Cost history unavailable" : "Token history unavailable")
                    .font(.system(size: 11, weight: .medium))
                    .help(costChart ? "Bridge has no current cost values for the recorded days." : "Bridge has no current token totals for the recorded days.")
                .frame(maxWidth: .infinity, minHeight: 92, alignment: .leading)
            } else {
                let maximum = max(1, chartDays.compactMap { metric($0).current }.max() ?? 1)
                HStack(alignment: .bottom, spacing: 3) {
                    ForEach(days) { day in
                        let value = metric(day).current
                        Button(action: { selectedDay = selectedDay == day.day ? "all" : day.day }) {
                            VStack(spacing: 0) {
                                Spacer(minLength: 0)
                                if let value = value {
                                    RoundedRectangle(cornerRadius: 2)
                                        .fill(ProviderStyle.accent(usage.provider).opacity(selectedDay == day.day ? 1 : 0.65))
                                        .frame(height: max(2, 64 * min(1, value / maximum)))
                                } else {
                                    Text("–").font(.system(size: 8)).foregroundColor(.secondary)
                                }
                            }.frame(maxWidth: .infinity).frame(height: 66).contentShape(Rectangle())
                        }.buttonStyle(.plain)
                            .help("\(day.day): \(costChart ? moneyLabel(metric(day)) : countLabel(metric(day)) + " tokens")")
                            .accessibilityLabel("\(day.day), \(costChart ? moneyLabel(metric(day)) : countLabel(metric(day)) + " tokens")")
                    }
                }.accessibilityLabel("Daily \(costChart ? "cost" : "token") history")
                HStack {
                    Text(days.first!.day)
                    Spacer()
                    Text(days.last!.day)
                }.font(.system(size: 9)).foregroundColor(.secondary)
                Picker("History day", selection: Binding(get: { days.contains { $0.day == selectedDay } ? selectedDay : "all" }, set: { selectedDay = $0 })) {
                    Text("All recorded days").tag("all")
                    ForEach(days.reversed()) { day in Text(day.day).tag(day.day) }
                }.font(.system(size: 11))
                if selectedDay != "all" {
                    if showTokens { detail("Tokens", countLabel(selected.tokens)) }
                    if showCost { detail(usage.provider == "cursor" ? "API-rate cost" : "Cost", moneyLabel(selected.costMicrousd)) }
                }
                Text("Recorded days only · missing history is not zero usage")
                    .font(.system(size: 9)).foregroundColor(.secondary)
            }
        }
        .frame(height: days.isEmpty ? 88 : 184, alignment: .topLeading)
        .clipped()
    }
}

import SwiftUI

struct MenuCard: View {
    @ObservedObject var state: MenuState
    var body: some View {
        let presentation = state.presentation
        let settings = presentation.settings
        VStack(alignment: .leading, spacing: 13) {
            HStack(spacing: 7) {
                Image(systemName: "terminal.fill").font(.system(size: 14))
                Text("Usage").font(.system(size: 12, weight: .semibold))
                Spacer()
                if presentation.refreshing { ProgressView().controlSize(.small) }
                else { Text("Bridge").font(.system(size: 11)).foregroundColor(.secondary) }
            }
            if settings.enabledProviders.count > 1 {
                Picker("Provider", selection: Binding(get: { settings.activeProvider ?? "codex" }, set: { state.selectProvider($0) })) {
                    ForEach(settings.enabledProviders, id: \.self) { id in Text(providerName(id)).tag(id) }
                }.pickerStyle(.segmented).labelsHidden().accessibilityLabel("Usage provider")
            }
            Divider()
            if settings.enabledProviders.isEmpty {
                Text("No providers enabled").font(.headline)
                Text("Enable a provider in Menu Bar settings to see its usage.").foregroundColor(.secondary)
            } else if let usage = presentation.selectedUsage {
                HStack(alignment: .firstTextBaseline) {
                    Text(providerName(usage.provider)).font(.system(size: 20, weight: .semibold))
                    Spacer()
                    if let plan = usage.plan { Text(plan.capitalized).font(.system(size: 11, weight: .medium)).foregroundColor(.secondary) }
                }
                if settings.showAccount {
                    Text(usage.account ?? "Account unavailable").font(.system(size: 11)).foregroundColor(.secondary).lineLimit(1)
                }
                TimelineView(.periodic(from: .now, by: 30)) { context in
                    VStack(alignment: .leading, spacing: 12) {
                        ForEach(usage.windows) { window in
                            quota(window, date: context.date, remaining: settings.displayMode != "used",
                                stale: presentation.error != nil || usage.observedAt.map { context.date.timeIntervalSince1970 - Double($0) >= 600 } == true)
                        }
                        if let observed = usage.observedAt {
                            Text("Updated \(relativeTime(observed, now: context.date))").font(.system(size: 10)).foregroundColor(.secondary)
                        }
                    }
                }
                if let error = presentation.error ?? usage.error {
                    Text(error).font(.system(size: 11)).foregroundColor(.secondary).fixedSize(horizontal: false, vertical: true)
                }
                if settings.showCost, let amounts = usage.accountMetrics, !amounts.isEmpty {
                    Divider()
                    Text("Account billing").fontWeight(.semibold)
                    ForEach(amounts) { amount in detail(amount.label, moneyLabel(amount.value)) }
                }
                if settings.showTokens || settings.showCost {
                    Divider()
                    HStack {
                        Text("Today").fontWeight(.semibold)
                        Spacer()
                        if settings.showCost { Text(moneyLabel(usage.today.costMicrousd)).monospacedDigit() }
                    }
                    if settings.showTokens {
                        detail("Tokens", countLabel(usage.today.tokens))
                    }
                    if settings.showCost {
                        detail("Last 30 days", moneyLabel(usage.month.costMicrousd))
                        if usage.today.costMicrousd.source == "estimated" || usage.month.costMicrousd.source == "estimated" {
                            Text("≈ Estimated from recorded tokens and model pricing").font(.system(size: 10)).foregroundColor(.secondary)
                        }
                    }
                    if settings.showTokens {
                        ForEach(Array(usage.today.models.prefix(4))) { model in
                            VStack(alignment: .leading, spacing: 4) {
                                HStack {
                                    Text(model.model).font(.system(size: 11, weight: .medium)).lineLimit(1)
                                    Spacer()
                                    if settings.showCost { Text(moneyLabel(model.costMicrousd)).font(.system(size: 11)).monospacedDigit() }
                                }
                                Text("In \(countLabel(model.inputTokens)) · Out \(countLabel(model.outputTokens)) · Cache \(countLabel(model.cacheTokens))")
                                    .font(.system(size: 10)).foregroundColor(.secondary).fixedSize(horizontal: false, vertical: true)
                            }
                        }
                    }
                    Text(usage.coverage).font(.system(size: 10)).foregroundColor(.secondary).fixedSize(horizontal: false, vertical: true)
                }
            } else {
                Text(presentation.refreshing ? "Reading provider usage…" : "Usage unavailable").font(.headline)
                Text(presentation.error ?? "Refresh to read your account limits and recorded token usage.").foregroundColor(.secondary)
            }
        }
        .font(.system(size: 12))
        .padding(.horizontal, 16).padding(.vertical, 12)
        .frame(width: 350, alignment: .leading)
    }

    func detail(_ label: String, _ value: String) -> some View {
        HStack { Text(label).foregroundColor(.secondary); Spacer(); Text(value).monospacedDigit() }
    }

    func quota(_ window: QuotaWindow, date: Date, remaining: Bool, stale: Bool) -> some View {
        let expired = window.resetsAt.map { Double($0) <= date.timeIntervalSince1970 } ?? false
        let value = expired || stale ? nil : window.usedPercent.current
        return VStack(alignment: .leading, spacing: 5) {
            HStack {
                Text(window.label).fontWeight(.semibold)
                Spacer()
                Text(value.map { String(format: "%.0f%% %@", remaining ? max(0, 100 - $0) : $0, remaining ? "left" : "used") }
                     ?? (window.usedPercent.status == "stale" || expired || stale ? "Stale" : "Unavailable"))
                    .foregroundColor(.secondary).monospacedDigit()
            }
            GeometryReader { geometry in
                ZStack(alignment: .leading) {
                    Capsule().fill(Color.primary.opacity(0.10))
                    if let value = value {
                        Capsule().fill(Color.accentColor).frame(width: geometry.size.width * min(100, max(0, remaining ? 100 - value : value)) / 100)
                    }
                }
            }.frame(height: 5)
            if let reset = window.resetsAt {
                Text(expired ? "Reset passed · refresh for current limits" : "Resets in \(countdown(reset, now: date))")
                    .font(.system(size: 10)).foregroundColor(.secondary)
            }
        }.accessibilityElement(children: .combine)
    }
}

func countdown(_ timestamp: Int64, now: Date) -> String {
    let minutes = max(1, Int(ceil((Double(timestamp) - now.timeIntervalSince1970) / 60)))
    if minutes >= 1440 { return "\(minutes / 1440)d \((minutes % 1440) / 60)h" }
    if minutes >= 60 { return "\(minutes / 60)h \(minutes % 60)m" }
    return "\(minutes)m"
}

func relativeTime(_ timestamp: Int64, now: Date) -> String {
    let seconds = max(0, Int(now.timeIntervalSince1970) - Int(timestamp))
    if seconds < 60 { return "just now" }
    if seconds < 3600 { return "\(seconds / 60)m ago" }
    if seconds < 86400 { return "\(seconds / 3600)h ago" }
    return "\(seconds / 86400)d ago"
}

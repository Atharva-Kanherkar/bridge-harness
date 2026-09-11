import SwiftUI

// The native card mirrors CodexBar's switcher + overview quota sections. Its
// data comes only from Bridge snapshots; no auth or collection lives here.
struct MenuCard: View {
    @ObservedObject var state: MenuState
    var body: some View {
        let presentation = state.presentation
        let settings = presentation.settings
        VStack(alignment: .leading, spacing: 13) {
            HStack {
                Text(state.showingOverview ? "Usage overview" : "Usage & spend").font(.system(size: 12, weight: .semibold))
                Spacer()
                Text(presentation.refreshing ? "Refreshing…" : "Bridge").font(.system(size: 11)).foregroundColor(.secondary)
            }
            if !settings.enabledProviders.isEmpty {
                Picker("Provider", selection: Binding(get: { state.showingOverview ? "overview" : settings.activeProvider ?? "codex" }, set: { state.select($0) })) {
                    Text("Overview").tag("overview")
                    ForEach(settings.enabledProviders, id: \.self) { id in Text(providerName(id)).tag(id) }
                }.pickerStyle(.segmented).controlSize(.small).font(.system(size: 11)).labelsHidden().accessibilityLabel("Usage provider")
            }
            if settings.enabledProviders.isEmpty {
                Text("No providers enabled").font(.headline)
                Text("Enable a provider in Menu Bar settings to see its usage.").foregroundColor(.secondary)
            } else if state.showingOverview {
                overview(presentation)
            } else if let usage = presentation.selectedUsage {
                Divider()
                providerHeader(usage, compact: false)
                if settings.showAccount {
                    Text(usage.account ?? "Account unavailable").font(.system(size: 11)).foregroundColor(.secondary).lineLimit(1)
                }
                ProviderQuotaSection(usage: usage, presentation: presentation, compact: false)
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
                    if settings.showTokens { detail("Tokens", countLabel(usage.today.tokens)) }
                    if settings.showCost { detail("Last 30 days", moneyLabel(usage.month.costMicrousd)) }
                    if settings.showTokens { detail("30-day tokens", countLabel(usage.month.tokens)) }
                    if settings.showHistory ?? true {
                        DailyUsageView(usage: usage, showCost: settings.showCost, showTokens: settings.showTokens, onResize: state.contentChanged).id(usage.provider)
                    } else if settings.showTokens {
                        Text("Models · last 30 days").fontWeight(.semibold)
                        ModelUsageRows(models: usage.month.models, showCost: settings.showCost)
                    }
                    if settings.showCost && (usage.today.costMicrousd.source == "estimated" || usage.month.costMicrousd.source == "estimated") {
                        Text("≈ Estimated from recorded tokens and model pricing").font(.system(size: 10)).foregroundColor(.secondary)
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

    @ViewBuilder func overview(_ presentation: Presentation) -> some View {
        ForEach(presentation.settings.enabledProviders, id: \.self) { provider in
            Divider()
            if let usage = presentation.usage?.providers.first(where: { $0.provider == provider }) {
                Button(action: { state.select(provider) }) {
                    HStack {
                        providerHeader(usage, compact: true)
                        Image(systemName: "chevron.right").font(.system(size: 10)).foregroundColor(.secondary)
                    }.contentShape(Rectangle())
                }.buttonStyle(.plain).accessibilityLabel("Show \(providerName(provider)) details")
                ProviderQuotaSection(usage: usage, presentation: presentation, compact: true)
            } else {
                Text(providerName(provider)).fontWeight(.semibold)
                Text(presentation.refreshing ? "Reading usage…" : "Usage unavailable").foregroundColor(.secondary)
            }
        }
    }
}

private func providerHeader(_ usage: UsageOverview, compact: Bool) -> some View {
    HStack(alignment: .firstTextBaseline) {
        Text(providerName(usage.provider)).font(.system(size: compact ? 13 : 20, weight: .semibold))
        Spacer()
        if let plan = usage.plan { Text(plan.replacingOccurrences(of: "_", with: " ").capitalized).font(.system(size: 11, weight: .medium)).foregroundColor(.secondary) }
    }
}

func detail(_ label: String, _ value: String) -> some View {
    HStack { Text(label).foregroundColor(.secondary); Spacer(); Text(value).monospacedDigit() }
}

struct ProviderQuotaSection: View {
    let usage: UsageOverview
    let presentation: Presentation
    let compact: Bool
    var body: some View {
        TimelineView(.periodic(from: .now, by: 30)) { context in
            VStack(alignment: .leading, spacing: compact ? 9 : 12) {
                ForEach(usage.windows) { window in
                    QuotaRow(window: window, usage: usage, mode: presentation.settings.quotaDisplayMode ?? "used",
                             date: context.date, failed: presentation.error != nil, showPace: !compact)
                }
                if usage.windows.isEmpty {
                    Text("Account limits unavailable").foregroundColor(.secondary)
                }
                if let error = presentation.error ?? usage.error {
                    Text(error).font(.system(size: 10)).foregroundColor(.secondary)
                        .lineLimit(compact ? 2 : nil).fixedSize(horizontal: false, vertical: true).help(error)
                }
                if let observed = usage.observedAt {
                    Text("Updated \(relativeTime(observed, now: context.date))").font(.system(size: 10)).foregroundColor(.secondary)
                }
                if !compact, let source = usage.quotaSource {
                    Text("Source: \(source)").font(.system(size: 10)).foregroundColor(.secondary)
                }
            }
        }
    }
}

struct QuotaRow: View {
    let window: QuotaWindow
    let usage: UsageOverview
    let mode: String
    let date: Date
    let failed: Bool
    let showPace: Bool
    var body: some View {
        let display = QuotaDisplay(window, usage: usage, mode: mode, now: date, failed: failed)
        VStack(alignment: .leading, spacing: 5) {
            HStack {
                Text(quotaLabel(window, provider: usage.provider)).fontWeight(.semibold)
                Spacer()
                Text(display.valueLabel).foregroundColor(.secondary).monospacedDigit()
            }
            GeometryReader { geometry in
                ZStack(alignment: .leading) {
                    Capsule().fill(Color.primary.opacity(0.10))
                    if let fill = display.fill {
                        Capsule().fill(Color.primary.opacity(0.8)).frame(width: geometry.size.width * fill)
                    }
                }
            }.frame(height: 5).accessibilityHidden(true)
            HStack(alignment: .top) {
                if let reset = window.resetsAt {
                    Text(display.expired ? "Reset passed · refresh for current limits" : "Resets in \(countdown(reset, now: date))")
                }
                Spacer(minLength: 4)
                if let counterpart = display.counterpart { Text(counterpart).monospacedDigit() }
            }.font(.system(size: 10)).foregroundColor(.secondary)
            if showPace, let pace = quotaPace(window, used: display.used, now: date) {
                Text("Pace: \(pace)").font(.system(size: 10)).foregroundColor(.secondary)
                    .help("Estimate relative to evenly distributed use across this quota window; not an additional allowance.")
            }
        }.accessibilityElement(children: .combine)
    }
}

struct ModelUsageRows: View {
    let models: [ModelUsage]
    let showCost: Bool
    var body: some View {
        ForEach(models) { model in
            VStack(alignment: .leading, spacing: 4) {
                HStack {
                    Text(model.model).font(.system(size: 11, weight: .medium)).lineLimit(1).help(model.model)
                    Spacer()
                    if showCost { Text(moneyLabel(model.costMicrousd)).font(.system(size: 11)).monospacedDigit() }
                }
                Text("In \(countLabel(model.inputTokens)) · Out \(countLabel(model.outputTokens)) · Cache \(countLabel(model.cacheTokens))")
                    .font(.system(size: 10)).foregroundColor(.secondary).fixedSize(horizontal: false, vertical: true)
            }
        }
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

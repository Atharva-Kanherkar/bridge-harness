import AppKit

// Adapted to Bridge's bounded token contract from CodexBar's MenuBarLayout
// compositor. An unavailable field remains explicit; no expression evaluation.
struct StatusLayout {
    let lines: [[String]]
    let presentation: Presentation
    let now: Int64
    var usage: UsageOverview? { presentation.statusUsage }
    var custom: Bool { !lines.isEmpty }
    var hasContent: Bool { lines.joined().contains { $0 != "space" && $0 != "dot" } }
    var hasLeadingIcon: Bool { lines.count == 1 && lines.first?.first == "icon" }
    var requiresTemplateImage: Bool { lines.count > 1 || (!hasLeadingIcon && lines.joined().contains("icon")) }

    init(_ presentation: Presentation, now: Int64) {
        self.presentation = presentation
        self.now = now
        lines = Array((presentation.settings.statusLayout ?? []).prefix(2)).map { Array($0.prefix(12)) }
    }

    func quota(_ preference: String, remaining: Bool) -> (String, String) {
        guard let usage = usage, let window = usage.menuWindow(preference, now: now) else { return ("—", "quota unavailable") }
        let display = QuotaDisplay(window, usage: usage, mode: remaining ? "remaining" : "used", now: Date(timeIntervalSince1970: Double(now)), failed: presentation.error != nil)
        guard let used = display.used else { return ("—", "\(quotaLabel(window, provider: usage.provider)) \(display.valueLabel.lowercased())") }
        let value = quotaPercentLabel(remaining ? max(0, 100 - used) : used)
        return (value, "\(quotaLabel(window, provider: usage.provider)), \(value) \(remaining ? "remaining" : "used")")
    }

    func text(_ token: String) -> (String, String) {
        switch token {
        case "icon": return ("\u{fffc}", "Bridge")
        case "provider":
            let label = presentation.settings.statusProvider.map(providerName) ?? "Bridge"
            return (label, label)
        case "used", "remaining": return quota(presentation.settings.quotaWindow, remaining: token == "remaining")
        case "weeklyUsed", "weeklyRemaining":
            let value = quota("weekly", remaining: token == "weeklyRemaining"); return ("7d \(value.0)", value.1)
        case "fiveHourUsed", "fiveHourRemaining":
            let value = quota("fiveHour", remaining: token == "fiveHourRemaining"); return ("5h \(value.0)", value.1)
        case "reset":
            if let window = usage?.menuWindow(presentation.settings.quotaWindow, now: now),
               let usage = usage, QuotaDisplay(window, usage: usage, mode: "used", now: Date(timeIntervalSince1970: Double(now)), failed: presentation.error != nil).used != nil,
               let reset = window.resetsAt, reset > now {
                let value = countdown(reset, now: Date(timeIntervalSince1970: Double(now)))
                return (value, "resets in \(value)")
            }
            return ("—", "reset unavailable")
        case "todayCost":
            let value = moneyLabel(usage?.today.costMicrousd ?? .unavailable)
            return (value == "Unavailable" ? "—" : value, "today's cost \(value)")
        case "dot": return (" · ", "")
        case "space": return (" ", "")
        default: return ("", "")
        }
    }

    var visibleText: String { lines.map { $0.map { text($0).0 }.joined() }.joined(separator: "\n") }
    var accessibilityTitle: String {
        if let provider = presentation.settings.statusProvider, !presentation.settings.isProviderEnabled(provider) {
            return "Bridge usage menu, \(providerName(provider)), disconnected"
        }
        return "Bridge usage menu, " + lines.joined().map { text($0).1 }.filter { !$0.isEmpty }.joined(separator: ", ")
    }

    func attributedTitle(icon: NSImage, omitLeadingIcon: Bool = false, monochrome: Bool = false) -> NSAttributedString {
        let stacked = lines.count > 1
        let font = NSFont.monospacedDigitSystemFont(ofSize: stacked ? 9 : 12, weight: .medium)
        let style = NSMutableParagraphStyle()
        style.alignment = .center
        if stacked { style.minimumLineHeight = 10; style.maximumLineHeight = 10 }
        let text = omitLeadingIcon && hasLeadingIcon ? String(visibleText.dropFirst()) : visibleText
        let result = NSMutableAttributedString(string: text, attributes: [.font: font, .paragraphStyle: style, .foregroundColor: monochrome ? NSColor.black : NSColor.labelColor])
        let length = (result.string as NSString).length
        for index in (0..<length).reversed() where (result.string as NSString).character(at: index) == 0xfffc {
            let attachment = NSTextAttachment()
            let size: CGFloat = stacked ? 9 : 16
            let image = icon.copy() as! NSImage
            image.size = NSSize(width: size, height: size)
            attachment.image = image
            attachment.bounds = NSRect(x: 0, y: stacked ? -1 : -3, width: size, height: size)
            result.replaceCharacters(in: NSRange(location: index, length: 1), with: NSAttributedString(attachment: attachment))
        }
        // NSTextAttachment's replacement string otherwise resets the first
        // paragraph to default typography and makes a two-line label too tall.
        result.addAttributes([.font: font, .paragraphStyle: style, .foregroundColor: monochrome ? NSColor.black : NSColor.labelColor], range: NSRange(location: 0, length: result.length))
        return result
    }

    // Stacked/inline icons and text form one alpha mask. AppKit then applies
    // menu-bar contrast, highlight and inactive-display tint to the whole item.
    func templateImage(icon: NSImage) -> NSImage {
        let title = attributedTitle(icon: icon, monochrome: true)
        let textSize = title.size()
        let size = NSSize(width: ceil(textSize.width) + 2, height: ceil(textSize.height))
        let image = NSImage(size: size, flipped: false) { _ in
            title.draw(at: NSPoint(x: 1, y: 0))
            return true
        }
        image.isTemplate = true
        return image
    }
}

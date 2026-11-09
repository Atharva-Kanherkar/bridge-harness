import AppKit

// Unread chats, from core's attention/unread_summary. Optional on the wire: a
// snapshot from a host that does not send it decodes as nothing unread.
struct AttentionCounts: Decodable, Equatable {
    var total: Int
    var needsYou: Int
}

/// The menu row and the spoken suffix, or nil when nothing is unread.
func attentionLabel(total: Int, needsYou: Int) -> String? {
    guard total > 0 else { return nil }
    let chats = { (count: Int) in count == 1 ? "chat" : "chats" }
    guard needsYou > 0 else { return "\(total) unread \(chats(total))" }
    let waiting = "\(needsYou) \(chats(needsYou)) \(needsYou == 1 ? "needs" : "need") you"
    let rest = total - needsYou
    return rest > 0 ? "\(waiting) · \(rest) unread" : waiting
}

func attentionLabel(_ attention: AttentionCounts?) -> String? {
    attention.flatMap { attentionLabel(total: $0.total, needsYou: $0.needsYou) }
}

/// The status item's tooltip and VoiceOver title, with what is unread last.
func accessibleTitle(_ base: String, attention: AttentionCounts?) -> String {
    attentionLabel(attention).map { "\(base), \($0)" } ?? base
}

/// The status icon with a dot in its top-right corner, cut out of the artwork
/// so it reads at menu bar size. A template icon stays a template, so the dot
/// follows the menu bar's appearance like the rest of the glyph.
func iconWithUnreadDot(_ icon: NSImage) -> NSImage {
    let image = NSImage(size: icon.size, flipped: false) { rect in
        icon.draw(in: rect)
        let diameter = (rect.width * 0.36).rounded()
        let dot = NSRect(x: rect.maxX - diameter, y: rect.maxY - diameter, width: diameter, height: diameter)
        NSGraphicsContext.current?.compositingOperation = .clear
        NSBezierPath(ovalIn: dot.insetBy(dx: -1.5, dy: -1.5)).fill()
        NSGraphicsContext.current?.compositingOperation = .sourceOver
        (icon.isTemplate ? NSColor.black : NSColor.labelColor).setFill()
        NSBezierPath(ovalIn: dot).fill()
        return true
    }
    image.isTemplate = icon.isTemplate
    return image
}

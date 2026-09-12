import AppKit
import SwiftUI

// Adapted from CodexBar at 928166f: provider descriptors, MenuHighlightStyle,
// and the track/fill drawing in UsageProgressBar. Copyright (c) 2026 Peter
// Steinberger. See docs/third-party/CodexBar-LICENSE.txt.
enum ProviderStyle {
    static func accent(_ provider: String) -> Color {
        switch provider {
        case "codex": return Color(red: 73 / 255, green: 163 / 255, blue: 176 / 255)
        case "claude": return Color(red: 204 / 255, green: 124 / 255, blue: 94 / 255)
        case "cursor": return Color(red: 0 / 255, green: 191 / 255, blue: 165 / 255)
        case "opencode": return Color(red: 59 / 255, green: 130 / 255, blue: 246 / 255)
        default: return Color(nsColor: .labelColor)
        }
    }
}

struct ProviderProgressBar: View {
    // Nil means unavailable, never an observed zero. Bridge supplies the
    // used/remaining direction and retains its fractional percentage labels.
    let fill: Double?
    let provider: String

    var body: some View {
        Canvas { context, size in
            let cornerSize = CGSize(width: size.height / 2, height: size.height / 2)
            let rect = CGRect(origin: .zero, size: size)
            context.clip(to: Path(rect))
            let track = Path { $0.addRoundedRect(in: rect, cornerSize: cornerSize) }
            context.fill(track, with: .color(Color(nsColor: .tertiaryLabelColor).opacity(0.22)))
            if let fill = fill, fill.isFinite, fill > 0 {
                let filledRect = CGRect(x: 0, y: 0, width: size.width * min(1, fill), height: size.height)
                let filledPath = Path { $0.addRoundedRect(in: filledRect, cornerSize: cornerSize) }
                context.fill(filledPath, with: .color(ProviderStyle.accent(provider)))
            }
        }
        .frame(height: 6)
        .accessibilityHidden(true)
    }
}

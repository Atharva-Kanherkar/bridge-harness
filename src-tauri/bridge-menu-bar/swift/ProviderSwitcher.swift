import AppKit
import SwiftUI

// CodexBar's text-only switcher, adapted for Bridge's four-provider snapshot.
// Source: StatusItemController+SwitcherViews.swift and ProviderSwitcherButtons.swift
// at 928166f. Copyright (c) 2026 Peter Steinberger.
// See docs/third-party/CodexBar-LICENSE.txt. No provider collection lives here.
struct ProviderSwitcher: NSViewRepresentable {
    let providers: [String]
    let selection: String
    let onSelect: (String) -> Void

    func makeNSView(context: Context) -> ProviderSwitcherView {
        ProviderSwitcherView(providers: providers, selection: selection, onSelect: onSelect)
    }

    func updateNSView(_ view: ProviderSwitcherView, context: Context) {
        view.update(providers: providers, selection: selection, onSelect: onSelect)
    }
}

private final class PaddedProviderButton: NSButton {
    override var intrinsicContentSize: NSSize {
        let size = super.intrinsicContentSize
        return NSSize(width: size.width + 14, height: size.height + 8)
    }
}

final class ProviderSwitcherView: NSView {
    private var ids: [String] = []
    private(set) var buttons: [NSButton] = []
    private var onSelect: (String) -> Void
    private var hoveredIndex: Int?
    private var hoverTrackingArea: NSTrackingArea?

    init(providers: [String], selection: String, onSelect: @escaping (String) -> Void) {
        self.onSelect = onSelect
        super.init(frame: NSRect(x: 0, y: 0, width: 350, height: 30))
        setAccessibilityElement(false)
        setAccessibilityLabel("Usage provider")
        update(providers: providers, selection: selection, onSelect: onSelect)
    }

    required init?(coder: NSCoder) { nil }
    override var intrinsicContentSize: NSSize { NSSize(width: 350, height: 30) }

    func update(providers: [String], selection: String, onSelect: @escaping (String) -> Void) {
        self.onSelect = onSelect
        let next = ["overview"] + providers
        if ids != next {
            buttons.forEach { $0.removeFromSuperview() }
            ids = next
            hoveredIndex = nil
            buttons = ids.enumerated().map { index, id in
                let button = PaddedProviderButton(title: id == "overview" ? "Overview" : providerName(id),
                    target: self, action: #selector(selectProvider(_:)))
                button.tag = index
                button.bezelStyle = .regularSquare
                button.isBordered = false
                button.controlSize = .small
                button.font = NSFont.systemFont(ofSize: NSFont.smallSystemFontSize)
                button.setButtonType(.toggle)
                button.alignment = .center
                button.wantsLayer = true
                button.layer?.cornerRadius = 6
                button.setAccessibilityIdentifier("menu-provider-\(id)")
                addSubview(button)
                return button
            }
        }
        for button in buttons { button.state = ids[button.tag] == selection ? .on : .off }
        needsLayout = true
        updateStyles()
    }

    override func layout() {
        super.layout()
        guard !buttons.isEmpty else { return }
        // Measure both toggle states so switching never changes spacing.
        let desired = buttons.map { button -> CGFloat in
            let state = button.state
            defer { button.state = state }
            button.state = .off
            let off = button.intrinsicContentSize.width
            button.state = .on
            return ceil(max(off, button.intrinsicContentSize.width))
        }.max() ?? 0
        let evenDesired = ceil(desired / 2) * 2
        let count = CGFloat(buttons.count)
        let minimumGap: CGFloat = 1
        let gaps = CGFloat(max(0, buttons.count - 1))
        func allowed(_ padding: CGFloat) -> CGFloat {
            floor(max(0, bounds.width - padding * 2 - minimumGap * gaps) / count / 2) * 2
        }
        // CodexBar preserves the 16pt content grid, then relaxes to 10 or 6
        // only when the longest title needs more space (not smaller text).
        let padding: CGFloat = [CGFloat(16), 10, 6].first { allowed($0) >= evenDesired } ?? 6
        let width = min(evenDesired, allowed(padding))
        let gap = gaps > 0 ? max(minimumGap, (bounds.width - padding * 2 - width * count) / gaps) : 0
        for (index, button) in buttons.enumerated() {
            let x = buttons.count == 1 ? (bounds.width - width) / 2 : padding + CGFloat(index) * (width + gap)
            button.frame = NSRect(x: x, y: 0, width: width, height: 30)
        }
    }

    @objc private func selectProvider(_ sender: NSButton) {
        guard ids.indices.contains(sender.tag) else { return }
        for button in buttons { button.state = button === sender ? .on : .off }
        updateStyles()
        onSelect(ids[sender.tag])
    }

    override func viewDidChangeEffectiveAppearance() { super.viewDidChangeEffectiveAppearance(); updateStyles() }

    override func updateTrackingAreas() {
        super.updateTrackingAreas()
        if let tracking = hoverTrackingArea { removeTrackingArea(tracking) }
        let tracking = NSTrackingArea(rect: .zero,
            options: [.activeAlways, .inVisibleRect, .mouseEnteredAndExited, .mouseMoved], owner: self)
        addTrackingArea(tracking)
        hoverTrackingArea = tracking
    }

    override func mouseMoved(with event: NSEvent) {
        let point = convert(event.locationInWindow, from: nil)
        hoveredIndex = buttons.first { $0.frame.contains(point) }?.tag
        updateStyles()
    }
    override func mouseExited(with event: NSEvent) { hoveredIndex = nil; updateStyles() }

    private func updateStyles() {
        effectiveAppearance.performAsCurrentDrawingAppearance {
            CATransaction.begin()
            CATransaction.setDisableActions(true)
            defer { CATransaction.commit() }
            let light = effectiveAppearance.bestMatch(from: [.aqua, .darkAqua]) == .aqua
            for button in buttons {
                let selected = button.state == .on
                button.contentTintColor = selected ? .white : .secondaryLabelColor
                let hover = light ? NSColor.black.withAlphaComponent(0.095) : NSColor.labelColor.withAlphaComponent(0.06)
                button.layer?.backgroundColor = (selected ? NSColor.controlAccentColor : hoveredIndex == button.tag ? hover : .clear).cgColor
            }
        }
    }
}

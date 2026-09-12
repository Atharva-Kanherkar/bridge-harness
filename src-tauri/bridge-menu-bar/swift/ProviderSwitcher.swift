import AppKit
import SwiftUI

// CodexBar's text-only switcher styling, adapted to a bounded Bridge strip.
// Source: StatusItemController+SwitcherViews.swift at 928166f.
// Copyright (c) 2026 Peter Steinberger. See the bundled MIT notice.
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
    var hoverChanged: ((Bool) -> Void)?
    private var hoverArea: NSTrackingArea?
    override var intrinsicContentSize: NSSize {
        let size = super.intrinsicContentSize
        return NSSize(width: size.width + 14, height: size.height + 8)
    }
    override func updateTrackingAreas() {
        super.updateTrackingAreas()
        if let hoverArea = hoverArea { removeTrackingArea(hoverArea) }
        let area = NSTrackingArea(rect: .zero, options: [.activeAlways, .inVisibleRect, .mouseEnteredAndExited], owner: self)
        addTrackingArea(area)
        hoverArea = area
    }
    override func mouseEntered(with event: NSEvent) { hoverChanged?(true) }
    override func mouseExited(with event: NSEvent) { hoverChanged?(false) }
}

private final class HorizontalProviderScrollView: NSScrollView {
    var scrollRequested: ((CGFloat) -> Void)?
    override func scrollWheel(with event: NSEvent) {
        let delta = abs(event.scrollingDeltaX) >= abs(event.scrollingDeltaY) ? event.scrollingDeltaX : event.scrollingDeltaY
        guard delta != 0 else { return }
        scrollRequested?(delta)
    }
}

final class ProviderSwitcherView: NSView {
    static let fixedWidth: CGFloat = 350
    static let rowHeight: CGFloat = 30
    static let minimumProviderWidth: CGFloat = 70
    private static let overviewWidth: CGFloat = 76
    private static let arrowWidth: CGFloat = 16
    private static let outerInset: CGFloat = 6
    private static let arrowGap: CGFloat = 4
    private static let gap: CGFloat = 2

    private var ids: [String] = []
    private(set) var buttons: [NSButton] = []
    private let scrollView = HorizontalProviderScrollView()
    private let providerDocument = NSView()
    private let previousButton = NSButton()
    private let nextButton = NSButton()
    private var onSelect: (String) -> Void
    private var pendingReveal: String?
    private var hoveredIndex: Int?
    private let scrollAnimation = ProviderScrollAnimation()

    init(providers: [String], selection: String, onSelect: @escaping (String) -> Void) {
        self.onSelect = onSelect
        super.init(frame: NSRect(x: 0, y: 0, width: Self.fixedWidth, height: Self.rowHeight))
        setAccessibilityElement(false)
        setAccessibilityLabel("Usage provider")
        scrollView.drawsBackground = false
        scrollView.hasHorizontalScroller = false
        scrollView.hasVerticalScroller = false
        scrollView.horizontalScrollElasticity = .none
        scrollView.verticalScrollElasticity = .none
        scrollView.documentView = providerDocument
        scrollView.scrollRequested = { [weak self] delta in
            self?.scrollByWheelDelta(delta)
        }
        addSubview(scrollView)
        configureArrow(previousButton, symbol: "chevron.left", action: #selector(scrollBackward))
        configureArrow(nextButton, symbol: "chevron.right", action: #selector(scrollForward))
        update(providers: providers, selection: selection, onSelect: onSelect)
    }

    required init?(coder: NSCoder) { nil }
    override var intrinsicContentSize: NSSize { NSSize(width: Self.fixedWidth, height: Self.rowHeight) }

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        if window == nil { scrollAnimation.cancel() }
    }

    private func configureArrow(_ button: NSButton, symbol: String, action: Selector) {
        button.image = NSImage(systemSymbolName: symbol, accessibilityDescription: nil)
        button.imagePosition = .imageOnly
        button.isBordered = false
        button.bezelStyle = .regularSquare
        button.controlSize = .mini
        button.target = self
        button.action = action
        button.contentTintColor = .secondaryLabelColor
        button.setAccessibilityLabel(symbol == "chevron.left" ? "Previous providers" : "More providers")
        addSubview(button)
    }

    func update(providers: [String], selection: String, onSelect: @escaping (String) -> Void) {
        self.onSelect = onSelect
        let uniqueProviders = providers.reduce(into: [String]()) { if !$0.contains($1) { $0.append($1) } }
        let next = ["overview"] + uniqueProviders
        let listChanged = ids != next
        let previousSelection = buttons.first { $0.state == .on }.flatMap { ids.indices.contains($0.tag) ? ids[$0.tag] : nil }
        if listChanged {
            scrollAnimation.cancel()
            buttons.forEach { $0.removeFromSuperview() }
            ids = next
            buttons = ids.enumerated().map { index, id in
                let button = PaddedProviderButton(title: id == "overview" ? "Overview" : providerName(id), target: self, action: #selector(selectProvider(_:)))
                button.tag = index
                button.bezelStyle = .regularSquare
                button.isBordered = false
                button.controlSize = .small
                button.font = NSFont.systemFont(ofSize: NSFont.smallSystemFontSize)
                button.setButtonType(.toggle)
                button.alignment = .center
                button.cell?.lineBreakMode = .byTruncatingTail
                button.wantsLayer = true
                button.layer?.cornerRadius = 6
                button.setAccessibilityIdentifier("menu-provider-\(id)")
                button.hoverChanged = { [weak self] hovering in
                    self?.hoveredIndex = hovering ? index : nil
                    self?.updateStyles()
                }
                (id == "overview" ? self : providerDocument).addSubview(button)
                return button
            }
        }
        for button in buttons { button.state = ids[button.tag] == selection ? .on : .off }
        if listChanged || previousSelection != selection { pendingReveal = selection }
        needsLayout = true
        layoutSubtreeIfNeeded()
        updateStyles()
    }

    override func layout() {
        super.layout()
        guard let overview = buttons.first else { return }
        let providerButtons = Array(buttons.dropFirst())
        let available = bounds.width - Self.outerInset * 2 - Self.overviewWidth - Self.gap
        // Reserve exactly three comfortable provider slots. A long future name
        // truncates inside its own slot rather than widening every provider.
        let arrowAllowance = (Self.arrowWidth + Self.arrowGap) * 2
        let threeSlotViewport = available - arrowAllowance
        let segmentWidth = max(Self.minimumProviderWidth, floor((threeSlotViewport - Self.gap * 2) / 3))
        let contentWidth = providerButtons.isEmpty ? 0 : CGFloat(providerButtons.count) * segmentWidth + CGFloat(max(0, providerButtons.count - 1)) * Self.gap
        let overflows = contentWidth > available
        let arrowsWidth = overflows ? arrowAllowance : 0
        let viewportWidth = max(0, available - arrowsWidth)
        let leadingInset = Self.outerInset + (overflows ? Self.arrowWidth + Self.arrowGap : 0)
        overview.frame = NSRect(x: leadingInset, y: 0, width: Self.overviewWidth, height: Self.rowHeight)
        scrollView.frame = NSRect(x: overview.frame.maxX + Self.gap, y: 0, width: viewportWidth, height: Self.rowHeight)
        providerDocument.frame = NSRect(x: 0, y: 0, width: max(viewportWidth, contentWidth), height: Self.rowHeight)
        for (index, button) in providerButtons.enumerated() {
            button.frame = NSRect(x: CGFloat(index) * (segmentWidth + Self.gap), y: 0, width: segmentWidth, height: Self.rowHeight)
            button.toolTip = button.intrinsicContentSize.width > segmentWidth ? button.title : nil
        }
        previousButton.isHidden = !overflows
        nextButton.isHidden = !overflows
        previousButton.frame = NSRect(x: Self.outerInset, y: 0, width: Self.arrowWidth, height: Self.rowHeight)
        nextButton.frame = NSRect(x: bounds.width - Self.outerInset - Self.arrowWidth, y: 0, width: Self.arrowWidth, height: Self.rowHeight)
        clampScrollOffset()
        if let selection = pendingReveal {
            reveal(selection)
            pendingReveal = nil
        }
        updateArrowState()
    }

    private var maximumOffset: CGFloat { max(0, providerDocument.frame.width - scrollView.contentView.bounds.width) }
    private func applyOffset(_ proposed: CGFloat) {
        let offset = min(max(0, proposed), maximumOffset)
        scrollView.contentView.scroll(to: NSPoint(x: offset, y: 0))
        scrollView.reflectScrolledClipView(scrollView.contentView)
        updateArrowState()
    }
    private func setOffset(_ proposed: CGFloat, animated: Bool = false) {
        scrollAnimation.cancel()
        let offset = min(max(0, proposed), maximumOffset)
        if animated && window != nil {
            scrollAnimation.start(from: scrollView.contentView.bounds.origin.x, to: offset) { [weak self] value in
                self?.applyOffset(value)
            }
        } else {
            applyOffset(offset)
        }
    }
    private func clampScrollOffset() {
        let origin = scrollView.contentView.bounds.origin.x
        // Snapshot restyling must not interrupt an in-flight arrow gesture.
        if origin < 0 || origin > maximumOffset { setOffset(origin) }
    }
    private func scrollByWheelDelta(_ delta: CGFloat) {
        // AppKit reports positive deltas toward the leading edge.
        setOffset(scrollView.contentView.bounds.origin.x - delta)
    }
    private func reveal(_ id: String) {
        guard id != "overview", let index = ids.firstIndex(of: id), index > 0 else { return }
        let frame = buttons[index].frame
        let visible = scrollView.contentView.bounds
        if frame.minX < visible.minX { setOffset(frame.minX) }
        else if frame.maxX > visible.maxX { setOffset(frame.maxX - visible.width) }
    }
    private func updateArrowState() {
        let offset = scrollView.contentView.bounds.origin.x
        previousButton.isEnabled = offset > 0.5
        nextButton.isEnabled = offset < maximumOffset - 0.5
    }
    @objc private func scrollBackward() { setOffset(scrollView.contentView.bounds.origin.x - scrollView.contentView.bounds.width, animated: true) }
    @objc private func scrollForward() { setOffset(scrollView.contentView.bounds.origin.x + scrollView.contentView.bounds.width, animated: true) }

    @objc private func selectProvider(_ sender: NSButton) {
        guard ids.indices.contains(sender.tag) else { return }
        scrollAnimation.cancel()
        for button in buttons { button.state = button === sender ? .on : .off }
        updateStyles(animated: true)
        reveal(ids[sender.tag])
        onSelect(ids[sender.tag])
    }

    override func viewDidChangeEffectiveAppearance() { super.viewDidChangeEffectiveAppearance(); updateStyles() }
    private func updateStyles(animated: Bool = false) {
        effectiveAppearance.performAsCurrentDrawingAppearance {
            CATransaction.begin()
            CATransaction.setDisableActions(true)
            defer { CATransaction.commit() }
            let light = effectiveAppearance.bestMatch(from: [.aqua, .darkAqua]) == .aqua
            let reduceMotion = NSWorkspace.shared.accessibilityDisplayShouldReduceMotion
            for button in buttons {
                let selected = button.state == .on
                button.contentTintColor = selected ? .white : .secondaryLabelColor
                let hover = light ? NSColor.black.withAlphaComponent(0.095) : NSColor.labelColor.withAlphaComponent(0.06)
                guard let layer = button.layer else { continue }
                let color = (selected ? NSColor.controlAccentColor : hoveredIndex == button.tag ? hover : .clear).cgColor
                let previous = layer.presentation()?.backgroundColor ?? layer.backgroundColor
                let changed = layer.backgroundColor != color
                layer.backgroundColor = color
                let key = "providerSelection"
                if reduceMotion || changed { layer.removeAnimation(forKey: key) }
                if animated && changed && !reduceMotion && window != nil {
                    // CodexBar's explicit presentation-layer fade avoids
                    // animating routine snapshots or SwiftUI menu geometry.
                    let fade = CABasicAnimation(keyPath: "backgroundColor")
                    fade.fromValue = previous
                    fade.toValue = color
                    fade.duration = 0.16
                    fade.timingFunction = CAMediaTimingFunction(name: .easeOut)
                    layer.add(fade, forKey: key)
                }
            }
        }
    }

    // Native control probes keep overflow behavior testable without live providers.
    var testScrollOffset: CGFloat { scrollView.contentView.bounds.origin.x }
    var testScrollAnimating: Bool { scrollAnimation.isRunning }
    var testMaximumOffset: CGFloat { maximumOffset }
    var testArrowState: (previous: Bool, next: Bool) { (previousButton.isEnabled, nextButton.isEnabled) }
    var testViewportWidth: CGFloat { scrollView.contentView.bounds.width }
    var testNavigationFrames: (previous: NSRect, overview: NSRect, viewport: NSRect, next: NSRect) {
        (previousButton.frame, buttons[0].frame, scrollView.frame, nextButton.frame)
    }
    func testScrollForward() { scrollForward() }
    func testScrollBackward() { scrollBackward() }
    func testSetScrollOffset(_ offset: CGFloat) { setOffset(offset) }
    func testScrollByWheelDelta(_ delta: CGFloat) { scrollByWheelDelta(delta) }
}

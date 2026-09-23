import AppKit
import SwiftUI

// Adapted from CodexBar's CostHistoryMenuScrollView at 928166f.
// Copyright (c) 2026 Peter Steinberger. See docs/third-party/CodexBar-LICENSE.txt.
final class MenuCardScrollView: NSScrollView {
    private let cardWidth: CGFloat
    private var viewportSize = NSSize.zero
    private var documentHeight: CGFloat = 0
    private var measuring = false
    private var scheduled = false
    private var scheduledMaximumHeight: CGFloat?
    private var scheduledResetScroll = false
    private var scheduledTransition = false

    init(document: NSView, width: CGFloat, maximumHeight: CGFloat) {
        cardWidth = width
        super.init(frame: .zero)
        borderType = .noBorder
        drawsBackground = false
        hasHorizontalScroller = false
        hasVerticalScroller = false
        document.wantsLayer = true
        documentView = document
        updateSize(maximumHeight: maximumHeight, resetScroll: true)
    }

    required init?(coder: NSCoder) { nil }

    // NSMenu uses intrinsic size when it lays out custom rows during tracking.
    override var intrinsicContentSize: NSSize { viewportSize }
    override var fittingSize: NSSize { viewportSize }

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        guard let screen = window?.screen else { return }
        updateSize(maximumHeight: Self.maximumHeight(on: screen))
    }

    static func maximumHeight(on screen: NSScreen?) -> CGFloat {
        // Leave room for native action rows and the menu's own screen margins.
        max(1, (screen?.visibleFrame.height ?? 800) - 210)
    }

    func updateSize(maximumHeight: CGFloat, resetScroll: Bool = false) {
        guard !measuring else {
            scheduleUpdateSize(maximumHeight: maximumHeight, resetScroll: resetScroll)
            return
        }
        guard let document = documentView else { return }
        measuring = true
        defer { measuring = false }
        let oldMaximumOffset = max(0, documentHeight - contentView.bounds.height)
        let oldTopOffset = document.isFlipped ? contentView.bounds.minY : oldMaximumOffset - contentView.bounds.minY

        document.setFrameSize(NSSize(width: cardWidth, height: document.frame.height))
        document.layoutSubtreeIfNeeded()
        // Re-measure every snapshot: content can grow without a width change.
        documentHeight = max(1, ceil(document.fittingSize.height))
        document.setFrameSize(NSSize(width: cardWidth, height: documentHeight))
        let size = NSSize(width: cardWidth, height: min(documentHeight, max(1, maximumHeight)))
        let sizeChanged = viewportSize != size
        viewportSize = size
        if sizeChanged {
            setFrameSize(size)
            invalidateIntrinsicContentSize()
            tile()
        }

        let maximumOffset = max(0, documentHeight - contentView.bounds.height)
        let topOffset = resetScroll ? 0 : min(maximumOffset, max(0, oldTopOffset))
        contentView.scroll(to: NSPoint(x: 0, y: document.isFlipped ? topOffset : maximumOffset - topOffset))
        reflectScrolledClipView(contentView)
    }

    func scheduleUpdateSize(maximumHeight: CGFloat, resetScroll: Bool = false, animateTransition: Bool = false) {
        scheduledMaximumHeight = maximumHeight
        scheduledResetScroll = scheduledResetScroll || resetScroll
        scheduledTransition = scheduledTransition || animateTransition
        guard !scheduled else { return }
        scheduled = true
        MenuRunLoop.schedule { [weak self] in
            guard let self = self else { return }
            self.scheduled = false
            let maximumHeight = self.scheduledMaximumHeight ?? maximumHeight
            let resetScroll = self.scheduledResetScroll
            let animateTransition = self.scheduledTransition
            self.scheduledMaximumHeight = nil
            self.scheduledResetScroll = false
            self.scheduledTransition = false
            // SwiftUI can leave most of an observed card unpainted while NSMenu
            // owns the event-tracking loop. Rebind its root before measuring so
            // the retained provider data is drawn with the new snapshot.
            if let hosting = self.documentView as? NSHostingView<MenuCard> {
                hosting.rootView = MenuCard(state: hosting.rootView.state)
            }
            self.updateSize(maximumHeight: maximumHeight, resetScroll: resetScroll)
            self.documentView?.layoutSubtreeIfNeeded()
            self.documentView?.displayIfNeeded()
            if animateTransition { self.animateDocumentTransition() }
        }
    }

    private func animateDocumentTransition() {
        guard window != nil, let layer = documentView?.layer else { return }
        let key = "providerTransition"
        layer.removeAnimation(forKey: key)
        guard !NSWorkspace.shared.accessibilityDisplayShouldReduceMotion else { return }
        // CodexBar uses layer opacity for layout-neutral menu fades. Keep the
        // model layer fully visible and never animate NSMenu geometry or retain
        // outgoing SwiftUI cards during fitting-size measurement.
        let fade = CABasicAnimation(keyPath: "opacity")
        fade.fromValue = 0.5
        fade.toValue = 1.0
        fade.duration = 0.2
        fade.timingFunction = CAMediaTimingFunction(name: .easeOut)
        layer.add(fade, forKey: key)
    }
}

// Preserve the full appearance object, including accessibility attributes.
// CodexBar uses the same pinning in StatusItemController+MenuAppearance.swift.
enum MenuAppearance {
    static func pin(_ menu: NSMenu, to appearance: NSAppearance = NSApplication.shared.effectiveAppearance) {
        menu.appearance = appearance
        for item in menu.items {
            if let submenu = item.submenu { pin(submenu, to: appearance) }
        }
    }
}

// The hosted card draws its own controls; suppress AppKit's parallel highlight.
final class MenuCardItem: NSMenuItem {
    override var isHighlighted: Bool { false }
}

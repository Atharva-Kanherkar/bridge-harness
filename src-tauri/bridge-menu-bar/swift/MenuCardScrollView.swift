import AppKit

// Adapted from CodexBar's CostHistoryMenuScrollView at 928166f.
// Copyright (c) 2026 Peter Steinberger. See docs/third-party/CodexBar-LICENSE.txt.
final class MenuCardScrollView: NSScrollView {
    private let cardWidth: CGFloat
    private var viewportSize = NSSize.zero
    private var documentHeight: CGFloat = 0

    init(document: NSView, width: CGFloat, maximumHeight: CGFloat) {
        cardWidth = width
        super.init(frame: .zero)
        borderType = .noBorder
        drawsBackground = false
        hasHorizontalScroller = false
        autohidesScrollers = true
        scrollerStyle = .overlay
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
        guard let document = documentView else { return }
        let oldMaximumOffset = max(0, documentHeight - contentView.bounds.height)
        let oldTopOffset = document.isFlipped ? contentView.bounds.minY : oldMaximumOffset - contentView.bounds.minY

        document.setFrameSize(NSSize(width: cardWidth, height: document.frame.height))
        document.layoutSubtreeIfNeeded()
        // Re-measure every snapshot: content can grow without a width change.
        documentHeight = max(1, ceil(document.fittingSize.height))
        document.setFrameSize(NSSize(width: cardWidth, height: documentHeight))
        let size = NSSize(width: cardWidth, height: min(documentHeight, max(1, maximumHeight)))
        viewportSize = size
        hasVerticalScroller = documentHeight > size.height
        setFrameSize(size)
        invalidateIntrinsicContentSize()
        tile()

        let maximumOffset = max(0, documentHeight - contentView.bounds.height)
        let topOffset = resetScroll ? 0 : min(maximumOffset, max(0, oldTopOffset))
        contentView.scroll(to: NSPoint(x: 0, y: document.isFlipped ? topOffset : maximumOffset - topOffset))
        reflectScrolledClipView(contentView)
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

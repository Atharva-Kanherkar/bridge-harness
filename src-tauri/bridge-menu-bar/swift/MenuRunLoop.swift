import AppKit

// Adapted from CodexBar's ProviderSwitcherTrackingRunLoopScheduler at 928166f.
// Copyright (c) 2026 Peter Steinberger. See docs/third-party/CodexBar-LICENSE.txt.
private final class PendingMenuOperation {
    private var operation: (() -> Void)?

    init(_ operation: @escaping () -> Void) { self.operation = operation }

    func run() {
        precondition(Thread.isMainThread)
        guard let operation = operation else { return }
        self.operation = nil
        operation()
    }
}

enum MenuRunLoop {
    // May be called from Rust's worker thread. Both blocks execute on the main
    // thread, and share a one-shot operation so the context is released once.
    static func schedule(_ operation: @escaping () -> Void) {
        let pending = PendingMenuOperation(operation)
        let runLoop = CFRunLoopGetMain()
        // NSMenu owns a nested tracking loop. The normal Tauri event queue can
        // wait until dismissal; explicitly serve both modes without reentering it.
        for mode in [RunLoop.Mode.eventTracking, .default] {
            CFRunLoopPerformBlock(runLoop, mode.rawValue as CFString) { pending.run() }
        }
        CFRunLoopWakeUp(runLoop)
    }
}

@_cdecl("bridge_menu_bar_schedule")
public func scheduleMenuBarTask(
    _ callback: @escaping @convention(c) (UnsafeMutableRawPointer) -> Void,
    _ context: UnsafeMutableRawPointer
) {
    MenuRunLoop.schedule { callback(context) }
}

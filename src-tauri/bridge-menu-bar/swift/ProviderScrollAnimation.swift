import AppKit

// Animate the clip origin itself so visible tabs and their hit targets stay in
// sync. The menu and viewport frames never change during arrow paging.
final class ProviderScrollAnimation {
    private var timer: Timer?
    var isRunning: Bool { timer != nil }

    func cancel() {
        timer?.invalidate()
        timer = nil
    }

    func start(from: CGFloat, to: CGFloat, update: @escaping (CGFloat) -> Void) {
        cancel()
        guard abs(to - from) > 0.5,
              !NSWorkspace.shared.accessibilityDisplayShouldReduceMotion else {
            update(to)
            return
        }
        let started = ProcessInfo.processInfo.systemUptime
        let timer = Timer(timeInterval: 1.0 / 60.0, repeats: true) { [weak self] _ in
            guard let self = self else { return }
            let elapsed = ProcessInfo.processInfo.systemUptime - started
            let progress = NSWorkspace.shared.accessibilityDisplayShouldReduceMotion ? 1 : min(1, elapsed / 0.18)
            let eased = 1 - pow(1 - progress, 3)
            update(from + (to - from) * CGFloat(eased))
            if progress >= 1 { self.cancel() }
        }
        self.timer = timer
        // NSMenu runs its own tracking mode. No nested loop or geometry update.
        RunLoop.main.add(timer, forMode: .eventTracking)
        RunLoop.main.add(timer, forMode: .default)
    }

    deinit { cancel() }
}

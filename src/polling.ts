type TimerHandle = ReturnType<typeof setTimeout>;

/** Merge refresh notifications while a read is pending, then fetch once more
 * for changes that arrived during that read. No overlapping snapshots or lost
 * final update, even when a worker emits a burst of state changes. */
export function createCoalescedRefresh(task: () => Promise<void>): () => Promise<void> {
  let running: Promise<void> | undefined;
  let requested = false;
  return () => {
    requested = true;
    if (!running) {
      running = Promise.resolve().then(async () => {
        try {
          while (requested) {
            requested = false;
            try {
              await task();
            } catch (error) {
              // A newer notification still needs its read, even if this one
              // failed. Without one, report the failure instead of retrying.
              if (!requested) throw error;
            }
          }
        } finally {
          running = undefined;
        }
      });
    }
    return running;
  };
}

/**
 * Runs a poll immediately, then schedules the next run only after the current
 * one settles. Slow native work can therefore never build an invoke backlog.
 */
export function startSerialPoll(
  task: () => Promise<unknown>,
  intervalMs: number,
  schedule: (callback: () => void, delay: number) => TimerHandle = setTimeout,
  cancel: (handle: TimerHandle) => void = clearTimeout,
): () => void {
  let stopped = false;
  let timer: TimerHandle | undefined;

  const run = async () => {
    try {
      await task();
    } catch {
      // Polls are best-effort snapshots. A transient native/provider failure
      // must not stop future refreshes or surface as an unhandled rejection.
    } finally {
      if (!stopped) timer = schedule(() => void run(), intervalMs);
    }
  };

  void run();
  return () => {
    stopped = true;
    if (timer !== undefined) cancel(timer);
  };
}

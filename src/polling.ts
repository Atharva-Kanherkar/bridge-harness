type TimerHandle = ReturnType<typeof setTimeout>;

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

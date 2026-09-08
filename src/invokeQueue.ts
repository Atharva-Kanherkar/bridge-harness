/** Keep UI bursts in JavaScript until a native connection can serve them.
 * Jobs start in arrival order and run once; an error never retries a mutation. */
export function createInvokeQueue(concurrency: number) {
  let active = 0;
  const waiting: Array<() => void> = [];

  return function enqueue<T>(invoke: () => Promise<T>): Promise<T> {
    return new Promise<T>((resolve, reject) => {
      const start = async () => {
        active += 1;
        try {
          resolve(await invoke());
        } catch (error) {
          reject(error);
        } finally {
          active -= 1;
          waiting.shift()?.();
        }
      };
      if (active < concurrency) void start();
      else waiting.push(() => void start());
    });
  };
}

/** What one terminal's keystrokes become on the wire. */
export type TerminalWrite = (data: string, binary: boolean) => Promise<void>;

type Chunk = { data: string; binary: boolean };

/** Ordered input for one terminal: one write in flight at a time, and every
 *  keystroke typed meanwhile joins the next write.
 *
 *  Each keystroke used to be its own IPC call. The desktop shell runs calls on
 *  a thread pool and the daemon proxy refuses a flood as "busy", so fast typing
 *  or a held key could arrive out of order or not at all. Binary input (X10
 *  mouse reports) never merges with text: the runtime decodes the two
 *  differently. */
export class TerminalInput {
  private queue: Chunk[] = [];
  private sending = false;
  private closed = false;

  constructor(private readonly write: TerminalWrite, private readonly onError: (error: unknown) => void) {}

  send(data: string, binary = false) {
    if (this.closed || !data) return;
    const last = this.queue.at(-1);
    if (last && last.binary === binary) last.data += data;
    else this.queue.push({ data, binary });
    void this.flush();
  }

  close() {
    this.closed = true;
    this.queue = [];
  }

  private async flush() {
    if (this.sending) return;
    this.sending = true;
    try {
      while (!this.closed && this.queue.length) {
        const chunk = this.queue.shift()!;
        try { await this.write(chunk.data, chunk.binary); }
        catch (error) { if (!this.closed) this.onError(error); }
      }
    } finally {
      this.sending = false;
    }
  }
}

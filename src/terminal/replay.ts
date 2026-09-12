import type { TerminalFrame, TerminalSnapshot } from "./types";

/** Orca's checkpoint + sequence boundary, adapted to Bridge's event transport.
 * Subscribe before start(), then reconcile only frames newer than the snapshot.
 * Parsing is serialized with resizes; backlog is bounded outside React state. */
export class TerminalReplay {
  private frames: TerminalFrame[] = [];
  private bytes = 0;
  private sequence = -1;
  private generation = "";
  private retired = new Set<string>();
  private needsSnapshot = true;
  private stopped = false;
  private failed = false;
  private busy: Promise<void> | null = null;

  constructor(private readonly io: {
    snapshot: () => Promise<TerminalSnapshot>;
    restore: (snapshot: TerminalSnapshot) => Promise<void>;
    frame: (frame: TerminalFrame) => Promise<void>;
    error: (error: unknown) => void;
  }) {}

  receive(frame: TerminalFrame) {
    if (this.stopped || this.retired.has(frame.generation)) return;
    if (frame.generation === this.generation && frame.sequence <= this.sequence) return;
    if (this.generation && frame.generation !== this.generation) this.needsSnapshot = true;
    this.frames.push(frame);
    this.bytes += frame.data.length;
    if (this.bytes > 2 * 1024 * 1024) { this.frames = []; this.bytes = 0; this.needsSnapshot = true; }
    if (!this.failed) void this.pump();
  }

  recover() { this.failed = false; this.needsSnapshot = true; return this.pump(); }
  dispose() { this.stopped = true; this.frames = []; }

  private pump(): Promise<void> {
    if (this.stopped) return Promise.resolve();
    if (this.busy) return this.busy;
    this.busy = this.drain().catch(error => {
      this.failed = true;
      if (!this.stopped) this.io.error(error);
    }).finally(() => {
      this.busy = null;
      if (!this.stopped && !this.failed && (this.frames.length || this.needsSnapshot)) void this.pump();
    });
    return this.busy;
  }

  private async drain() {
    while (!this.stopped) {
      if (this.needsSnapshot) {
        this.needsSnapshot = false;
        const snapshot = await this.io.snapshot();
        if (this.stopped) return;
        if (this.generation && this.generation !== snapshot.record.generation) this.retired.add(this.generation);
        this.generation = snapshot.record.generation;
        await this.io.restore(snapshot);
        this.sequence = snapshot.sequence;
        if (this.stopped) return;
      }
      this.frames = this.frames.filter(f => f.generation === this.generation && f.sequence > this.sequence);
      this.bytes = this.frames.reduce((n, f) => n + f.data.length, 0);
      if (!this.frames.length) return;
      this.frames.sort((a, b) => a.sequence - b.sequence);
      const frame = this.frames.shift()!;
      this.bytes -= frame.data.length;
      if (frame.sequence !== this.sequence + 1) {
        this.frames.unshift(frame);
        this.needsSnapshot = true;
        continue;
      }
      await this.io.frame(frame);
      this.sequence = frame.sequence;
    }
  }
}

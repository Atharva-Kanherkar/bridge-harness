import type { SessionForestSnapshot } from "./types";

/** A cache is a convenience, not history. Evicted chats reload from SQLite.
 * Count AND serialized-size limits include tool data, images and metadata,
 * which a text-only budget misses. UTF-16 bytes are an estimate, not RSS. */
export class ForestCache {
  private entries = new Map<string, { value: SessionForestSnapshot; bytes: number }>();
  private bytes = 0;
  constructor(private maxEntries = 4, private maxBytes = 16 * 1024 * 1024) {}
  get(id: string) {
    const entry = this.entries.get(id);
    if (!entry) return undefined;
    this.entries.delete(id);
    this.entries.set(id, entry);
    return entry.value;
  }
  set(id: string, value: SessionForestSnapshot) {
    const old = this.entries.get(id);
    if (old) { this.bytes -= old.bytes; this.entries.delete(id); }
    const bytes = JSON.stringify(value).length * 2;
    // Large selected snapshots can be displayed without retaining another
    // reference after the user leaves that chat.
    if (bytes > this.maxBytes) return;
    this.entries.set(id, { value, bytes });
    this.bytes += bytes;
    while (this.entries.size > this.maxEntries || this.bytes > this.maxBytes) {
      const oldest = this.entries.keys().next().value!;
      this.bytes -= this.entries.get(oldest)!.bytes;
      this.entries.delete(oldest);
    }
  }
  get size() { return this.entries.size; }
  get retainedBytes() { return this.bytes; }
}

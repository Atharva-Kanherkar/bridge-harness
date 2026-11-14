import { describe, expect, it, vi } from "vitest";
import { ForestCache, estimateForestBytes, isPartialForest, pageIsComplete } from "./forestCache";
import type { SessionForestSnapshot } from "./types";
const snapshot = (text: string) => ({ entries: [{ data: { output: text } }] } as unknown as SessionForestSnapshot);
it("evicts least recently visited chats and reloads from durable history", () => {
  const cache = new ForestCache(2);
  const a = snapshot("a"); cache.set("a", a); cache.set("b", snapshot("b"));
  expect(cache.get("a")).toBe(a);
  cache.set("c", snapshot("c"));
  expect(cache.get("b")).toBeUndefined(); expect(cache.size).toBe(2);
});
it("bounds nested tool data and does not retain an oversized selected history", () => {
  const cache = new ForestCache(10, 1000);
  for (let i = 0; i < 100; i++) cache.set(String(i), snapshot("x".repeat(150)));
  expect(cache.retainedBytes).toBeLessThanOrEqual(1000);
  cache.set("99", snapshot("x".repeat(1000)));
  expect(cache.get("99")).toBeUndefined();
  expect(cache.retainedBytes).toBeLessThanOrEqual(1000);
});

describe("long chats", () => {
  const entry = (sequence: number, text: string) => ({ id: `e${sequence}`, sequence, payload: { text } });
  const forest = (count: number, text: string) => ({
    entries: Array.from({ length: count }, (_, index) => entry(index + 1, text)),
    entryWindow: { returned: count, total: count, trimmedPayloads: 0 },
  } as unknown as SessionForestSnapshot);

  it("keeps the newest page of a chat too large to keep whole, marked partial", () => {
    const cache = new ForestCache(4, 40_000, 10);
    const long = forest(200, "x".repeat(200));
    cache.set("long", long, "digest-1");
    const cached = cache.peek("long")!;
    expect(cached.partial).toBe(true);
    expect(isPartialForest(cached.value)).toBe(true);
    // The newest entries, oldest first, exactly as the snapshot had them.
    expect(cached.value.entries.map(item => item.sequence)).toEqual(Array.from({ length: 10 }, (_, index) => 191 + index));
    // A digest describes the whole snapshot, never a page cut from it.
    expect(cached.digest).toBeUndefined();
    expect(cache.retainedBytes).toBeLessThanOrEqual(40_000);
  });

  it("remembers the digest of a whole snapshot", () => {
    const cache = new ForestCache();
    cache.set("short", forest(3, "hello"), "digest-2");
    expect(cache.peek("short")).toMatchObject({ partial: false, digest: "digest-2" });
  });

  it("estimates size without stringifying the snapshot", () => {
    const stringify = vi.spyOn(JSON, "stringify");
    try {
      const cache = new ForestCache();
      cache.set("a", forest(500, "y".repeat(100)));
      expect(stringify).not.toHaveBeenCalled();
      expect(cache.retainedBytes).toBeGreaterThan(500 * 100 * 2);
    } finally {
      stringify.mockRestore();
    }
  });

  it("estimates strings, numbers and keys from the shape alone", () => {
    expect(estimateForestBytes({ ab: "xyz", n: 1, list: [true] })).toBe(
      (2 * 2 + 4) + (3 * 2 + 2) + (1 * 2 + 4) + 8 + (4 * 2 + 4) + 2 + 8,
    );
  });

  it("calls a page complete when it holds the whole branch or a daemon ignored the limit", () => {
    const page = (returned: number, total: number) => ({ entryWindow: { returned, total, trimmedPayloads: 0 } } as SessionForestSnapshot);
    expect(pageIsComplete(page(40, 40))).toBe(true);
    expect(pageIsComplete(page(160, 900))).toBe(false);
    expect(pageIsComplete(page(1500, 9000))).toBe(true);
    expect(pageIsComplete({} as SessionForestSnapshot)).toBe(true);
  });
});

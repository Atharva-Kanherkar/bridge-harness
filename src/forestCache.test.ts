import { expect, it } from "vitest";
import { ForestCache } from "./forestCache";
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

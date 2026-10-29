import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import * as conversation from "../conversation";
import type { SessionEntry } from "../types";
import { projectSessionConversationCached, resetProjectionCache } from "./projectionCache";

function branch(size: number, sessionId = "s"): SessionEntry[] {
  return Array.from({ length: size }, (_, i) => ({
    id: `e${i}`, sessionId, parentEntryId: i ? `e${i - 1}` : null, sequence: i + 1,
    semanticSchemaVersion: 2, kind: i % 2 ? "assistant.message" : "user.message",
    payload: { itemId: `m${i}`, text: `message ${i}`, role: i % 2 ? "assistant" : "user", status: "completed" },
    providerEventId: null, contextVisibility: "eligible", tokenEstimate: null, createdAt: "now",
  }));
}

describe("projectSessionConversationCached", () => {
  beforeEach(() => { resetProjectionCache(); });
  afterEach(() => { vi.restoreAllMocks(); });

  it("caches by entries identity and leaf id", () => {
    const entries = branch(20);
    const spy = vi.spyOn(conversation, "projectSessionConversation");
    const first = projectSessionConversationCached(entries, "e19");
    const second = projectSessionConversationCached(entries, "e19");
    expect(second).toBe(first);
    expect(spy).toHaveBeenCalledTimes(1);
    // A null leaf is its own key, not a miss on a named one.
    const whole = projectSessionConversationCached(entries, null);
    expect(projectSessionConversationCached(entries, null)).toBe(whole);
    expect(spy).toHaveBeenCalledTimes(2);
  });

  it("re-projects when the leaf changes", () => {
    const entries = branch(20);
    const spy = vi.spyOn(conversation, "projectSessionConversation");
    const head = projectSessionConversationCached(entries, "e19");
    const earlier = projectSessionConversationCached(entries, "e9");
    expect(earlier).not.toBe(head);
    expect(earlier).toHaveLength(10);
    expect(spy).toHaveBeenCalledTimes(2);
    expect(projectSessionConversationCached(entries, "e19")).toBe(head);
    expect(spy).toHaveBeenCalledTimes(2);
  });

  it("re-projects when the entries array identity changes even with equal contents", () => {
    const first = branch(12);
    const second = branch(12);
    const spy = vi.spyOn(conversation, "projectSessionConversation");
    const projected = projectSessionConversationCached(first, "e11");
    const again = projectSessionConversationCached(second, "e11");
    expect(again).not.toBe(projected);
    expect(again).toEqual(projected);
    expect(spy).toHaveBeenCalledTimes(2);
  });

  it("matches the uncached projection", () => {
    const entries = branch(200);
    const expected = conversation.projectSessionConversation(entries, "e199");
    expect(projectSessionConversationCached(entries, "e199")).toEqual(expected);
  });

  it("evicts the oldest entries arrays", () => {
    const arrays = Array.from({ length: 10 }, (_, index) => branch(4, `s${index}`));
    const spy = vi.spyOn(conversation, "projectSessionConversation");
    arrays.forEach(entries => projectSessionConversationCached(entries, "e3"));
    expect(spy).toHaveBeenCalledTimes(10);
    // The newest array is still warm.
    projectSessionConversationCached(arrays[9], "e3");
    expect(spy).toHaveBeenCalledTimes(10);
    // The cap is eight, so the first two were evicted; re-adding one evicts the
    // array that is oldest at that moment.
    projectSessionConversationCached(arrays[1], "e3");
    expect(spy).toHaveBeenCalledTimes(11);
    projectSessionConversationCached(arrays[2], "e3");
    expect(spy).toHaveBeenCalledTimes(12);
    projectSessionConversationCached(arrays[9], "e3");
    expect(spy).toHaveBeenCalledTimes(12);
  });

  it("bounds leaf variants per array in LRU order", () => {
    const entries = branch(8);
    const spy = vi.spyOn(conversation, "projectSessionConversation");
    // The cap is four: five leaves leave `e7` behind.
    for (const leaf of ["e7", "e6", "e5", "e4", "e3"]) projectSessionConversationCached(entries, leaf);
    expect(spy).toHaveBeenCalledTimes(5);
    // Touching surviving variants moves them to the front.
    projectSessionConversationCached(entries, "e6");
    projectSessionConversationCached(entries, "e3");
    expect(spy).toHaveBeenCalledTimes(5);
    // `e5` is now the least recently used, so `e7`'s miss evicts it.
    projectSessionConversationCached(entries, "e7");
    expect(spy).toHaveBeenCalledTimes(6);
    for (const leaf of ["e7", "e3", "e6", "e4"]) projectSessionConversationCached(entries, leaf);
    expect(spy).toHaveBeenCalledTimes(6);
    projectSessionConversationCached(entries, "e5");
    expect(spy).toHaveBeenCalledTimes(7);
  });
});

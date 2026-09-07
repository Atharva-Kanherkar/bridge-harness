import { describe, expect, it, vi } from "vitest";
import { mergeConversationProjections, projectSessionConversation, reduceConversation, type ConversationItem } from "../conversation";
import type { AgentEvent, SessionEntry } from "../types";
import { asWireKind } from "./wire";

const event = (i: number): AgentEvent => ({ id: i + 1, sequence: i + 1, sessionId: "s", protocolVersion: 1,
  kind: asWireKind("message.completed"), itemId: `m${i}`, role: i % 2 ? "assistant" : "user",
  text: `message ${i}`, status: "completed", title: null, data: {}, providerMeta: {}, createdAt: "now" });

describe("long completed histories with a bounded live tail", () => {
  it.each([100, 1000, 5000])("projects and merges %i entries with linear fallback work", size => {
    const entries: SessionEntry[] = Array.from({ length: size }, (_, i) => ({
      id: `e${i}`, sessionId: "s", parentEntryId: i ? `e${i - 1}` : null, sequence: i + 1,
      semanticSchemaVersion: 2, kind: i % 2 ? "assistant.message" : "user.message",
      payload: { itemId: `m${i}`, text: `message ${i}`, role: i % 2 ? "assistant" : "user", status: "completed" },
      providerEventId: null, contextVisibility: "eligible", tokenEstimate: null, createdAt: "now",
    }));
    // Count the sizes of maps traversed, independently of CPU/JIT timings.
    // Re-copying the fold for each completed assistant makes this quadratic.
    let traversed = 0;
    const values = Map.prototype.values;
    const spy = vi.spyOn(Map.prototype, "values").mockImplementation(function(this: Map<unknown, unknown>) {
      traversed += this.size;
      return values.call(this);
    });
    let durable: ConversationItem[];
    try { durable = projectSessionConversation(entries, `e${size - 1}`); }
    finally { spy.mockRestore(); }
    expect(durable).toHaveLength(size);
    expect(traversed).toBeLessThan(size * 4);
    const live = reduceConversation(Array.from({ length: Math.min(size, 1000) }, (_, i) => event(size - Math.min(size, 1000) + i)));
    let textReads = 0;
    const counted = (item: ConversationItem): ConversationItem => ({ ...item, get text() { textReads++; return item.text; } });
    const merged = mergeConversationProjections(durable.map(counted), live.map(counted));
    expect(merged).toHaveLength(size);
    expect(textReads).toBeLessThan((size + live.length) * 5);
    expect(merged.map(item => item.text)).toEqual(durable.map(item => item.text));
  });

  it("uses the first text-shadow anchor without hiding named completions or user messages", () => {
    const item = (key: string, sequence: number, status: string, itemId?: string, role = "assistant"): ConversationItem => ({
      key, type: "message", eventId: sequence, sequence, text: "same", data: {}, status, itemId, role, turn: 1,
    });
    const durable = [item("d", 20, "completed", "d")];
    const live = [item("a", 9, "streaming"), item("b", 2, "streaming"), item("c", 10, "completed", "c"), item("u", 11, "completed", undefined, "user")];
    const merged = mergeConversationProjections(durable, live);
    expect(merged.map(row => row.key)).toEqual(["d", "c", "u"]);
    expect(merged[0].sequence).toBe(9);
    expect(durable[0].sequence).toBe(20);
  });
});

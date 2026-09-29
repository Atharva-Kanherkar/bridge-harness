import { describe, expect, it, vi } from "vitest";
import { createLiveReplay, LAG_REPLAY_SESSIONS, REPLAY_PAGE_SIZE } from "./liveReplay";
import type { AgentEvent } from "./types";
import { asWireKind } from "./transcript/wire";

function frame(sequence: number, sessionId = "s", kind = "turn.started", text: string | null = null): AgentEvent {
  return {
    id: sequence, sessionId, sequence, protocolVersion: 1, kind: asWireKind(kind),
    itemId: null, role: null, status: null, title: null, text, data: {}, providerMeta: {}, createdAt: "now",
  };
}

/** A daemon holding `stored` frames per session, answering replay like `replay_session_events`. */
function harness(stored: Record<string, AgentEvent[]> = {}) {
  const delivered: AgentEvent[] = [];
  const replay = vi.fn(async (sessionId: string, after: number, limit: number) =>
    (stored[sessionId] ?? []).filter(event => event.sequence > after).slice(0, limit));
  const live = createLiveReplay({ replay, deliver: events => delivered.push(...events) });
  return { live, replay, delivered };
}

const range = (from: number, to: number, sessionId = "s") =>
  Array.from({ length: to - from + 1 }, (_, index) => frame(from + index, sessionId));

describe("createLiveReplay", () => {
  it("fills the incident's hole: a stale forest, one user frame, then a jump", async () => {
    const reply = frame(14268, "s", "assistant.message", "There are 7 secrets.");
    const { live, replay, delivered } = harness({ s: [frame(14264), frame(14265), frame(14266), frame(14267), reply, ...range(14269, 14271)] });
    live.seed("s", 14263);
    live.observe(frame(14264));
    live.observe(frame(14271));
    await live.idle();
    expect(replay).toHaveBeenCalledWith("s", 14264, REPLAY_PAGE_SIZE);
    expect(delivered).toContainEqual(reply);
  });

  it("never replays contiguous frames", async () => {
    const { live, replay } = harness();
    live.seed("s", 10);
    for (const event of range(11, 20)) live.observe(event);
    await live.idle();
    expect(replay).not.toHaveBeenCalled();
  });

  it("ignores duplicates and transient frames", async () => {
    const { live, replay } = harness();
    live.seed("s", 10);
    live.observe(frame(9));
    live.observe(frame(10));
    live.observe({ ...frame(0), id: 0 });
    await live.idle();
    expect(replay).not.toHaveBeenCalled();
  });

  it("adopts the first frame of an unknown session instead of calling it a gap", async () => {
    const { live, replay } = harness();
    live.observe(frame(500));
    live.observe(frame(501));
    await live.idle();
    expect(replay).not.toHaveBeenCalled();
  });

  it("never lowers a cursor when seeded with an older snapshot", async () => {
    const { live, replay } = harness({ s: range(1, 30) });
    live.seed("s", 20);
    live.seed("s", 5);
    live.observe(frame(22));
    await live.idle();
    expect(replay).toHaveBeenCalledWith("s", 20, REPLAY_PAGE_SIZE);
  });

  it("pages until a short page so a wide hole is filled", async () => {
    const stored = range(1, REPLAY_PAGE_SIZE * 2 + 10);
    const { live, replay, delivered } = harness({ s: stored });
    live.seed("s", 1);
    live.observe(frame(stored.length));
    await live.idle();
    expect(replay).toHaveBeenCalledTimes(3);
    expect(delivered.map(event => event.sequence)).toEqual(stored.slice(1).map(event => event.sequence));
  });

  it("runs one more pass, not a concurrent replay, for a gap seen mid-replay", async () => {
    let release!: () => void;
    const gate = new Promise<void>(resolve => { release = resolve; });
    const delivered: AgentEvent[] = [];
    let stored = range(1, 5);
    let concurrent = 0;
    let peak = 0;
    const replay = vi.fn(async (_: string, after: number) => {
      concurrent += 1; peak = Math.max(peak, concurrent);
      if (replay.mock.calls.length === 1) await gate;
      concurrent -= 1;
      return stored.filter(event => event.sequence > after);
    });
    const live = createLiveReplay({ replay, deliver: events => delivered.push(...events) });
    live.seed("s", 1);
    live.observe(frame(4));
    stored = range(1, 9);
    live.observe(frame(9));
    release();
    await live.idle();
    expect(peak).toBe(1);
    expect(replay).toHaveBeenCalledTimes(2);
    expect(Math.max(...delivered.map(event => event.sequence))).toBe(9);
  });

  it("replays the most recently active sessions on a lag marker", async () => {
    const stored = Object.fromEntries(Array.from({ length: LAG_REPLAY_SESSIONS + 2 }, (_, index) => [`s${index}`, range(1, 3, `s${index}`)]));
    const { live, replay } = harness(stored);
    for (const sessionId of Object.keys(stored)) live.observe(frame(1, sessionId));
    live.lagged();
    await live.idle();
    const replayed = new Set(replay.mock.calls.map(([sessionId]) => sessionId));
    expect(replayed.size).toBe(LAG_REPLAY_SESSIONS);
    expect(replayed.has("s0")).toBe(false);
    expect(replayed.has(`s${LAG_REPLAY_SESSIONS + 1}`)).toBe(true);
  });

  it("swallows a failed replay and retries on the next gap", async () => {
    const delivered: AgentEvent[] = [];
    const replay = vi.fn()
      .mockRejectedValueOnce(new Error("daemon busy"))
      .mockResolvedValue(range(2, 6));
    const live = createLiveReplay({ replay, deliver: events => delivered.push(...events) });
    live.seed("s", 1);
    live.observe(frame(4));
    await live.idle();
    expect(delivered).toHaveLength(0);
    live.observe(frame(6));
    await live.idle();
    expect(delivered.map(event => event.sequence)).toEqual([2, 3, 4, 5, 6]);
  });
});

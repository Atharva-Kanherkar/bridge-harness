import { afterEach, describe, expect, it, vi } from "vitest";
import { playAttentionSound, resetAttentionSound, soundFor, SOUND_PATTERNS } from "./attentionSound";

function stubContext() {
  const frequencies: number[] = [];
  const starts: number[] = [];
  const param = () => ({ value: 0, setValueAtTime: vi.fn(), linearRampToValueAtTime: vi.fn() });
  const context = {
    currentTime: 1,
    destination: {},
    resume: vi.fn(async () => undefined),
    createOscillator: () => {
      const frequency = param();
      return {
        type: "",
        frequency,
        connect: (node: unknown) => node,
        start: (at: number) => { frequencies.push(frequency.value); starts.push(at); },
        stop: vi.fn(),
      };
    },
    createGain: () => ({ gain: param(), connect: (node: unknown) => node }),
  };
  return { context: context as unknown as AudioContext, frequencies, starts };
}

describe("playAttentionSound", () => {
  afterEach(() => resetAttentionSound());

  it("rises for a request and plays one tone for a finished turn", () => {
    const stub = stubContext();
    playAttentionSound("needsYou", undefined, () => stub.context);
    expect(stub.frequencies).toEqual([...SOUND_PATTERNS.request]);
    expect(stub.starts[1]).toBeGreaterThan(stub.starts[0]);
    resetAttentionSound();
    const second = stubContext();
    playAttentionSound("finished", undefined, () => second.context);
    expect(second.frequencies).toEqual([...SOUND_PATTERNS.turn]);
  });

  it("reuses one context", () => {
    const stub = stubContext();
    const create = vi.fn(() => stub.context);
    playAttentionSound("failed", undefined, create);
    playAttentionSound("escalation", undefined, create);
    expect(create).toHaveBeenCalledTimes(1);
  });

  it("swallows a context that cannot play", () => {
    expect(() => playAttentionSound("needsYou", undefined, () => { throw new Error("no audio device"); })).not.toThrow();
  });

  it("rises for an actionable post and plays one tone for an informational one", () => {
    expect(soundFor("posted", "actionable")).toEqual(SOUND_PATTERNS.request);
    expect(soundFor("posted", "informational")).toEqual(SOUND_PATTERNS.turn);
  });
});


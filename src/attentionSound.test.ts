import { afterEach, describe, expect, it, vi } from "vitest";
import { playAttentionSound, resetAttentionSound, SOUND_PATTERNS } from "./attentionSound";

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
    playAttentionSound("needsYou", () => stub.context);
    expect(stub.frequencies).toEqual([...SOUND_PATTERNS.request]);
    expect(stub.starts[1]).toBeGreaterThan(stub.starts[0]);
    resetAttentionSound();
    const second = stubContext();
    playAttentionSound("finished", () => second.context);
    expect(second.frequencies).toEqual([...SOUND_PATTERNS.turn]);
  });

  it("reuses one context", () => {
    const stub = stubContext();
    const create = vi.fn(() => stub.context);
    playAttentionSound("failed", create);
    playAttentionSound("escalation", create);
    expect(create).toHaveBeenCalledTimes(1);
  });

  it("swallows a context that cannot play", () => {
    expect(() => playAttentionSound("needsYou", () => { throw new Error("no audio device"); })).not.toThrow();
  });
});

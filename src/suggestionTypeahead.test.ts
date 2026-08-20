// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { scheduleSuggestion } from "./suggestionTypeahead";
import type { SuggestCompletionResult } from "./protocol/generated/protocol";

const result = (suggestion: string): SuggestCompletionResult => ({ suggestion, usedFallback: false, fallbackReason: null });

beforeEach(() => {
  vi.useFakeTimers();
});

afterEach(() => {
  vi.useRealTimers();
});

describe("scheduleSuggestion", () => {
  it("does not request until the debounce window elapses", () => {
    const request = vi.fn().mockResolvedValue(result(" world"));
    scheduleSuggestion({ text: "hello", enabled: true, request, onResult: () => {}, generation: { current: 0 } });

    vi.advanceTimersByTime(399);
    expect(request).not.toHaveBeenCalled();

    vi.advanceTimersByTime(1);
    expect(request).toHaveBeenCalledWith("hello");
  });

  it("never fires when the toggle is off", () => {
    const request = vi.fn();
    scheduleSuggestion({ text: "hello", enabled: false, request, onResult: () => {}, generation: { current: 0 } });
    vi.advanceTimersByTime(10_000);
    expect(request).not.toHaveBeenCalled();
  });

  it("never fires for an empty or whitespace-only draft", () => {
    const request = vi.fn();
    scheduleSuggestion({ text: "   ", enabled: true, request, onResult: () => {}, generation: { current: 0 } });
    vi.advanceTimersByTime(10_000);
    expect(request).not.toHaveBeenCalled();
  });

  it("clears any pending ghost text immediately, before the debounce fires", () => {
    const onResult = vi.fn();
    scheduleSuggestion({ text: "hello", enabled: true, request: vi.fn(), onResult, generation: { current: 0 } });
    expect(onResult).toHaveBeenCalledWith(undefined);
  });

  it("cancels the pending request when the draft changes before the debounce fires", () => {
    const request = vi.fn().mockResolvedValue(result(" world"));
    const generation = { current: 0 };
    const cancel = scheduleSuggestion({ text: "hel", enabled: true, request, onResult: () => {}, generation });

    vi.advanceTimersByTime(200);
    cancel(); // the cleanup a keystroke's re-run of the effect would call

    vi.advanceTimersByTime(400);
    expect(request).not.toHaveBeenCalled();
  });

  it("drops a response for a superseded draft instead of racing it onto the screen", async () => {
    const onResult = vi.fn();
    const generation = { current: 0 };
    // The stale request: never resolves until after the newer one already has.
    let resolveStale: (value: SuggestCompletionResult) => void = () => {};
    const staleRequest = vi.fn(() => new Promise<SuggestCompletionResult>(resolve => { resolveStale = resolve; }));
    scheduleSuggestion({ text: "hel", enabled: true, request: staleRequest, onResult, generation });
    vi.advanceTimersByTime(400);
    expect(staleRequest).toHaveBeenCalledTimes(1);

    // A newer keystroke bumps the generation before the stale request answers.
    const freshRequest = vi.fn().mockResolvedValue(result(" lo, world"));
    scheduleSuggestion({ text: "hello", enabled: true, request: freshRequest, onResult, generation });
    vi.advanceTimersByTime(400);
    await Promise.resolve();
    await Promise.resolve();
    expect(onResult).toHaveBeenCalledWith(result(" lo, world"));

    onResult.mockClear();
    resolveStale(result(" p"));
    await Promise.resolve();
    await Promise.resolve();
    expect(onResult).not.toHaveBeenCalled();
  });

  it("treats an empty suggestion from the model as nothing to show", async () => {
    const onResult = vi.fn();
    const request = vi.fn().mockResolvedValue(result(""));
    scheduleSuggestion({ text: "hello?", enabled: true, request, onResult, generation: { current: 0 } });
    vi.advanceTimersByTime(400);
    await Promise.resolve();
    await Promise.resolve();

    // Called once eagerly (clearing any prior ghost text) and once more with
    // the empty result normalised to `undefined` — never with an empty string.
    expect(onResult).toHaveBeenCalledWith(undefined);
    expect(onResult).not.toHaveBeenCalledWith(result(""));
  });

  it("clears the suggestion when the request fails, for a draft still current", async () => {
    const onResult = vi.fn();
    const request = vi.fn().mockRejectedValue(new Error("the provider process ended"));
    scheduleSuggestion({ text: "hello", enabled: true, request, onResult, generation: { current: 0 } });
    vi.advanceTimersByTime(400);
    await Promise.resolve();
    await Promise.resolve();

    expect(onResult).toHaveBeenLastCalledWith(undefined);
  });

  it("drops an in-flight response when the schedule is cancelled, including turning the toggle off", async () => {
    const onResult = vi.fn();
    const generation = { current: 0 };
    let resolveInFlight: (value: SuggestCompletionResult) => void = () => {};
    const request = vi.fn(() => new Promise<SuggestCompletionResult>(resolve => { resolveInFlight = resolve; }));
    const cancel = scheduleSuggestion({ text: "hello", enabled: true, request, onResult, generation });
    vi.advanceTimersByTime(400);
    expect(request).toHaveBeenCalledTimes(1);

    cancel();
    scheduleSuggestion({ text: "hello", enabled: false, request, onResult, generation });

    onResult.mockClear();
    resolveInFlight(result(" world"));
    await Promise.resolve();
    await Promise.resolve();
    expect(onResult).not.toHaveBeenCalled();
  });
});

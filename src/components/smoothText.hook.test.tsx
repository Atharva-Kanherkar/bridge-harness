// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { REVEAL_MS, useSmoothText } from "./smoothText";

function Probe({ text, streaming }: { text: string; streaming: boolean }) {
  return <p>{useSmoothText(text, streaming)}</p>;
}

/** One act per frame: each frame's state update schedules the next frame from an effect. */
async function frames(ms: number) {
  for (let elapsed = 0; elapsed < ms; elapsed += 16) await act(async () => { vi.advanceTimersByTime(16); });
}

describe("useSmoothText", () => {
  beforeEach(() => {
    (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
    vi.useFakeTimers({ toFake: ["requestAnimationFrame", "cancelAnimationFrame", "performance"] });
  });
  afterEach(() => vi.useRealTimers());

  it("shows text present on mount at once, then drains each extension within the window", async () => {
    const host = document.createElement("div");
    const root = createRoot(host);
    await act(async () => root.render(<Probe text="Hello" streaming />));
    expect(host.textContent).toBe("Hello");
    await act(async () => root.render(<Probe text="Hello, streaming world" streaming />));
    expect(host.textContent).toBe("Hello");
    await frames(REVEAL_MS / 2);
    const midway = host.textContent ?? "";
    expect(midway.length).toBeGreaterThan(5);
    expect(midway.length).toBeLessThan(22);
    expect("Hello, streaming world".startsWith(midway)).toBe(true);
    await frames(REVEAL_MS + 16);
    expect(host.textContent).toBe("Hello, streaming world");
    await act(async () => root.unmount());
  });

  it("finishes a reveal that was in flight when the message settled", async () => {
    const host = document.createElement("div");
    const root = createRoot(host);
    await act(async () => root.render(<Probe text="A" streaming />));
    await act(async () => root.render(<Probe text="A longer reply" streaming />));
    await act(async () => root.render(<Probe text="A longer reply" streaming={false} />));
    await frames(REVEAL_MS * 2);
    expect(host.textContent).toBe("A longer reply");
    await act(async () => root.unmount());
  });

  it("shows the whole reply at once when only the status settles", async () => {
    // The terminal frame of a stream carries no new characters: the same text
    // the last delta delivered, with the row now settled. Only watching `text`
    // ignored that, so a reply could sit on a truncated prefix for the rest of
    // the reveal window while its own action bar was already showing. No frames
    // are advanced here on purpose: the row is settled, so there is nothing to
    // drain.
    const host = document.createElement("div");
    const root = createRoot(host);
    await act(async () => root.render(<Probe text="A" streaming />));
    await act(async () => root.render(<Probe text="A longer reply" streaming />));
    expect(host.textContent).toBe("A");
    await act(async () => root.render(<Probe text="A longer reply" streaming={false} />));
    expect(host.textContent).toBe("A longer reply");
    await act(async () => root.unmount());
  });
});

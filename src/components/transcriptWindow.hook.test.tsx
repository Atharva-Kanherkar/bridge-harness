// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useTranscriptWindow } from "./transcriptWindow";

function Probe({ total, sessionKey = "session", ready = true, full = false }: {
  total: number;
  sessionKey?: string;
  ready?: boolean;
  full?: boolean;
}) {
  const { start, instantBefore } = useTranscriptWindow(total, sessionKey, full, ready);
  return <div data-start={start} data-instant={instantBefore} />;
}

describe("useTranscriptWindow", () => {
  let container: HTMLDivElement;
  let root: Root | undefined;

  beforeEach(() => {
    vi.useFakeTimers();
    (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
    container = document.createElement("div");
    document.body.appendChild(container);
    root = createRoot(container);
  });

  afterEach(async () => {
    if (root) await act(async () => root!.unmount());
    root = undefined;
    container.remove();
    vi.useRealTimers();
  });

  const start = () => Number((container.firstElementChild as HTMLElement).dataset.start);
  const instant = () => Number((container.firstElementChild as HTMLElement).dataset.instant);

  it("opens on the newest rows", () => {
    act(() => root!.render(<Probe total={1500} />));
    expect(start()).toBe(1500 - 32);
    expect(instant()).toBe(1500 - 32);
  });

  it("keeps showing the newest rows when a rewind shrinks the branch", () => {
    act(() => root!.render(<Probe total={1500} />));
    expect(start()).toBe(1468);
    act(() => root!.render(<Probe total={100} />));
    // Not clamped to the row count: a start of 100 would render nothing.
    expect(start()).toBe(68);
  });

  it("mounts everything once a chat is short enough", () => {
    act(() => root!.render(<Probe total={120} />));
    expect(start()).toBe(88);
    act(() => root!.render(<Probe total={20} />));
    expect(start()).toBe(0);
  });

  it("does not seed from the previous chat's rows while the forest catches up", () => {
    act(() => root!.render(<Probe total={10} sessionKey="a" />));
    expect(start()).toBe(0);

    // The session switches first; the old forest is still in props.
    act(() => root!.render(<Probe total={10} sessionKey="b" ready={false} />));
    expect(start()).toBe(0);

    // The new forest arrives with 300 rows. Seeding from the ten-row chat
    // leaves a start of 0, so all 300 rows would mount on the first commit.
    act(() => root!.render(<Probe total={300} sessionKey="b" />));
    expect(start()).toBe(268);
  });

  it("drains hydration to the first row after a shrink past the stored start", async () => {
    act(() => root!.render(<Probe total={600} sessionKey="b" />));
    expect(start()).toBe(568);
    // A rewind shrinks the same chat. The stored start stays 568 while the
    // visible clamp is 168, so an idle slice must advance from the clamp.
    act(() => root!.render(<Probe total={200} sessionKey="b" />));
    expect(start()).toBe(168);

    for (let guard = 0; start() > 0 && guard < 100; guard += 1) {
      await act(async () => { vi.advanceTimersByTime(16); });
    }
    expect(start()).toBe(0);
  });

  it("keeps the old window on the mismatch commit when the next chat opens full", () => {
    act(() => root!.render(<Probe total={600} sessionKey="a" />));
    expect(start()).toBe(568);

    // The next chat has a remembered position, so its window will be full.
    // Until its forest arrives, `full` describes a chat that is not on screen;
    // applying it now would mount every row of the old chat.
    act(() => root!.render(<Probe total={600} sessionKey="b" ready={false} full />));
    expect(start()).toBe(568);

    act(() => root!.render(<Probe total={300} sessionKey="b" full />));
    expect(start()).toBe(0);
  });

  it("keeps the opening boundary fixed when the tail grows", () => {
    act(() => root!.render(<Probe total={300} />));
    expect(instant()).toBe(268);
    act(() => root!.render(<Probe total={340} />));
    // The newest 40 rows joined below the opening window; the boundary that
    // decides which rows arrive instantly does not move with them.
    expect(instant()).toBe(268);
  });
});

// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useTranscriptWindow } from "./transcriptWindow";

function Probe({ total }: { total: number }) {
  const start = useTranscriptWindow(total, "session", false);
  return <div data-start={start} />;
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

  it("opens on the newest rows", () => {
    act(() => root!.render(<Probe total={1500} />));
    expect(start()).toBe(1500 - 32);
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
});

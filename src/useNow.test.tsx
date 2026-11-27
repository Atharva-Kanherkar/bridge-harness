// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { useNow } from "./useNow";

let root: Root;
let host: HTMLDivElement;

beforeEach(() => {
  vi.useFakeTimers();
  vi.setSystemTime(1_000_000);
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
});

afterEach(() => {
  act(() => root.unmount());
  host.remove();
  vi.useRealTimers();
});

function Clock({ intervalMs }: { intervalMs: number | null }) {
  return <output>{useNow(intervalMs)}</output>;
}

const shown = () => Number(host.querySelector("output")?.textContent);

it("ticks every interval and stops ticking on null", () => {
  act(() => root.render(<Clock intervalMs={1000} />));
  expect(shown()).toBe(1_000_000);
  act(() => { vi.advanceTimersByTime(1000); });
  expect(shown()).toBe(1_001_000);
  act(() => root.render(<Clock intervalMs={null} />));
  act(() => { vi.advanceTimersByTime(5000); });
  expect(shown()).toBe(1_001_000);
  expect(vi.getTimerCount()).toBe(0);
});

it("re-reads the clock as soon as the interval changes", () => {
  act(() => root.render(<Clock intervalMs={null} />));
  vi.setSystemTime(2_000_000);
  act(() => root.render(<Clock intervalMs={1000} />));
  expect(shown()).toBe(2_000_000);
});

// @vitest-environment jsdom
import { act, useEffect, useLayoutEffect } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { useLatest } from "./useLatest";

let root: Root;
let host: HTMLDivElement;

beforeEach(() => {
  vi.useFakeTimers();
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

it("hands effects and handlers the newest value without re-subscribing", () => {
  const seen: string[] = [];
  let subscriptions = 0;
  function Reader({ value }: { value: string }) {
    const latest = useLatest(value);
    useEffect(() => {
      subscriptions += 1;
      const id = setInterval(() => seen.push(latest.current), 100);
      return () => clearInterval(id);
    }, [latest]);
    return null;
  }
  act(() => root.render(<Reader value="a" />));
  act(() => { vi.advanceTimersByTime(100); });
  act(() => root.render(<Reader value="b" />));
  act(() => { vi.advanceTimersByTime(100); });
  expect(seen).toEqual(["a", "b"]);
  expect(subscriptions).toBe(1);
});

it("is synced before the same component's later layout effects run", () => {
  const seen: number[] = [];
  function Reader({ value }: { value: number }) {
    const latest = useLatest(value);
    useLayoutEffect(() => { seen.push(latest.current); });
    return null;
  }
  act(() => root.render(<Reader value={1} />));
  act(() => root.render(<Reader value={2} />));
  expect(seen).toEqual([1, 2]);
});

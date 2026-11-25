// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { describe, expect, it } from "vitest";
import { sameItems, useShallowStableArray, useStableCallback } from "./useStableProps";

describe("useShallowStableArray", () => {
  it("keeps the previous array while the items are the same, and takes a changed one", async () => {
    const a = { id: 1 };
    const b = { id: 2 };
    const seen: Array<Array<{ id: number }>> = [];
    function Probe({ items }: { items: Array<{ id: number }> }) {
      seen.push(useShallowStableArray(items));
      return null;
    }
    const root = createRoot(document.createElement("div"));
    const first = [a, b];
    await act(async () => root.render(<Probe items={first} />));
    await act(async () => root.render(<Probe items={[a, b]} />));
    const changed = [a, { id: 2 }];
    await act(async () => root.render(<Probe items={changed} />));
    await act(async () => root.unmount());

    expect(seen[1]).toBe(first);
    expect(seen[2]).toBe(changed);
  });
});

describe("useStableCallback", () => {
  it("keeps one identity and calls the newest function", async () => {
    const seen: Array<() => string> = [];
    function Probe({ label }: { label: string }) {
      seen.push(useStableCallback(() => label));
      return null;
    }
    const root = createRoot(document.createElement("div"));
    await act(async () => root.render(<Probe label="first" />));
    await act(async () => root.render(<Probe label="second" />));

    expect(seen[0]).toBe(seen[1]);
    expect(seen[1]()).toBe("second");
    await act(async () => root.unmount());
  });
});

describe("sameItems", () => {
  it("compares by identity and order", () => {
    const item = {};
    expect(sameItems([item], [item])).toBe(true);
    expect(sameItems([item], [{}])).toBe(false);
    expect(sameItems([item], [item, item])).toBe(false);
  });
});

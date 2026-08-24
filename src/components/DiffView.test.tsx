// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { PatchView } from "./DiffView";

const PATCH = "@@ -1,2 +1,2 @@\n-const a = 1;\n+const a = 2;";

describe("PatchView", () => {
  let container: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    container = document.createElement("div");
    document.body.appendChild(container);
    root = createRoot(container);
  });

  afterEach(() => {
    act(() => { root.unmount(); });
    container.remove();
  });

  // Real dynamic-import + real Shiki tokenization, timed against the actual
  // wall clock — under a full, concurrent test-suite run, a single
  // `setTimeout(0)` tick isn't a reliable wait. Poll instead of guessing a
  // fixed delay.
  const waitFor = async (check: () => boolean, timeoutMs = 3000) => {
    const start = Date.now();
    while (!check()) {
      if (Date.now() - start > timeoutMs) throw new Error("timed out waiting for colorization");
      await act(async () => { await new Promise(resolve => setTimeout(resolve, 10)); });
    }
  };

  it("lays out rows, kinds and the gutter on first render, before any colour arrives", () => {
    // Sync act(): flushes the effect's immediate plain-escaped setRows call
    // without waiting for colorizePatch's promise, so this reliably observes
    // the pre-colour frame regardless of system load.
    act(() => { root.render(<PatchView patch={PATCH} path="a.ts" />); });
    const rows = container.querySelectorAll(".stx > div > div");
    expect(rows.length).toBeGreaterThan(0);
    expect(container.textContent).toContain("const a = 1;");
    expect(container.textContent).toContain("const a = 2;");
    expect(container.innerHTML).not.toContain("stx-keyword");
  });

  it("upgrades bodies to .stx-* coloured spans once the grammar loads", async () => {
    act(() => { root.render(<PatchView patch={PATCH} path="a.ts" />); });
    await waitFor(() => container.innerHTML.includes("stx-keyword"));
  });

  it("resets to plain text immediately on a new patch, instead of keeping the previous one's colour", async () => {
    act(() => { root.render(<PatchView patch={PATCH} path="a.ts" />); });
    await waitFor(() => container.innerHTML.includes("stx-keyword"));

    // A plain (non-async) act() observes the reset before `colorizePatch`'s
    // promise for the new patch has had a chance to resolve, regardless of
    // how warm the shared Shiki module cache already is.
    const NEXT_PATCH = "@@ -1 +1 @@\n-const b = 1;\n+const b = 2;";
    act(() => { root.render(<PatchView patch={NEXT_PATCH} path="a.ts" />); });
    expect(container.innerHTML).not.toContain("stx-keyword");
    expect(container.textContent).toContain("const b = 2;");
  });

  it("renders nothing for an empty patch", async () => {
    await act(async () => { root.render(<PatchView patch="" path="a.ts" />); });
    expect(container.innerHTML).toBe("");
  });
});

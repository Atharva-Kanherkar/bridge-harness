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
    // `.stx` is a column now — a scrolling rows area above an optional fold
    // bar — so the rows sit one level deeper than they used to.
    const rows = container.querySelectorAll(".stx > div > div > div");
    expect(rows.length).toBeGreaterThan(0);
    expect(container.textContent).toContain("const a = 1;");
    expect(container.textContent).toContain("const a = 2;");
    expect(container.innerHTML).not.toContain("stx-keyword");
  });

  it("gives an add and a del row a coloured run edge", async () => {
    // C18. An 8–10% body tint has no boundary against surrounding context, so
    // a short run inside a long file was near-invisible; the edge is the cue.
    await act(async () => { root.render(<PatchView patch={PATCH} path="src/a.ts" />); });
    const rowFor = (marker: string) => [...container.querySelectorAll("div.group\\/hunk")]
      .find(node => node.textContent?.includes(marker));
    const edge = (node: Element | undefined) =>
      node?.querySelector('span[aria-hidden].absolute')?.className ?? "";
    expect(edge(rowFor("const a = 2;"))).toContain("bg-success");
    expect(edge(rowFor("const a = 1;"))).toContain("bg-destructive");
    // Context rows have no run to bound.
    expect(edge(rowFor("@@"))).toBe("");
  });

  it("bands a hunk header across the gutter as well as the body", async () => {
    await act(async () => { root.render(<PatchView patch={PATCH} path="src/a.ts" />); });
    const hunk = [...container.querySelectorAll("div.group\\/hunk")]
      .find(node => node.textContent?.includes("@@"))!;
    const gutter = hunk.querySelector("span.sticky")!;
    expect(hunk.className).toContain("u-diff-band");
    expect(hunk.className).toContain("pt-px");
    expect(gutter.className).toContain("bg-inherit");
    expect(gutter.className).not.toMatch(/bg-(code|transparent)/);
  });

  it("clips add and del tints inside a rounded card", async () => {
    await act(async () => { root.render(<PatchView patch={PATCH} path="src/a.ts" />); });
    const frame = container.querySelector(".stx .rounded-sm")!;
    expect(frame.className).toContain("bg-card");
    expect(frame.className).toContain("overflow-hidden");
    expect(container.querySelector(".stx .p-1")).not.toBeNull();
    expect(container.firstElementChild!.className).toContain("bg-card");
    const added = [...container.querySelectorAll("div.group\\/hunk")]
      .find(node => node.textContent?.includes("const a = 2;"))!;
    expect(added.className).toContain("bg-success");
    expect(added.parentElement).toBe(frame);
  });

  it("does not draw a grey gutter border beside a coloured run edge", async () => {
    // Minor from review: 2px of colour abutting a 1px border read as a
    // three-pixel smear.
    await act(async () => { root.render(<PatchView patch={PATCH} path="src/a.ts" />); });
    const added = [...container.querySelectorAll("div.group\\/hunk")]
      .find(node => node.textContent?.includes("const a = 2;"))!;
    expect(added.querySelector("span.sticky")!.className).not.toContain("border-r");
  });

  it("shows one line number per row", () => {
    const patch = ["@@ -1,3 +1,3 @@", " keep", "-const a = 1;", "+const a = 2;"].join("\n");
    act(() => { root.render(<PatchView patch={patch} path="a.ts" />); });
    const numbers = (marker: string) => {
      const row = [...container.querySelectorAll("div.group\\/hunk")].find(node => node.textContent?.includes(marker))!;
      return [...row.querySelectorAll("span.sticky span")].filter(node => node.className.includes("tabular-nums"));
    };
    expect(numbers("keep")).toHaveLength(1);
    expect(numbers("keep")[0].textContent).toBe("1");
    expect(numbers("const a = 1;")).toHaveLength(1);
    expect(numbers("const a = 1;")[0].textContent).toBe("2");
    expect(numbers("const a = 2;")).toHaveLength(1);
    expect(numbers("const a = 2;")[0].textContent).toBe("2");
  });

  it("wraps long lines instead of overflowing the pane", () => {
    act(() => { root.render(<PatchView patch={PATCH} path="a.ts" />); });
    const body = container.querySelector("span.flex-1")!;
    expect(body.className).toContain("whitespace-pre-wrap");
    expect(body.className).toContain("break-words");
    expect(container.querySelector(".w-max")).toBeNull();
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

  // Folding by hunk is what lets an edit render inline by default: the reader
  // sees what changed first without a long patch taking over the transcript,
  // and — unlike the old `patch.slice(-8000)` — nothing is cut mid-hunk.
  describe("hunk folding", () => {
    const TWO_HUNKS = [
      "@@ -1,2 +1,2 @@",
      "-const a = 1;",
      "+const a = 2;",
      "@@ -40,2 +40,2 @@",
      "-const z = 9;",
      "+const z = 10;",
    ].join("\n");

    const foldBar = () => container.querySelector<HTMLButtonElement>(".stx button");

    it("shows the first hunk and folds the rest behind a bar", () => {
      act(() => { root.render(<PatchView patch={TWO_HUNKS} path="a.ts" foldAfterHunks={1} />); });
      expect(container.textContent).toContain("const a = 2;");
      expect(container.textContent).not.toContain("const z = 10;");
      expect(foldBar()?.textContent).toMatch(/1 more hunk\b.*expand/);
    });

    it("reveals the rest when the bar is pressed", () => {
      act(() => { root.render(<PatchView patch={TWO_HUNKS} path="a.ts" foldAfterHunks={1} />); });
      act(() => foldBar()!.click());
      expect(container.textContent).toContain("const a = 2;");
      expect(container.textContent).toContain("const z = 10;");
      expect(foldBar()).toBeNull();
    });

    it("pluralises the count", () => {
      const three = `${TWO_HUNKS}\n@@ -80,1 +80,1 @@\n+const q = 0;`;
      act(() => { root.render(<PatchView patch={three} path="a.ts" foldAfterHunks={1} />); });
      expect(foldBar()?.textContent).toMatch(/2 more hunks/);
    });

    it("adds no bar to a single-hunk patch", () => {
      act(() => { root.render(<PatchView patch={PATCH} path="a.ts" foldAfterHunks={1} />); });
      expect(foldBar()).toBeNull();
    });

    it("treats a fragment with no hunk header as one group", () => {
      act(() => { root.render(<PatchView patch={"+added line\n-removed line\n context"} path="a.ts" foldAfterHunks={1} />); });
      expect(container.textContent).toContain("added line");
      expect(foldBar()).toBeNull();
    });

    it("keeps the whole patch when no fold is asked for", () => {
      act(() => { root.render(<PatchView patch={TWO_HUNKS} path="a.ts" />); });
      expect(container.textContent).toContain("const z = 10;");
      expect(foldBar()).toBeNull();
    });

    it("re-folds when the patch is replaced, so one expansion does not leak into the next", () => {
      act(() => { root.render(<PatchView patch={TWO_HUNKS} path="a.ts" foldAfterHunks={1} />); });
      act(() => foldBar()!.click());
      expect(foldBar()).toBeNull();

      const other = TWO_HUNKS.replace("const z = 10;", "const z = 11;");
      act(() => { root.render(<PatchView patch={other} path="a.ts" foldAfterHunks={1} />); });
      expect(foldBar()?.textContent).toMatch(/1 more hunk\b/);
    });
  });
});

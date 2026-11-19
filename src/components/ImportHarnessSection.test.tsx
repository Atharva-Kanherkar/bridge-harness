// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { ImportHarnessSection } from "./ImportHarnessSection";

async function flush() {
  for (let i = 0; i < 5; i += 1) await new Promise(resolve => setTimeout(resolve, 0));
}

describe("ImportHarnessSection", () => {
  let container: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
    container = document.createElement("div");
    document.body.append(container);
    root = createRoot(container);
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
  });

  it("is the found tools and an Import button, with nothing to review first", async () => {
    await act(async () => { root.render(<ImportHarnessSection onError={error => { throw new Error(error); }} />); await flush(); });
    const text = container.textContent ?? "";
    expect(text).toContain("Import");
    expect(text).toContain("Everything stays on this Mac, and secrets are removed before anything is saved.");
    expect(text).toContain("Found on this Mac");
    for (const gone of ["Review", "Choose folder", "Choose export", "Choose a source", "final confirmation"]) {
      expect(text).not.toContain(gone);
    }
    const buttons = [...container.querySelectorAll("button")].map(button => button.textContent?.trim());
    expect(buttons).toEqual(["Import"]);
  });
});

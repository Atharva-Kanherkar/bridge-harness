// @vitest-environment jsdom
import { describe, expect, it, vi } from "vitest";
import { registerBrowserPageValidator, validateBrowserSelectionPage, type BrowserPageSnapshot } from "./browserRuntime";

const snapshot = (navigationId = 4): BrowserPageSnapshot => ({ url: "http://localhost:3000/", title: "Preview", navigationId, canGoBack: false, canGoForward: false, loading: false });
describe("live browser selection validation", () => {
  it("requires the current task, tab, and completed navigation", async () => {
    const read = vi.fn(async () => snapshot());
    const dispose = registerBrowserPageValidator("task-a", "tab-a", read);
    expect(await validateBrowserSelectionPage("task-a", "tab-a", 4)).toBe(true);
    expect(await validateBrowserSelectionPage("task-b", "tab-a", 4)).toBe(false);
    expect(await validateBrowserSelectionPage("task-a", "tab-a", 3)).toBe(false);
    read.mockResolvedValue({ ...snapshot(), loading: true });
    expect(await validateBrowserSelectionPage("task-a", "tab-a", 4)).toBe(false);
    read.mockResolvedValue({ ...snapshot(), error: "Failed" });
    expect(await validateBrowserSelectionPage("task-a", "tab-a", 4)).toBe(false);
    dispose();
    expect(await validateBrowserSelectionPage("task-a", "tab-a", 4)).toBe(false);
  });
  it("rejects an in-flight read after the page closes or its owner remounts", async () => {
    let finish!: (value: BrowserPageSnapshot) => void;
    const removeOld = registerBrowserPageValidator("task", "tab", () => new Promise(resolve => { finish = resolve; }));
    const pending = validateBrowserSelectionPage("task", "tab", 4);
    const removeNew = registerBrowserPageValidator("task", "tab", async () => snapshot(8));
    removeOld();
    finish(snapshot());
    expect(await pending).toBe(false);
    expect(await validateBrowserSelectionPage("task", "tab", 8)).toBe(true);
    removeNew();
  });
});

// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { bridgeApi } from "../api";
import type { ImportJob } from "../protocol/generated/protocol";
import { MoveToBridge } from "./MoveToBridge";

async function flush() {
  for (let i = 0; i < 5; i += 1) await new Promise(resolve => setTimeout(resolve, 0));
}

function button(container: HTMLElement, label: string) {
  const result = [...container.querySelectorAll("button")].find(item => item.textContent?.trim().startsWith(label));
  if (!result) throw new Error(`Button ${label} was not rendered: ${container.textContent}`);
  return result as HTMLButtonElement;
}

const job = (fields: Partial<ImportJob>): ImportJob => ({
  jobId: "job-1", tool: "claude_code", status: "running", total: 0, processed: 0, imported: 0, memories: 0,
  alreadyInBridge: 0, changed: 0, failed: 0, leftOut: 0, notCarried: [], errors: [],
  startedAt: "2026-11-19T10:00:00Z", updatedAt: "2026-11-19T10:00:00Z", ...fields,
});

describe("MoveToBridge", () => {
  let container: HTMLDivElement;
  let root: Root;
  let emit: (job: ImportJob) => void;
  let unlisten: ReturnType<typeof vi.fn>;

  beforeEach(() => {
    (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
    container = document.createElement("div");
    document.body.append(container);
    root = createRoot(container);
    unlisten = vi.fn();
    vi.spyOn(bridgeApi, "onImportProgress").mockImplementation(async handler => { emit = handler; return unlisten; });
    vi.spyOn(bridgeApi, "importStatus").mockResolvedValue({});
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    vi.restoreAllMocks();
    container.remove();
  });

  const render = async () => {
    await act(async () => { root.render(<MoveToBridge onError={error => { throw new Error(error); }} />); await flush(); });
  };
  // Counts glide to each new value over 450 ms; wait for them to land.
  const send = async (next: ImportJob) => {
    await act(async () => { emit(next); await flush(); });
    await act(async () => { await new Promise(resolve => setTimeout(resolve, 600)); });
  };

  it("lists every tool with one Import button, only where Bridge can import today", async () => {
    await render();
    const text = container.textContent ?? "";
    expect(text).toContain("Found on this Mac");
    expect(text).toContain("3 chats in 1 project · 1 memory · 1 setup item");
    expect(text).toContain("Not found on this Mac");
    expect(text).not.toContain("Review");
    expect(container.querySelectorAll("button").length).toBe(1);
    expect(button(container, "Import").disabled).toBe(false);
  });

  it("starts the import in the background and shows its progress live", async () => {
    const start = vi.spyOn(bridgeApi, "startImport").mockResolvedValue(job({}));
    await render();

    await act(async () => { button(container, "Import").click(); await flush(); });

    expect(start).toHaveBeenCalledWith("claude_code");
    expect(container.textContent).toContain("Finding your chats…");
    expect(button(container, "Importing…").disabled).toBe(true);

    await send(job({ total: 400, processed: 100, imported: 90, alreadyInBridge: 10, currentProject: "/Users/you/work/invoice-app", updatedAt: "2026-11-19T10:00:05Z" }));
    const bar = container.querySelector("[role='progressbar']")!;
    expect(bar.getAttribute("aria-valuenow")).toBe("25");
    expect(container.textContent).toContain("100 of 400");
    expect(container.textContent).toContain("90 came over");
    expect(container.textContent).toContain("10 already in Bridge");
    expect(container.textContent).toContain("Now reading …/work/invoice-app");
    expect(container.textContent).toContain("You can leave this page or close the window");
  });

  it("shows what came over, what was left out and what did not come over when it ends", async () => {
    vi.spyOn(bridgeApi, "startImport").mockResolvedValue(job({}));
    await render();
    await act(async () => { button(container, "Import").click(); await flush(); });

    await send(job({
      status: "done", total: 400, processed: 400, imported: 380, memories: 3, alreadyInBridge: 15, failed: 1, leftOut: 51190,
      notCarried: [{ text: "documents or PDFs inside messages", count: 4 }], errors: [],
      updatedAt: "2026-11-19T10:09:00Z", finishedAt: "2026-11-19T10:09:00Z",
    }));

    const text = container.textContent ?? "";
    expect(text).toContain("Import finished");
    expect(container.querySelector("[data-import-result='imported']")?.textContent).toBe("380");
    expect(container.querySelector("[data-import-result='skipped']")?.textContent).toBe("15");
    expect(text).toContain("Background runs left out");
    expect(text).toContain("51,190");
    expect(text).toContain("4 × documents or PDFs inside messages");
    expect(text).toContain("Could not be imported");
    expect(container.querySelector("[role='progressbar']")).toBeNull();
    expect(button(container, "Import again").disabled).toBe(false);
  });

  it("opened in the middle of an import, shows that import instead of a fresh button", async () => {
    vi.spyOn(bridgeApi, "importStatus").mockResolvedValue({ job: job({ total: 50, processed: 20, imported: 20, updatedAt: "2026-11-19T10:00:03Z" }) });
    await render();

    expect(container.textContent).toContain("20 of 50");
    expect(button(container, "Importing…").disabled).toBe(true);
  });

  it("keeps the newest progress when an older status read arrives late", async () => {
    let resolveStatus: (value: { job?: ImportJob }) => void = () => undefined;
    vi.spyOn(bridgeApi, "importStatus").mockReturnValue(new Promise(resolve => { resolveStatus = resolve; }));
    await render();
    await send(job({ total: 50, processed: 40, updatedAt: "2026-11-19T10:00:09Z" }));

    await act(async () => { resolveStatus({ job: job({ total: 50, processed: 10, updatedAt: "2026-11-19T10:00:02Z" }) }); await flush(); });

    expect(container.textContent).toContain("40 of 50");
  });

  it("stops listening when the page closes", async () => {
    await render();
    await act(async () => root.unmount());
    expect(unlisten).toHaveBeenCalled();
    root = createRoot(container);
  });
});

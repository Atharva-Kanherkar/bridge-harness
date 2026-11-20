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
  jobId: "job-1", tool: "claude_code", status: "running", total: 0, processed: 0, imported: 0,
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

  const section = (tool: string) => container.querySelector(`[data-import-tool="${tool}"]`) as HTMLElement;

  it("gives every tool its own section, with its mark and, where Bridge can import it, its own button", async () => {
    await render();
    const text = container.textContent ?? "";
    expect(text).toContain("Found on this Mac");
    expect(text).not.toContain("Review");
    const tools = [...container.querySelectorAll("[data-import-tool]")].map(item => item.getAttribute("data-import-tool"));
    expect(tools).toEqual(["claude_code", "codex", "opencode", "conductor", "t3code", "orca", "superset"]);
    expect(section("claude_code").textContent).toContain("3 chats in 1 project · 1 setup item");
    expect(section("claude_code").textContent).toContain("Claude Code keeps using its own memory");
    expect(section("codex").textContent).toContain("Codex keeps using its own memory");
    expect(section("claude_code").querySelector("[data-tool-mark] svg[data-harness='claude']")).not.toBeNull();
    expect(section("codex").querySelector("[data-tool-mark] svg[data-harness='codex']")).not.toBeNull();
    expect(section("opencode").textContent).toContain("9 chats in 3 projects");
    expect(section("opencode").textContent).toContain("OpenCode keeps its own memory and setup");
    expect(section("opencode").querySelector("[data-tool-mark] svg[data-harness='opencode']")).not.toBeNull();
    expect(section("conductor").querySelector("[data-tool-mark]")?.textContent).toBe("C");
    expect(button(section("claude_code"), "Import").disabled).toBe(false);
    expect(button(section("codex"), "Import").disabled).toBe(false);
    expect(button(section("opencode"), "Import").disabled).toBe(false);
    expect(button(section("conductor"), "Import").disabled).toBe(false);
    expect(section("conductor").textContent).toContain("5 workspaces in 2 repos");
    expect(section("conductor").textContent).toContain("left where they are");
    expect(section("t3code").textContent).toContain("Not found on this Mac");
    expect(container.querySelectorAll("button").length).toBe(4);
  });

  it("each tool keeps its own last import", async () => {
    vi.spyOn(bridgeApi, "importStatus").mockResolvedValue({ jobs: [
      job({ jobId: "c", tool: "codex", status: "done", total: 7, processed: 7, imported: 7, finishedAt: "2026-11-20T10:00:00Z" }),
      job({ jobId: "a", tool: "claude_code", status: "done", total: 5, processed: 5, imported: 5, finishedAt: "2026-11-19T10:00:00Z" }),
    ] });
    await render();
    expect(section("claude_code").querySelector("[data-import-result='imported']")?.textContent).toBe("5");
    expect(section("codex").querySelector("[data-import-result='imported']")?.textContent).toBe("7");
    expect(button(section("claude_code"), "Import again").disabled).toBe(false);
    expect(button(section("codex"), "Import again").disabled).toBe(false);
  });

  it("starts the import in the background and shows its progress live", async () => {
    const start = vi.spyOn(bridgeApi, "startImport").mockResolvedValue(job({}));
    await render();

    await act(async () => { button(container, "Import").click(); await flush(); });

    expect(start).toHaveBeenCalledWith("claude_code");
    expect(section("claude_code").textContent).toContain("Finding your chats…");
    expect(button(section("claude_code"), "Importing…").disabled).toBe(true);
    expect(button(section("codex"), "Import").disabled).toBe(true);
    expect(section("codex").textContent).toContain("Waits for the import that is running.");
    expect(section("codex").querySelector("[aria-label='Import progress']")).toBeNull();

    await send(job({ total: 400, processed: 100, imported: 90, alreadyInBridge: 10, currentProject: "/Users/you/work/invoice-app", updatedAt: "2026-11-19T10:00:05Z" }));
    const bar = container.querySelector("[role='progressbar']")!;
    expect(bar.getAttribute("aria-valuenow")).toBe("25");
    expect(container.textContent).toContain("100 of 400");
    expect(container.textContent).toContain("90 came over");
    expect(container.textContent).toContain("10 already in Bridge");
    expect(container.textContent).toContain("Now reading …/work/invoice-app");
    expect(container.textContent).toContain("You can leave this page or close the window");
  });

  it("draws the transfer: chats arcing from Claude Code to Bridge, with the time left", async () => {
    vi.spyOn(bridgeApi, "startImport").mockResolvedValue(job({}));
    await render();
    await act(async () => { button(container, "Import").click(); await flush(); });
    await send(job({ total: 400, processed: 100, imported: 100, startedAt: new Date(Date.now() - 20_000).toISOString(), updatedAt: "2026-11-19T10:00:05Z" }));

    const stage = container.querySelector("[data-transfer]")!;
    expect(stage.getAttribute("data-transfer")).toBe("running");
    expect(stage.textContent).toContain("Claude Code");
    expect(stage.textContent).toContain("Bridge");
    expect(stage.querySelectorAll(".import-transfer-card").length).toBe(6);
    expect(stage.querySelector(".import-transfer-check")).toBeNull();
    expect(container.querySelector("[data-import-time-left]")?.textContent).toBe("Less than a minute left");
  });

  it("draws a Codex import from Codex's tile, in Codex's section", async () => {
    vi.spyOn(bridgeApi, "importStatus").mockResolvedValue({ job: job({ tool: "codex", total: 40, processed: 10, imported: 10, updatedAt: "2026-11-19T10:00:03Z" }) });
    await render();
    expect(section("claude_code").querySelector("[data-transfer]")).toBeNull();
    const stage = section("codex").querySelector("[data-transfer]")!;
    expect(stage.textContent).toContain("Codex");
    expect(stage.textContent).not.toContain("Claude Code");
    expect(stage.querySelector("[data-harness='codex']")).not.toBeNull();
  });

  it("imports OpenCode from OpenCode's own section, drawn from OpenCode's tile", async () => {
    const start = vi.spyOn(bridgeApi, "startImport").mockResolvedValue(job({ tool: "opencode" }));
    await render();

    await act(async () => { button(section("opencode"), "Import").click(); await flush(); });

    expect(start).toHaveBeenCalledWith("opencode");
    expect(button(section("opencode"), "Importing…").disabled).toBe(true);
    expect(button(section("claude_code"), "Import").disabled).toBe(true);
    expect(section("claude_code").textContent).toContain("Waits for the import that is running.");
    await send(job({ tool: "opencode", total: 30, processed: 10, imported: 10, updatedAt: "2026-11-19T10:00:05Z" }));
    const stage = section("opencode").querySelector("[data-transfer]")!;
    expect(stage.textContent).toContain("OpenCode");
    expect(stage.querySelector("[data-harness='opencode']")).not.toBeNull();
    expect(section("claude_code").querySelector("[data-transfer]")).toBeNull();
  });

  it("imports Conductor's workspaces and says the archived ones were left out", async () => {
    vi.spyOn(bridgeApi, "importStatus").mockResolvedValue({ jobs: [
      job({ jobId: "w", tool: "conductor", status: "done", total: 6, processed: 6, imported: 4, alreadyInBridge: 0, leftOut: 1, notCarried: [{ text: "workspaces on a detached HEAD, which need a branch first", count: 1 }], finishedAt: "2026-11-20T10:00:00Z" }),
    ] });
    await render();
    const conductor = section("conductor");
    expect(conductor.querySelector("[data-import-result='imported']")?.textContent).toBe("4");
    expect(conductor.textContent).toContain("Archived in Conductor, left out");
    expect(conductor.textContent).not.toContain("Background runs left out");
    expect(conductor.textContent).toContain("so Conductor keeps working with them");
    expect(conductor.textContent).toContain("need a branch first");
    expect(conductor.querySelector("[data-tool-initial='conductor']")?.textContent).toBe("C");
  });

  it("lands the transfer with a check when the import ends", async () => {
    vi.spyOn(bridgeApi, "startImport").mockResolvedValue(job({}));
    await render();
    await act(async () => { button(container, "Import").click(); await flush(); });
    await send(job({ status: "done", total: 4, processed: 4, imported: 4, updatedAt: "2026-11-19T10:09:00Z", finishedAt: "2026-11-19T10:09:00Z" }));

    const stage = container.querySelector("[data-transfer]")!;
    expect(stage.getAttribute("data-transfer")).toBe("done");
    expect(stage.querySelectorAll(".import-transfer-card").length).toBe(0);
    expect(stage.querySelector(".import-transfer-check")).not.toBeNull();
  });

  it("shows what came over, what was left out and what did not come over when it ends", async () => {
    vi.spyOn(bridgeApi, "startImport").mockResolvedValue(job({}));
    await render();
    await act(async () => { button(container, "Import").click(); await flush(); });

    await send(job({
      status: "done", total: 400, processed: 400, imported: 380, alreadyInBridge: 15, failed: 1, leftOut: 51190,
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
    expect(button(section("claude_code"), "Import again").disabled).toBe(false);
    expect(button(section("codex"), "Import").disabled).toBe(false);
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

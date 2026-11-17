// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { bridgeApi } from "../api";
import type { ExternalImportArtifact, ExternalImportCandidate, ExternalImportDiscovery } from "../protocol/generated/protocol";
import { MoveToBridge } from "./MoveToBridge";

async function flush() {
  for (let i = 0; i < 5; i += 1) await new Promise(resolve => setTimeout(resolve, 0));
}

function button(container: HTMLElement, label: string) {
  const result = [...container.querySelectorAll("button")].find(item => item.textContent?.trim().startsWith(label));
  if (!result) throw new Error(`Button ${label} was not rendered: ${container.textContent}`);
  return result as HTMLButtonElement;
}

const artifact = (index: number): ExternalImportArtifact => ({
  artifactId: `a${index}`, canonicalSourceRef: `/h/.claude/projects/p/${index}.jsonl`, sourceLabel: `projects/p/${index}.jsonl`,
  kind: "conversation", classification: "version_gated_private", stability: "version_gated", estimatedBytes: 100, modifiedAt: null,
  requiredSchemaGate: "claude-jsonl-v2", title: `Chat ${index}`, projectPath: index % 2 ? "/work/api" : "/work/web",
  startedAt: `2026-11-${String((index % 28) + 1).padStart(2, "0")}T10:00:00Z`,
});

const candidateFor = (id: string): ExternalImportCandidate => ({
  candidateId: `cand-${id}`, source: { provider: "claude_code", adapterVersion: "2", sourceVersion: null, schemaVersion: "claude-jsonl-v2", canonicalSourceRef: id, sourcePathFingerprint: "f", discoveredAt: "now", sourceMetadata: {} },
  sourceNativeId: id, kind: "conversation", title: id, createdAt: null, updatedAt: null, projectHint: null, contentHash: "h", stability: "version_gated",
  confidenceBps: 8000, selectedByDefault: false, redactionSummary: { structuredFieldsExcluded: 0, textValuesRedacted: 0, categories: [], safelyRepresentable: true },
  diagnostics: id === "a3" ? [{ code: "excluded_attachment", severity: "warning", classification: "documented", message: "1 attachment: image too large or not an image; the message text was kept", recovery: null, sourceLabel: id }] : [],
  normalizedPayload: { messageCount: 4 },
});

describe("MoveToBridge", () => {
  let container: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
    container = document.createElement("div");
    document.body.append(container);
    root = createRoot(container);
  });

  afterEach(async () => {
    vi.restoreAllMocks();
    await act(async () => root.unmount());
    container.remove();
  });

  it("lists every tool on first open, with Review only where Bridge can import today", async () => {
    await act(async () => { root.render(<MoveToBridge onError={error => { throw new Error(error); }} />); await flush(); });
    const text = container.textContent ?? "";
    expect(text).toContain("Found on this Mac");
    expect(text).toContain("3 chats in 1 project · 1 memory · 1 setup item");
    expect(text).toContain("Conductor runs Claude Code, so its chats come over with the Claude Code import");
    expect(text).toContain("5 workspaces in 2 repos");
    expect(text).toContain("Not found on this Mac");
    expect(container.querySelectorAll("button").length).toBe(1);
    expect(button(container, "Review").disabled).toBe(false);
  });

  it("reviews by project, imports 30 chats in two batches scoped to their artifacts, and reports what did not come over", async () => {
    const artifacts = Array.from({ length: 30 }, (_, index) => artifact(index));
    const discovery: ExternalImportDiscovery = { discoveryId: "d1", provider: "claude_code", approvedRoots: ["/h/.claude"], sourceVersion: null, formatVersions: {}, artifacts, diagnostics: [], discoveredAt: "now" };
    vi.spyOn(bridgeApi, "discoverExternalImport").mockResolvedValue(discovery);
    const preview = vi.spyOn(bridgeApi, "previewExternalImport").mockImplementation(async (_, ids) => ({ candidates: ids.map(candidateFor) }));
    const commit = vi.spyOn(bridgeApi, "commitExternalImport").mockImplementation(async (_, __, plan) => ({
      importId: "i", candidateResults: [], createdBridgeIds: [], imported: plan.selectedCandidateIds.length - 1, skipped: 1, changed: 0, conflicted: 0, rejected: 0, unsupported: 0, rollbackState: "committed", diagnostics: [], createdAt: "now",
    }));
    await act(async () => { root.render(<MoveToBridge onError={error => { throw new Error(error); }} />); await flush(); });
    await act(async () => { button(container, "Review").click(); await flush(); });
    const projects = [...container.querySelectorAll("[data-import-project]")].map(node => node.getAttribute("data-import-project"));
    expect(projects).toEqual(["/work/api", "/work/web"]);
    expect(container.textContent).toContain("30 of 30 chats selected");
    expect(container.textContent).toContain("Show all 15");
    // Turn one project off: its 15 chats leave the selection.
    const projectSwitch = container.querySelector<HTMLButtonElement>('[role="switch"][aria-label="All chats in /work/web"]')!;
    await act(async () => { projectSwitch.click(); await flush(); });
    expect(container.textContent).toContain("15 of 30 chats selected");
    await act(async () => { projectSwitch.click(); await flush(); });
    await act(async () => { button(container, "Import 30 selected").click(); await flush(); });
    expect(preview).toHaveBeenCalledTimes(2);
    expect(preview.mock.calls.map(call => call[1].length)).toEqual([25, 5]);
    expect(commit).toHaveBeenCalledTimes(2);
    // Each commit names exactly the artifacts its preview read.
    expect(commit.mock.calls.map(call => call[3]?.length)).toEqual([25, 5]);
    expect(commit.mock.calls[0][2].memoryScope).toBeNull();
    expect(container.querySelector('[data-import-result="imported"]')?.textContent).toBe("28");
    expect(container.querySelector('[data-import-result="skipped"]')?.textContent).toBe("2");
    expect(container.textContent).toContain("1 × attachment: image too large or not an image; the message text was kept");
    await act(async () => { button(container, "Start over").click(); await flush(); });
    expect(container.textContent).toContain("Found on this Mac");
  });

  it("keeps going after a batch fails and says so", async () => {
    const artifacts = Array.from({ length: 26 }, (_, index) => artifact(index));
    vi.spyOn(bridgeApi, "discoverExternalImport").mockResolvedValue({ discoveryId: "d", provider: "claude_code", approvedRoots: [], sourceVersion: null, formatVersions: {}, artifacts, diagnostics: [], discoveredAt: "now" });
    vi.spyOn(bridgeApi, "previewExternalImport").mockImplementationOnce(async () => { throw new Error("The Bridge daemon connection was lost"); })
      .mockImplementation(async (_, ids) => ({ candidates: ids.map(candidateFor) }));
    vi.spyOn(bridgeApi, "commitExternalImport").mockResolvedValue({ importId: "i", candidateResults: [], createdBridgeIds: [], imported: 1, skipped: 0, changed: 0, conflicted: 0, rejected: 0, unsupported: 0, rollbackState: "committed", diagnostics: [], createdAt: "now" });
    await act(async () => { root.render(<MoveToBridge onError={error => { throw new Error(error); }} />); await flush(); });
    await act(async () => { button(container, "Review").click(); await flush(); });
    await act(async () => { button(container, "Import 26 selected").click(); await flush(); });
    expect(container.textContent).toContain("The Bridge daemon connection was lost");
    expect(container.querySelector('[data-import-result="imported"]')?.textContent).toBe("1");
  });

  it("reports a failed detection instead of spinning forever", async () => {
    vi.spyOn(bridgeApi, "detectImportSources").mockRejectedValue(new Error("HOME is not set"));
    const errors: string[] = [];
    await act(async () => { root.render(<MoveToBridge onError={error => errors.push(error)} />); await flush(); });
    expect(errors).toEqual(["HOME is not set"]);
    expect(container.textContent).not.toContain("Looking for other tools");
  });
});

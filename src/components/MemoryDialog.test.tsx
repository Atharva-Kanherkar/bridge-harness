// @vitest-environment jsdom
// The dialog's contract (testing/feat-memory-ui.md): list, filter, save,
// forget under account:local; refetch on memory-changed behind a read
// generation; refuse over the cap instead of clipping; keep no state closed;
// and say plainly what this surface is not.
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { bridgeApi } from "../api";
import type { MemoryChangedPayload, MemoryExtractionSettings, MemoryRecord } from "../types";
import { MemoryDialog, rememberAction } from "./MemoryDialog";

const record = (id: string, body: string, kind = "preference"): MemoryRecord => ({
  id,
  scopeKey: "account:local",
  kind,
  body,
  provenance: "user_explicit",
  status: "active",
  createdAt: "2026-08-20T00:00:00Z",
  updatedAt: "2026-08-20T00:00:00Z",
});

let container: HTMLDivElement;
let root: Root;
let store: MemoryRecord[];
let extractionSettings: MemoryExtractionSettings;
let memoryHandler: ((payload: MemoryChangedPayload) => void) | undefined;

const flush = async () => { await act(async () => {}); };

function mount(overrides: Partial<Parameters<typeof MemoryDialog>[0]> = {}) {
  act(() => {
    root.render(<MemoryDialog open onClose={() => {}} onError={() => {}} {...overrides} />);
  });
}

const textarea = () => document.querySelector<HTMLTextAreaElement>("textarea")!;
const buttonByText = (needle: string) =>
  [...document.querySelectorAll<HTMLButtonElement>("button")].find(button => button.textContent?.includes(needle))!;
const click = (element: Element) => {
  act(() => { element.dispatchEvent(new MouseEvent("click", { bubbles: true })); });
};
const setBody = (value: string) => {
  const setter = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")!.set!;
  act(() => {
    setter.call(textarea(), value);
    textarea().dispatchEvent(new Event("input", { bubbles: true }));
  });
};

beforeEach(() => {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
  store = [
    record("r-tabs", "Prefers tabs over spaces"),
    record("r-tz", "Works in IST", "fact"),
    {
      ...record("r-prop", "Deploys only on Fridays", "constraint"),
      status: "proposed",
      provenance: "model_proposal",
      confidenceBps: 8200,
      rationale: "Said twice in one chat",
    },
  ];
  memoryHandler = undefined;
  extractionSettings = { scopeKey: "account:local", mode: "remember" };
  vi.spyOn(bridgeApi, "listMemoryRecords").mockImplementation(async (scopeKey, status) => ({
    scopeKey,
    records: store.filter(item => item.status === (status ?? "active")),
  }));
  vi.spyOn(bridgeApi, "getExtractionSettings").mockImplementation(async () => structuredClone(extractionSettings));
  vi.spyOn(bridgeApi, "updateExtractionSettings").mockImplementation(async (mode, harness, model) => {
    if (mode === "auto_apply") throw new Error("Auto-apply does not exist until a replay bench can justify it.");
    if (mode === "propose" && (!harness || !model)) throw new Error("Propose mode needs a pinned harness and model to run on.");
    extractionSettings = { ...extractionSettings, mode, harness: harness ?? undefined, model: model ?? undefined };
    return structuredClone(extractionSettings);
  });
  vi.spyOn(bridgeApi, "supersedeMemoryRecord").mockImplementation(async (recordId, newBody, newKind) => {
    const old = store.find(item => item.id === recordId && item.status === "active")!;
    old.status = "superseded";
    const replacement: MemoryRecord = {
      ...record(`r-edit-${store.length}`, newBody, newKind ?? old.kind),
      supersedes: old.id,
    };
    store = [replacement, ...store];
    memoryHandler?.({ scopeKey: "account:local" });
    return replacement;
  });
  vi.spyOn(bridgeApi, "approveMemoryRecord").mockImplementation(async recordId => {
    const found = store.find(item => item.id === recordId && item.status === "proposed")!;
    found.status = "active";
    memoryHandler?.({ scopeKey: "account:local" });
    return found;
  });
  vi.spyOn(bridgeApi, "rejectMemoryRecord").mockImplementation(async recordId => {
    const found = store.find(item => item.id === recordId && item.status === "proposed")!;
    found.status = "rejected";
    memoryHandler?.({ scopeKey: "account:local" });
    return found;
  });
  vi.spyOn(bridgeApi, "saveMemoryRecord").mockImplementation(async (body, kind) => {
    const saved = record(`r-${store.length}`, body, kind ?? "preference");
    store = [saved, ...store];
    memoryHandler?.({ scopeKey: "account:local" });
    return saved;
  });
  vi.spyOn(bridgeApi, "deleteMemoryRecord").mockImplementation(async recordId => {
    const found = store.find(item => item.id === recordId)!;
    found.status = "deleted";
    memoryHandler?.({ scopeKey: "account:local" });
    return found;
  });
  vi.spyOn(bridgeApi, "getMemoryCapabilities").mockResolvedValue({
    ledger: { exists: true, scopeKey: "account:local", maxBodyChars: 4000, kinds: ["preference", "fact", "decision", "constraint"] },
    providerNative: [{ harness: "claude", command: "memory", description: "Edit CLAUDE.md memory files" }],
  });
  vi.spyOn(bridgeApi, "onMemoryChanged").mockImplementation(async handler => {
    memoryHandler = handler;
    return () => { memoryHandler = undefined; };
  });
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.restoreAllMocks();
});

describe("MemoryDialog", () => {
  it("lists account pins and names what this surface is not", async () => {
    mount();
    await flush();
    expect(container.textContent).toContain("Prefers tabs over spaces");
    expect(container.textContent).toContain("Works in IST");
    expect(container.textContent).toContain("not the helper picker");
    expect(container.textContent).toContain("Not this chat's history");
    expect(container.textContent).toContain("account:local");
  });

  it("names provider-owned memory as staying on its provider", async () => {
    mount();
    await flush();
    expect(container.textContent).toContain("Provider-owned memory");
    expect(container.textContent).toContain("/memory");
    expect(container.textContent).toContain("Stays on that provider");
  });

  it("kind chips filter client-side, and a second click clears the filter", async () => {
    mount();
    await flush();
    click(buttonByText("fact"));
    expect(container.textContent).toContain("Works in IST");
    expect(container.textContent).not.toContain("Prefers tabs over spaces");
    click(buttonByText("fact"));
    expect(container.textContent).toContain("Prefers tabs over spaces");
  });

  it("saving goes through the api and the list refreshes on the hint", async () => {
    mount();
    await flush();
    setBody("Always run bun run check first");
    click(buttonByText("Save pin"));
    await flush();
    expect(bridgeApi.saveMemoryRecord).toHaveBeenCalledWith("Always run bun run check first", "preference", undefined);
    expect(container.textContent).toContain("Always run bun run check first");
    expect(textarea().value).toBe("");
  });

  it("refuses over the cap with a visible count instead of clipping", async () => {
    mount();
    await flush();
    setBody("x".repeat(4001));
    expect(container.textContent).toContain("4001 / 4000");
    expect(buttonByText("Save pin").disabled).toBe(true);
    click(buttonByText("Save pin"));
    await flush();
    expect(bridgeApi.saveMemoryRecord).not.toHaveBeenCalled();
  });

  it("forget tombstones through the api and the row leaves the list", async () => {
    mount();
    await flush();
    click([...document.querySelectorAll('[aria-label="Forget"]')][0]);
    await flush();
    expect(bridgeApi.deleteMemoryRecord).toHaveBeenCalledWith("r-tabs");
    expect(container.textContent).not.toContain("Prefers tabs over spaces");
  });

  it("a slow earlier read cannot clobber a newer one", async () => {
    let releaseSlow!: () => void;
    const slow = new Promise<void>(resolve => { releaseSlow = resolve; });
    vi.spyOn(bridgeApi, "listMemoryRecords")
      .mockImplementationOnce(async () => {
        await slow;
        return { scopeKey: "account:local", records: [record("r-stale", "Stale view")] };
      })
      .mockImplementation(async () => ({ scopeKey: "account:local", records: [record("r-fresh", "Fresh view")] }));
    mount();
    await flush();
    act(() => { memoryHandler?.({ scopeKey: "account:local" }); });
    await flush();
    expect(container.textContent).toContain("Fresh view");
    releaseSlow();
    await flush();
    expect(container.textContent).toContain("Fresh view");
    expect(container.textContent).not.toContain("Stale view");
  });

  it("opens pre-filled from an oversize remember and issues no save", async () => {
    mount({ initialBody: "y".repeat(4001) });
    await flush();
    expect(textarea().value).toBe("y".repeat(4001));
    expect(buttonByText("Save pin").disabled).toBe(true);
    expect(bridgeApi.saveMemoryRecord).not.toHaveBeenCalled();
  });

  it("a closed dialog keeps no state", async () => {
    mount();
    await flush();
    setBody("draft that must not survive");
    act(() => {
      root.render(<MemoryDialog open={false} onClose={() => {}} onError={() => {}} />);
    });
    mount();
    await flush();
    expect(textarea().value).toBe("");
  });
});

describe("MemoryDialog edit", () => {
  it("edit supersedes: the composer prefills and save writes a replacement, never a new pin", async () => {
    mount();
    await flush();
    click([...document.querySelectorAll('[aria-label="Edit"]')][0]);
    expect(textarea().value).toBe("Prefers tabs over spaces");
    expect(buttonByText("Save edit")).toBeDefined();
    setBody("Prefers spaces after all");
    click(buttonByText("Save edit"));
    await flush();
    expect(bridgeApi.supersedeMemoryRecord).toHaveBeenCalledWith("r-tabs", "Prefers spaces after all", "preference");
    expect(bridgeApi.saveMemoryRecord).not.toHaveBeenCalled();
    expect(container.textContent).toContain("Prefers spaces after all");
    expect(container.textContent).not.toContain("Prefers tabs over spaces");
    expect(container.textContent).toContain("replaced an earlier pin");
  });

  it("cancelling an edit restores the plain composer and writes nothing", async () => {
    mount();
    await flush();
    click([...document.querySelectorAll('[aria-label="Edit"]')][0]);
    click(buttonByText("Cancel"));
    expect(textarea().value).toBe("");
    expect(buttonByText("Save pin")).toBeDefined();
    expect(bridgeApi.supersedeMemoryRecord).not.toHaveBeenCalled();
    expect(bridgeApi.saveMemoryRecord).not.toHaveBeenCalled();
  });

  it("an approved suggestion is chipped as suggested; explicit pins are not", async () => {
    store.push({
      ...record("r-approved", "Ships behind a flag", "decision"),
      provenance: "model_proposal",
      confidenceBps: 7600,
    });
    mount();
    await flush();
    expect(container.textContent).toContain("suggested");
    expect(container.textContent).toContain("76% confident");
    const explicitRow = [...container.querySelectorAll("li")].find(item => item.textContent?.includes("Works in IST"))!;
    expect(explicitRow.textContent).not.toContain("suggested");
    expect(explicitRow.textContent).not.toContain("% confident");
  });
});

describe("MemoryDialog review queue", () => {
  it("lists proposals with confidence and rationale, and pins carry no fake confidence", async () => {
    mount();
    await flush();
    click(buttonByText("Review queue"));
    expect(container.textContent).toContain("Deploys only on Fridays");
    expect(container.textContent).toContain("82% confident");
    expect(container.textContent).toContain("Said twice in one chat");
    click(buttonByText("About me"));
    expect(container.textContent).not.toContain("% confident");
  });

  it("approve activates through the api and the row leaves the queue", async () => {
    mount();
    await flush();
    click(buttonByText("Review queue"));
    click(buttonByText("Approve"));
    await flush();
    expect(bridgeApi.approveMemoryRecord).toHaveBeenCalledWith("r-prop");
    expect(container.textContent).toContain("Nothing to review");
    click(buttonByText("About me"));
    expect(container.textContent).toContain("Deploys only on Fridays");
  });

  it("reject settles through the api and activates nothing", async () => {
    mount();
    await flush();
    click(buttonByText("Review queue"));
    click(buttonByText("Reject"));
    await flush();
    expect(bridgeApi.rejectMemoryRecord).toHaveBeenCalledWith("r-prop");
    click(buttonByText("About me"));
    expect(container.textContent).not.toContain("Deploys only on Fridays");
  });

  it("speaks memory words, and auto-apply is visibly disabled until the bench exists", async () => {
    mount();
    await flush();
    click(buttonByText("Review queue"));
    expect(container.textContent).toContain("Remember");
    expect(container.textContent).toContain("Propose");
    const autoApply = buttonByText("Auto-apply");
    expect(autoApply.disabled).toBe(true);
    expect(autoApply.title).toContain("replay bench");
  });

  it("propose without a pinned profile is refused and the mode does not flip", async () => {
    const onError = vi.fn();
    mount({ onError });
    await flush();
    click(buttonByText("Review queue"));
    click(buttonByText("Propose"));
    await flush();
    expect(onError).toHaveBeenCalledWith(expect.stringContaining("pinned harness and model"));
    expect(buttonByText("Remember").getAttribute("aria-pressed")).toBe("true");
  });

  it("the last run's observed spend is on screen", async () => {
    extractionSettings = {
      scopeKey: "account:local",
      mode: "propose",
      harness: "codex",
      model: "gpt-5.6-luna",
      lastRun: { status: "completed", proposalCount: 2, observedTokens: 420, spendMicrousd: 1700, updatedAt: "2026-08-21T00:00:00Z" },
    };
    mount();
    await flush();
    click(buttonByText("Review queue"));
    expect(container.textContent).toContain("Last run completed");
    expect(container.textContent).toContain("$0.0017");
  });
});

describe("rememberAction", () => {
  it("saves at the cap and opens the dialog one character over it", () => {
    expect(rememberAction("z".repeat(4000))).toBe("save");
    expect(rememberAction("z".repeat(4001))).toBe("open-dialog");
  });
});

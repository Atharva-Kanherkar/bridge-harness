// @vitest-environment jsdom
// The dialog's contract (testing/feat-memory-ui.md): list, filter, save,
// forget under account:local; refetch on memory-changed behind a read
// generation; refuse over the cap instead of clipping; keep no state closed;
// and say plainly what this surface is not.
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { bridgeApi } from "../api";
import type { AdapterDescriptor, MemoryChangedPayload, MemoryExtractionSettings, MemoryModelSettings, MemoryRecord, MemoryRetentionSettings } from "../types";
import { bodyLength, MemoryDialog } from "./MemoryDialog";

const record = (id: string, body: string, kind = "preference"): MemoryRecord => ({
  id,
  scopeKey: "account:local",
  kind,
  body,
  provenance: "user_explicit",
  status: "active",
  validFrom: "2026-08-20T00:00:00Z",
  createdAt: "2026-08-20T00:00:00Z",
  updatedAt: "2026-08-20T00:00:00Z",
});

let container: HTMLDivElement;
let root: Root;
let store: MemoryRecord[];
let extractionSettings: MemoryExtractionSettings;
let memoryModel: MemoryModelSettings;
let memoryHandler: ((payload: MemoryChangedPayload) => void) | undefined;
let maxRecords: number;

const flush = async () => { await act(async () => {}); };

function mount(overrides: Partial<Parameters<typeof MemoryDialog>[0]> = {}) {
  act(() => {
    root.render(<MemoryDialog open onClose={() => {}} onError={() => {}} {...overrides} />);
  });
}

const textarea = () => document.querySelector<HTMLTextAreaElement>("textarea")!;
const buttonByText = (needle: string) =>
  [...document.querySelectorAll<HTMLButtonElement>("button")].find(button => button.textContent?.includes(needle))!;
const buttonNamed = (name: string) =>
  [...document.querySelectorAll<HTMLButtonElement>("button")].find(button => button.textContent?.trim() === name);
const tab = (name: string) =>
  [...document.querySelectorAll<HTMLButtonElement>('[aria-label="Memory views"] button')].find(button => button.textContent?.startsWith(name))!;
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
/** Opens the named list and picks the option with that label. */
const choose = async (label: string, option: string) => {
  await act(async () => { document.querySelector<HTMLButtonElement>(`button[aria-label="${label}"]`)!.click(); });
  const item = [...document.querySelectorAll<HTMLElement>('[role="option"]')].find(node => node.textContent?.trim() === option)!;
  await act(async () => {
    item.dispatchEvent(new MouseEvent("pointerdown", { bubbles: true }));
    item.click();
  });
};
const setSearch = (value: string) => {
  const input = document.querySelector<HTMLInputElement>('input[aria-label="Search memory"]')!;
  const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!;
  act(() => {
    setter.call(input, value);
    input.dispatchEvent(new Event("input", { bubbles: true }));
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
  maxRecords = 200;
  vi.spyOn(bridgeApi, "getConsolidationSettings").mockImplementation(async () => ({
    scopeKey: "account:local",
    mode: "off",
    maxRecords,
    heldRecords: store.filter(item => item.status === "active").length,
    allowRemoval: false,
    debounceSeconds: 600,
  }));
  vi.spyOn(bridgeApi, "updateExtractionSettings").mockImplementation(async mode => {
    extractionSettings = { ...extractionSettings, mode };
    return structuredClone(extractionSettings);
  });
  memoryModel = { resolved: { harness: "claude", model: "opus", source: "chat_default" }, skipped: [] };
  vi.spyOn(bridgeApi, "getMemoryModel").mockImplementation(async () => structuredClone(memoryModel));
  vi.spyOn(bridgeApi, "setMemoryModel").mockImplementation(async (harness, model) => {
    if (harness === "codex") throw new Error("codex cannot run memory jobs, which must be tool-free: no per-tool authority");
    memoryModel = harness && model
      ? { pinned: { harness, model }, resolved: { harness, model, source: "user" }, skipped: [] }
      : { resolved: { harness: "claude", model: "opus", source: "chat_default" }, skipped: [] };
    return structuredClone(memoryModel);
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
  const noDaily = Array<number>(14).fill(0);
  vi.spyOn(bridgeApi, "memoryRecallStats").mockResolvedValue({
    perRecord: [
      { id: "r-tabs", recalls: 3, lastRecalledDay: 12, inPacketRatio: 0.75, daily: [...noDaily] },
      { id: "r-tz", recalls: 0, lastRecalledDay: -1, inPacketRatio: 0, daily: [...noDaily] },
    ],
    injectionsPerDay: [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 2, 0, 4],
    budgetCharsUsed: 1234,
    budgetCharsMax: 4000,
    packets: 7,
    packetsWithMemories: 6,
    activeRecords: 2,
    recalledRecords: 1,
    byKind: [{ kind: "preference", active: 1, recalled: 1, recalls: 3 }, { kind: "fact", active: 1, recalled: 0, recalls: 0 }],
    exclusions: [{ code: "over_budget", count: 2 }],
  });
  vi.spyOn(bridgeApi, "memoryActivityLog").mockResolvedValue([
    { source: "consolidation", status: "completed", applied: 3, refused: 1, detail: "merged two notes", at: new Date().toISOString() },
    { source: "extraction", status: "failed", applied: 0, refused: 0, detail: "the harness ended before answering", at: new Date().toISOString() },
  ]);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.restoreAllMocks();
});

describe("MemoryDialog", () => {
  it("lists account pins and explains their scope", async () => {
    mount();
    await flush();
    expect(container.textContent).toContain("Prefers tabs over spaces");
    expect(container.textContent).toContain("Works in IST");
  });

  it("shows a loading skeleton, not an empty list, until pins arrive", async () => {
    const releases: Array<() => void> = [];
    vi.spyOn(bridgeApi, "listMemoryRecords").mockImplementation(
      () => new Promise(resolve => { releases.push(() => resolve({ scopeKey: "account:local", records: [] })); }),
    );
    mount();
    expect(document.querySelector('[data-testid="memory-loading"]')).not.toBeNull();
    expect(container.textContent).not.toContain("Nothing remembered yet");
    await act(async () => { releases.forEach(release => release()); });
    await flush();
    expect(document.querySelector('[data-testid="memory-loading"]')).toBeNull();
    expect(container.textContent).toContain("Nothing remembered yet");
  });

  it("fills the main canvas like Projects, not a floating overlay", async () => {
    mount();
    await flush();
    expect(container.querySelector('[role="dialog"]')).toBeNull();
    const page = container.firstElementChild as HTMLElement;
    expect(page.className).toContain("h-full");
    expect(page.className).toContain("overflow-y-auto");
    expect(page.className).not.toContain("fixed");
    expect(page.className).not.toContain("inset-0");
    expect(page.className).not.toContain("bg-scrim");
    const inner = page.firstElementChild as HTMLElement;
    expect(inner.className).toContain("max-w-page");
    expect(inner.className).not.toContain("max-w-2xl");
    expect(inner.className).not.toContain("rounded-3xl");
    expect(inner.className).not.toContain("u-overlay-strong");
    expect(container.querySelector("h1")?.textContent).toBe("Memory");
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
    click(buttonNamed("Facts")!);
    expect(container.textContent).toContain("Works in IST");
    expect(container.textContent).not.toContain("Prefers tabs over spaces");
    click(buttonNamed("Facts")!);
    expect(container.textContent).toContain("Prefers tabs over spaces");
  });

  it("search filters pins client-side and clearing restores the list", async () => {
    mount();
    await flush();
    setSearch("ist");
    expect(container.textContent).toContain("Works in IST");
    expect(container.textContent).not.toContain("Prefers tabs over spaces");
    setSearch("zzz-no-match");
    expect(container.textContent).toContain("No memories match your search.");
    setSearch("");
    expect(container.textContent).toContain("Prefers tabs over spaces");
  });

  it("shows a recall meta line only for pins that were recalled", async () => {
    mount();
    await flush();
    const rows = [...document.querySelectorAll("li")];
    const recalled = rows.find(row => row.textContent?.includes("Prefers tabs over spaces"))!;
    expect(recalled.textContent).toContain("recalled 3× · last 1d ago");
    const quiet = rows.find(row => row.textContent?.includes("Works in IST"))!;
    expect(quiet.textContent).not.toContain("recalled");
  });

  it("the Activity tab shows recall volume, memory use, the packet budget, and recent runs", async () => {
    mount();
    await flush();
    click(tab("Activity"));
    expect(container.textContent).toContain("peak 4 injections / day");
    expect(container.textContent).toContain("1234 / 4000 chars");
    expect(container.textContent).toContain("6 of 7 packets carried a memory");
    expect(container.textContent).toContain("1 of 2 active memories reached a prompt.");
    expect(container.textContent).toContain("50%");
    expect(container.textContent).toContain("1 of 1 used · 3 recalls");
    expect(container.textContent).toContain("Over the packet budget");
    expect(container.textContent).toContain("3 applied, 1 refused · merged two notes");
    expect(container.textContent).toContain("failed: The harness ended before answering.");
    expect(container.textContent).toContain("does not measure whether a memory helped");
    const meter = document.querySelector('[role="meter"][aria-label="Packet budget"]')!;
    expect(meter.getAttribute("aria-valuenow")).toBe("31");
  });

  it("the Insights tab only reads until the user asks for an analysis", async () => {
    const insights = vi.spyOn(bridgeApi, "memoryInsights").mockImplementation(async refresh => refresh
      ? {
        status: "ready", generatedAt: new Date().toISOString(), harness: "claude", model: "m",
        report: {
          headline: "Preferences carry the load", summary: "One of two memories reached a prompt.",
          highlights: [{ title: "Idle fact", detail: "Works in IST was never recalled.", tone: "watch" }],
          themes: [{ label: "Workflow", share: 1 }], recommendations: ["Retire the idle fact."],
          stats: { perRecord: [], injectionsPerDay: Array<number>(14).fill(0), budgetCharsUsed: 0, budgetCharsMax: 4000, packets: 7, packetsWithMemories: 6, activeRecords: 2, recalledRecords: 1, byKind: [], exclusions: [] },
          memoriesAnalysed: 2,
        },
      }
      : { status: "empty" });
    mount();
    await flush();
    click(tab("Insights"));
    await flush();
    expect(insights).toHaveBeenCalledTimes(1);
    expect(insights).toHaveBeenLastCalledWith(false);
    expect(container.textContent).toContain("Nothing analysed yet");
    click(buttonNamed("Analyse my memory")!);
    await flush();
    expect(insights).toHaveBeenLastCalledWith(true);
    expect(container.textContent).toContain("Preferences carry the load");
    expect(container.textContent).toContain("Retire the idle fact.");
    expect(container.textContent).toContain("50%");
    expect(container.textContent).toContain("6 of 7");
  });

  it("saving goes through the api and the list refreshes on the hint", async () => {
    mount();
    await flush();
    setBody("Always run bun run check first");
    click(buttonNamed("Save")!);
    await flush();
    expect(bridgeApi.saveMemoryRecord).toHaveBeenCalledWith("Always run bun run check first", "preference", undefined);
    expect(container.textContent).toContain("Always run bun run check first");
    expect(textarea().value).toBe("");
  });

  it("accepts a body the ledger would accept, whatever JS length says", async () => {
    mount();
    await flush();
    setBody("😀".repeat(2001));
    expect(container.textContent).toContain("2001 / 4000");
    expect(buttonNamed("Save")!.disabled).toBe(false);
  });

  it("refuses over the cap with a visible count instead of clipping", async () => {
    mount();
    await flush();
    setBody("x".repeat(4001));
    expect(container.textContent).toContain("4001 / 4000");
    expect(buttonNamed("Save")!.disabled).toBe(true);
    click(buttonNamed("Save")!);
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

  it("a slow failing read cannot toast over a newer one", async () => {
    let failSlow!: (error: Error) => void;
    const slow = new Promise<never>((_, reject) => { failSlow = reject; });
    vi.spyOn(bridgeApi, "listMemoryRecords")
      .mockImplementationOnce(() => slow)
      .mockImplementation(async () => ({ scopeKey: "account:local", records: [record("r-fresh", "Fresh view")] }));
    const onError = vi.fn();
    mount({ onError });
    await flush();
    act(() => { memoryHandler?.({ scopeKey: "account:local" }); });
    await flush();
    expect(container.textContent).toContain("Fresh view");
    failSlow(new Error("the stale read failed"));
    await flush();
    expect(onError).not.toHaveBeenCalled();
    expect(container.textContent).toContain("Fresh view");
  });

  it("opens pre-filled from an oversize remember and issues no save", async () => {
    mount({ initialBody: "y".repeat(4001) });
    await flush();
    expect(textarea().value).toBe("y".repeat(4001));
    expect(buttonNamed("Save")!.disabled).toBe(true);
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
    expect(buttonNamed("Update")).toBeDefined();
    setBody("Prefers spaces after all");
    click(buttonNamed("Update")!);
    await flush();
    expect(bridgeApi.supersedeMemoryRecord).toHaveBeenCalledWith("r-tabs", "Prefers spaces after all", "preference");
    expect(bridgeApi.saveMemoryRecord).not.toHaveBeenCalled();
    expect(container.textContent).toContain("Prefers spaces after all");
    expect(container.textContent).not.toContain("Prefers tabs over spaces");
    expect(container.textContent).toContain("replaced an earlier memory");
  });

  it("cancelling an edit restores the plain composer and writes nothing", async () => {
    mount();
    await flush();
    click([...document.querySelectorAll('[aria-label="Edit"]')][0]);
    click(buttonByText("Cancel"));
    expect(textarea().value).toBe("");
    expect(buttonNamed("Update")).toBeUndefined();
    expect(bridgeApi.supersedeMemoryRecord).not.toHaveBeenCalled();
    expect(bridgeApi.saveMemoryRecord).not.toHaveBeenCalled();
  });

  it("an extracted memory is labeled honestly; explicit pins are not", async () => {
    store.push({
      ...record("r-approved", "Ships behind a flag", "decision"),
      provenance: "model_proposal",
      confidenceBps: 7600,
    });
    mount();
    await flush();
    expect(container.textContent).toContain("extracted");
    expect(container.textContent).toContain("76% confident");
    const explicitRow = [...container.querySelectorAll("li")].find(item => item.textContent?.includes("Works in IST"))!;
    expect(explicitRow.textContent).not.toContain("extracted");
    expect(explicitRow.textContent).not.toContain("% confident");
  });
});

describe("MemoryDialog injection toggle", () => {
  it("reads the flag from the api and writes the change through it", async () => {
    let enabled = true;
    vi.spyOn(bridgeApi, "getMemoryInjection").mockImplementation(async () => ({ scopeKey: "account:local", enabled }));
    vi.spyOn(bridgeApi, "setMemoryInjection").mockImplementation(async next => {
      enabled = next;
      return { scopeKey: "account:local", enabled };
    });
    mount();
    await flush();
    const toggle = document.querySelector<HTMLButtonElement>('[role="switch"][aria-label="Use active memory in new chats"]')!;
    expect(toggle.getAttribute("aria-checked")).toBe("true");
    expect(container.textContent).toContain("Use in new chats");
    act(() => { toggle.click(); });
    await flush();
    expect(bridgeApi.setMemoryInjection).toHaveBeenCalledWith(false);
    expect(toggle.getAttribute("aria-checked")).toBe("false");
  });
});

describe("MemoryDialog review queue", () => {
  it("lists proposals with confidence and rationale, and pins carry no fake confidence", async () => {
    mount();
    await flush();
    click(tab("Review"));
    expect(container.textContent).toContain("Deploys only on Fridays");
    expect(container.textContent).toContain("82% confident");
    expect(container.textContent).toContain("Said twice in one chat");
    click(tab("Memories"));
    expect(container.textContent).not.toContain("% confident");
  });

  it("approve activates through the api and the row leaves the queue", async () => {
    mount();
    await flush();
    click(tab("Review"));
    click(buttonByText("Approve"));
    await flush();
    expect(bridgeApi.approveMemoryRecord).toHaveBeenCalledWith("r-prop");
    expect(container.textContent).toContain("Nothing to review");
    click(tab("Memories"));
    expect(container.textContent).toContain("Deploys only on Fridays");
  });

  it("reject settles through the api and activates nothing", async () => {
    mount();
    await flush();
    click(tab("Review"));
    click(buttonByText("Reject"));
    await flush();
    expect(bridgeApi.rejectMemoryRecord).toHaveBeenCalledWith("r-prop");
    click(tab("Memories"));
    expect(container.textContent).not.toContain("Deploys only on Fridays");
  });

  it("offers explicit manual, review-first, and automatic modes with the safeguards explained", async () => {
    mount();
    await flush();
    click(tab("Review"));
    expect(container.textContent).toContain("Manual only");
    expect(container.textContent).toContain("Review first");
    expect(container.textContent).toContain("Automatic");
    expect(container.textContent).toContain("90%+");
    expect(container.textContent).toContain("other validated candidates that fit the budget stay here for review");
    expect(container.textContent).toContain("Invalid, unsafe, duplicate, or over-budget output is refused");
    expect(container.textContent).toContain("existing active memory stays active");
  });

  const memoryAdapters: AdapterDescriptor[] = [
    {
      id: "claude", label: "Claude", available: true, authState: "signed_in", version: "test", unavailableReason: null, capabilities: [],
      models: [{ id: "opus", label: "Claude Opus", tier: "strong", defaultForTier: true }, { id: "haiku", label: "Claude Haiku", tier: "fast", defaultForTier: true }],
      defaultModel: "opus",
    },
    {
      id: "codex", label: "Codex", available: true, authState: "signed_in", version: "test", unavailableReason: null, capabilities: [],
      models: [{ id: "gpt-5.6-luna", label: "GPT Luna", tier: "standard", defaultForTier: true }],
      defaultModel: "gpt-5.6-luna",
    },
  ];
  // Option labels for the ids these tests pick, as the Choice lists show them.
  const optionLabel: Record<string, string> = { "": "", claude: "Claude", codex: "Codex", opus: "Claude Opus", haiku: "Claude Haiku", "gpt-5.6-luna": "GPT Luna" };
  const pick = (label: string, value: string) =>
    choose(label, value === "" ? (label === "Memory harness" ? "Use my new-chat model" : "Model…") : optionLabel[value]);
  /** What a Choice trigger shows. */
  const pickerValue = (label: string) => document.querySelector<HTMLButtonElement>(`button[aria-label="${label}"]`)!.textContent?.trim();
  const sourceLine = () => container.querySelector('[data-testid="memory-model-source"]')?.textContent;

  it("changing the extraction mode sends only the mode", async () => {
    const onError = vi.fn();
    mount({ onError });
    await flush();
    click(tab("Review"));
    click(buttonByText("Automatic"));
    await flush();
    expect(onError).not.toHaveBeenCalled();
    expect(bridgeApi.updateExtractionSettings).toHaveBeenCalledWith("auto_apply");
    expect(buttonByText("Automatic").getAttribute("aria-pressed")).toBe("true");
  });

  it("the memory model card says what every memory job runs on and why", async () => {
    mount({ adapters: memoryAdapters });
    await flush();
    click(tab("Review"));
    expect(container.textContent).toContain("Memory model");
    expect(container.textContent).toContain("Runs extraction after chats, nightly cleanup and automatic approval");
    expect(container.textContent).not.toContain("Extraction model");
    expect(sourceLine()).toBe("Using Claude Opus, your new-chat model.");
    expect(pickerValue("Memory harness")).toBe("Use my new-chat model");
  });

  it("a skipped choice is shown with its reason", async () => {
    memoryModel = {
      resolved: { harness: "claude", model: "opus", source: "standard" },
      skipped: [{ harness: "codex", model: "gpt-5.6-luna", source: "chat_default", reason: "the app-server protocol has no per-tool authority" }],
    };
    mount({ adapters: memoryAdapters });
    await flush();
    click(tab("Review"));
    expect(sourceLine()).toBe("Using Claude Opus, Bridge's standard choice.");
    expect(container.textContent).toContain("Skipped GPT Luna, your new-chat model: the app-server protocol has no per-tool authority.");
  });

  it("picking a model saves the pin once, and the new-chat option clears it", async () => {
    const onError = vi.fn();
    mount({ onError, adapters: memoryAdapters });
    await flush();
    click(tab("Review"));
    await pick("Memory harness", "claude");
    await flush();
    expect(bridgeApi.setMemoryModel).not.toHaveBeenCalled();
    await pick("Memory model", "haiku");
    await flush();
    expect(bridgeApi.setMemoryModel).toHaveBeenCalledTimes(1);
    expect(bridgeApi.setMemoryModel).toHaveBeenCalledWith("claude", "haiku");
    expect(sourceLine()).toBe("Using Claude Haiku, your pick.");

    await pick("Memory harness", "");
    await flush();
    expect(bridgeApi.setMemoryModel).toHaveBeenLastCalledWith();
    expect(sourceLine()).toBe("Using Claude Opus, your new-chat model.");
    expect(onError).not.toHaveBeenCalled();
  });

  it("a harness that cannot run tool-free is refused and the pickers reset", async () => {
    const onError = vi.fn();
    mount({ onError, adapters: memoryAdapters });
    await flush();
    click(tab("Review"));
    await pick("Memory harness", "codex");
    await pick("Memory model", "gpt-5.6-luna");
    await flush();
    expect(onError).toHaveBeenCalledWith(expect.stringContaining("tool-free"));
    expect(pickerValue("Memory harness")).toBe("Use my new-chat model");
    expect(sourceLine()).toBe("Using Claude Opus, your new-chat model.");
  });

  it("the pickers follow a pin changed or cleared elsewhere", async () => {
    memoryModel = { pinned: { harness: "claude", model: "haiku" }, resolved: { harness: "claude", model: "haiku", source: "user" }, skipped: [] };
    mount({ adapters: memoryAdapters });
    await flush();
    click(tab("Review"));
    expect([pickerValue("Memory harness"), pickerValue("Memory model")]).toEqual(["Claude", "Claude Haiku"]);

    // Changed through the settings tool in another chat.
    memoryModel = { pinned: { harness: "claude", model: "opus" }, resolved: { harness: "claude", model: "opus", source: "user" }, skipped: [] };
    await act(async () => { memoryHandler?.({ scopeKey: "account:local" }); });
    await flush();
    expect(sourceLine()).toBe("Using Claude Opus, your pick.");
    expect([pickerValue("Memory harness"), pickerValue("Memory model")]).toEqual(["Claude", "Claude Opus"]);

    // Cleared elsewhere: back to the new-chat model.
    memoryModel = { resolved: { harness: "claude", model: "opus", source: "chat_default" }, skipped: [] };
    await act(async () => { memoryHandler?.({ scopeKey: "account:local" }); });
    await flush();
    expect([pickerValue("Memory harness"), pickerValue("Memory model")]).toEqual(["Use my new-chat model", "Model…"]);
  });

  it("a harness picked without a model stays a local draft until a model is chosen", async () => {
    memoryModel = { pinned: { harness: "claude", model: "haiku" }, resolved: { harness: "claude", model: "haiku", source: "user" }, skipped: [] };
    mount({ adapters: memoryAdapters });
    await flush();
    click(tab("Review"));
    await pick("Memory harness", "codex");
    await flush();
    expect([pickerValue("Memory harness"), pickerValue("Memory model")]).toEqual(["Codex", "Model…"]);
    await act(async () => { memoryHandler?.({ scopeKey: "account:local" }); });
    await flush();
    expect(pickerValue("Memory harness"), "an unrelated reload keeps the draft").toBe("Codex");
    expect(sourceLine(), "the stored pin is still what runs").toBe("Using Claude Haiku, your pick.");
    expect(bridgeApi.setMemoryModel).not.toHaveBeenCalled();
  });

  it("a read that started before a memory-model write cannot undo it", async () => {
    let release: () => void = () => {};
    const held = new Promise<void>(resolve => { release = resolve; });
    const list = vi.mocked(bridgeApi.listMemoryRecords).getMockImplementation()!;
    let first = true;
    vi.mocked(bridgeApi.listMemoryRecords).mockImplementation(async (...args) => {
      if (first) { first = false; await held; }
      return list(...args);
    });
    mount({ adapters: memoryAdapters });
    await flush();
    click(tab("Review"));
    await pick("Memory harness", "claude");
    await pick("Memory model", "haiku");
    await flush();
    expect(sourceLine()).toBe("Using Claude Haiku, your pick.");

    // The first read captured the unpinned model before the write.
    await act(async () => { release(); });
    await flush();
    expect(sourceLine()).toBe("Using Claude Haiku, your pick.");
    expect([pickerValue("Memory harness"), pickerValue("Memory model")]).toEqual(["Claude", "Claude Haiku"]);

    await pick("Memory harness", "");
    await flush();
    expect(bridgeApi.setMemoryModel).toHaveBeenLastCalledWith();
    expect(memoryModel.pinned, "the clear reached the store").toBeUndefined();
  });

  it("the last run's observed spend is on screen", async () => {
    extractionSettings = {
      scopeKey: "account:local",
      mode: "propose",
      lastRun: { status: "completed", proposalCount: 2, observedTokens: 420, spendMicrousd: 1700, updatedAt: "2026-08-21T00:00:00Z" },
    };
    mount();
    await flush();
    click(tab("Review"));
    expect(container.textContent).toContain("Last run completed");
    expect(container.textContent).toContain("2 extracted");
    expect(container.textContent).toContain("$0.0017");
    expect(container.textContent).not.toContain("already known");
  });

  it("the last run says how many candidates were already known", async () => {
    extractionSettings = {
      scopeKey: "account:local",
      mode: "propose",
      lastRun: { status: "completed", proposalCount: 1, knownCount: 3, observedTokens: 420, spendMicrousd: 1700, updatedAt: "2026-08-21T00:00:00Z" },
    };
    mount();
    await flush();
    click(tab("Review"));
    expect(container.textContent).toContain("1 extracted · 3 already known");
    expect(container.textContent).toContain("When a chat goes quiet");
    expect(container.textContent).not.toContain("After a chat turn");
  });

  it("a proposal that updates a memory names what it replaces", async () => {
    store.push({ ...record("r-update", "Prefers spaces over tabs", "preference"), status: "proposed", provenance: "model_proposal", supersedes: "r-tabs" });
    store.push({ ...record("r-orphan", "Lives in Berlin", "fact"), status: "proposed", provenance: "model_proposal", supersedes: "r-gone" });
    mount();
    await flush();
    click(tab("Review"));
    expect(container.textContent).toContain("Replaces: Prefers tabs over spaces");
    expect(container.textContent).toContain("Replaces an earlier version");
  });
});

describe("bodyLength", () => {
  it("measures what the ledger measures: trimmed, in code points", () => {
    // The ledger trims and counts chars; JS .length counts UTF-16 units, so
    // an emoji scored two and trailing whitespace scored at all.
    expect(bodyLength("😀".repeat(2001))).toBe(2001);
    expect(bodyLength(`${"x".repeat(4000)}   \n`)).toBe(4000);
  });
});

describe("memory hardening", () => {
  it("says the scope is full with active memories only, and that proposals still wait", async () => {
    maxRecords = 2;
    mount();
    await flush();
    const banner = container.querySelector('[role="status"]');
    expect(banner?.textContent).toContain("Memory is full.");
    expect(banner?.textContent).toContain("2 of 2 memories");
    expect(banner?.textContent).toContain("Proposals still wait for you");
    expect(banner?.textContent).not.toContain("counting proposals");
  });

  it("shows no banner while the scope has room", async () => {
    mount();
    await flush();
    expect(container.textContent).not.toContain("Memory is full.");
  });

  it("opens a reply as a draft to cut down and never saves it on its own", async () => {
    mount({ initialBody: "Here's what I did. CI is green. Also, never push to main." });
    await flush();
    expect(textarea().value).toContain("Here's what I did.");
    expect(container.textContent).toContain("Cut it down to the one rule or fact worth keeping");
    expect(bridgeApi.saveMemoryRecord).not.toHaveBeenCalled();
    setBody("Never push to main.");
    click(buttonNamed("Save")!);
    await flush();
    expect(bridgeApi.saveMemoryRecord).toHaveBeenCalledWith("Never push to main.", "preference", undefined);
  });

  it("labels what an agent proposed, in the queue and once accepted", async () => {
    store.push(
      { ...record("r-agent-prop", "Check upstream trackers before a hunt", "constraint"), status: "proposed", provenance: "agent_proposal", rationale: "the person said so" },
      { ...record("r-agent", "Keeps the main checkout on main", "fact"), provenance: "agent_proposal" },
    );
    mount();
    await flush();
    expect(container.textContent).toContain("from an agent");
    click(tab("Review"));
    expect(container.textContent).toContain("proposed by an agent");
    expect(container.textContent).toContain("expire");
  });
});

// testing/feat-memory-paging.md: lists page, badges show the true total, and
// proposals are decided in bulk with one call.
describe("MemoryDialog paging and bulk review", () => {
  // Each proposal is a minute older than the one before, so core's order
  // (newest change first) is index order.
  const minutesAgo = (minutes: number) => new Date(Date.UTC(2026, 10, 20) - minutes * 60_000).toISOString();
  const proposal = (index: number, updatedAt = minutesAgo(index)): MemoryRecord => ({
    ...record(`q-${String(index).padStart(3, "0")}`, `Proposal number ${index}`, "fact"),
    status: "proposed",
    provenance: "model_proposal",
    updatedAt,
  });
  const pageCursorsOf: string[] = [];
  const pagedList = () => vi.spyOn(bridgeApi, "listMemoryRecords").mockImplementation(async (scopeKey, status, page = {}) => {
    if (page.cursor) pageCursorsOf.push(page.cursor);
    // Core's order: newest change first, ties by id, both descending.
    const matching = store
      .filter(item => item.status === (status ?? "active"))
      .sort((a, b) => b.updatedAt.localeCompare(a.updatedAt) || b.id.localeCompare(a.id));
    const start = page.cursor ? Number(page.cursor) : 0;
    const end = start + 50;
    return {
      scopeKey,
      records: matching.slice(start, end),
      total: matching.length,
      ...(end < matching.length ? { nextCursor: String(end) } : {}),
    };
  });
  const cards = () => container.querySelectorAll('[role="checkbox"][aria-label^="Select proposal"]');
  const toggle = (element: Element) => {
    act(() => { (element as HTMLElement).click(); });
  };

  beforeEach(() => {
    pageCursorsOf.length = 0;
    store = store.filter(item => item.status === "active");
    pagedList();
    vi.spyOn(bridgeApi, "reviewMemoryRecords").mockImplementation(async (approve, reject) => {
      for (const id of approve) store.find(item => item.id === id)!.status = "active";
      for (const id of reject) store.find(item => item.id === id)!.status = "rejected";
      memoryHandler?.({ scopeKey: "account:local" });
      return { scopeKey: "account:local", approved: approve, rejected: reject, refused: [] };
    });
  });

  it("badges the true queue size and pages proposals with Show more", async () => {
    store.push(...Array.from({ length: 138 }, (_, index) => proposal(index)));
    mount();
    await flush();
    expect(tab("Review").textContent).toContain("138");
    click(tab("Review"));
    expect(cards()).toHaveLength(50);
    expect(container.textContent).toContain("50 of 138");
    click(buttonByText("Show 50 more"));
    await flush();
    expect(cards()).toHaveLength(100);
    click(buttonByText("Show 38 more"));
    await flush();
    expect(cards()).toHaveLength(138);
    expect(buttonByText("more")).toBeUndefined();
    expect(new Set([...cards()].map(input => input.getAttribute("aria-label"))).size).toBe(138);
  });

  it("approves the selected proposals in one call and clears the selection", async () => {
    store.push(proposal(1), proposal(2), proposal(3));
    mount();
    await flush();
    click(tab("Review"));
    toggle(cards()[0]);
    toggle(cards()[2]);
    expect(container.textContent).toContain("2 selected");
    click(buttonByText("Approve selected"));
    await flush();
    expect(bridgeApi.reviewMemoryRecords).toHaveBeenCalledTimes(1);
    expect(bridgeApi.reviewMemoryRecords).toHaveBeenCalledWith(["q-001", "q-003"], []);
    expect(bridgeApi.approveMemoryRecord).not.toHaveBeenCalled();
    expect(cards()).toHaveLength(1);
    expect(container.textContent).not.toContain("selected");
  });

  it("selects every shown proposal, not the unloaded ones, and rejects them together", async () => {
    store.push(...Array.from({ length: 60 }, (_, index) => proposal(index)));
    mount();
    await flush();
    click(tab("Review"));
    toggle(document.querySelector('[role="checkbox"][aria-label="Select all shown proposals"]')!);
    expect(container.textContent).toContain("50 selected");
    click(buttonByText("Reject selected"));
    await flush();
    const [approve, reject] = vi.mocked(bridgeApi.reviewMemoryRecords).mock.calls[0];
    expect(approve).toEqual([]);
    expect(reject).toHaveLength(50);
    expect(tab("Review").textContent).toContain("10");
  });

  it("reports proposals the batch refused", async () => {
    store.push(proposal(1), proposal(2));
    vi.mocked(bridgeApi.reviewMemoryRecords).mockResolvedValueOnce({
      scopeKey: "account:local",
      approved: ["q-001"],
      rejected: [],
      refused: [{ id: "q-002", reason: "This record is already rejected, not waiting for review." }],
    });
    const onError = vi.fn();
    mount({ onError });
    await flush();
    click(tab("Review"));
    toggle(document.querySelector('[role="checkbox"][aria-label="Select all shown proposals"]')!);
    click(buttonByText("Approve selected"));
    await flush();
    expect(onError).toHaveBeenCalledWith(expect.stringContaining("1 of 2 could not be approved"));
  });

  it("keeps the pages it showed when memory changes", async () => {
    store.push(...Array.from({ length: 120 }, (_, index) => proposal(index)));
    mount();
    await flush();
    click(tab("Review"));
    click(buttonByText("Show 50 more"));
    await flush();
    expect(cards()).toHaveLength(100);
    act(() => memoryHandler?.({ scopeKey: "account:local" }));
    await flush();
    expect(cards()).toHaveLength(100);
  });

  it("keeps a ticked proposal through a reload when newer proposals arrive", async () => {
    store.push(...Array.from({ length: 100 }, (_, index) => proposal(index)));
    mount();
    await flush();
    click(tab("Review"));
    click(buttonByText("Show 50 more"));
    await flush();
    expect(cards()).toHaveLength(100);
    const last = cards()[99];
    const ticked = last.getAttribute("aria-label");
    toggle(last);
    expect(container.textContent).toContain("1 selected");
    // An extraction run lands five newer proposals, which sort to the top.
    store.push(...Array.from({ length: 5 }, (_, index) => proposal(500 + index, minutesAgo(-1 - index))));
    act(() => memoryHandler?.({ scopeKey: "account:local" }));
    await flush();
    await flush();
    const labels = [...cards()].map(input => input.getAttribute("aria-label"));
    expect(labels).toContain(ticked);
    expect(cards()).toHaveLength(105);
    expect(container.textContent).toContain("1 selected");
    const kept = [...cards()].find(input => input.getAttribute("aria-label") === ticked)!;
    expect(kept.getAttribute("aria-checked")).toBe("true");
  });

  it("reads the rest of the memories before a search, so nothing unloaded is missed", async () => {
    store = Array.from({ length: 70 }, (_, index) => ({ ...record(`a-${index}`, `Ordinary memory ${index}`), updatedAt: minutesAgo(index) }));
    store.push({ ...record("a-late", "Deploys from a laptop only"), updatedAt: minutesAgo(500) });
    mount();
    await flush();
    expect(container.textContent).not.toContain("Deploys from a laptop only");
    expect(buttonByText("Show 21 more")).toBeDefined();
    setSearch("laptop");
    await flush();
    await flush();
    expect(pageCursorsOf).toContain("50");
    expect(container.textContent).toContain("Deploys from a laptop only");
  });
});

// testing/feat-memory-retention.md: clean-up settings, the Retired list with
// Restore, and retention sweeps in Activity.
describe("MemoryDialog retention", () => {
  let retention: MemoryRetentionSettings;
  beforeEach(() => {
    retention = { scopeKey: "account:local", proposalTtlDays: 7, retireUnusedAfterDays: 90, maxProposals: 100, maxActive: 200, heldActive: 2, heldProposals: 1 };
    vi.spyOn(bridgeApi, "getRetentionSettings").mockImplementation(async () => structuredClone(retention));
    vi.spyOn(bridgeApi, "setRetentionSettings").mockImplementation(async (proposalTtlDays, retireUnusedAfterDays) => {
      retention = { ...retention, proposalTtlDays, retireUnusedAfterDays: retireUnusedAfterDays ?? undefined };
      return structuredClone(retention);
    });
    vi.spyOn(bridgeApi, "restoreMemoryRecord").mockImplementation(async recordId => {
      const old = store.find(item => item.id === recordId)!;
      const restored = { ...old, id: `${recordId}-back`, status: "active", supersedes: recordId };
      old.status = "superseded";
      store = [restored, ...store];
      memoryHandler?.({ scopeKey: "account:local" });
      return restored;
    });
  });

  it("explains the queue's own expiry and allowance", async () => {
    mount();
    await flush();
    click(tab("Review"));
    expect(container.textContent).toContain("within 7 days expire");
    expect(container.textContent).toContain("The queue holds 1 of 100");
  });

  it("saves a shorter proposal expiry and never retiring from the Clean up card", async () => {
    mount();
    await flush();
    click(tab("Review"));
    await choose("Proposals expire after", "Expire after 3 days");
    expect(bridgeApi.setRetentionSettings).toHaveBeenLastCalledWith(3, 90);
    await choose("Retire unused memories after", "Never retire");
    expect(bridgeApi.setRetentionSettings).toHaveBeenLastCalledWith(3, null);
    expect(container.textContent).toContain("within 3 days expire");
  });

  it("lists retired memories and restores one", async () => {
    store.push({ ...record("r-old", "Keeps staging on Postgres 15", "fact"), status: "expired", provenance: "model_proposal", validTo: "2026-08-20T00:00:00Z" });
    mount();
    await flush();
    const retired = container.querySelector('[aria-label="Retired memories"]')!;
    expect(retired.textContent).toContain("Keeps staging on Postgres 15");
    click([...retired.querySelectorAll("button")].find(button => button.textContent === "Restore")!);
    await flush();
    expect(bridgeApi.restoreMemoryRecord).toHaveBeenCalledWith("r-old");
    expect(container.querySelector('[aria-label="Retired memories"]')).toBeNull();
  });

  it("shows retention sweeps in Activity", async () => {
    vi.mocked(bridgeApi.memoryActivityLog).mockResolvedValue([
      { source: "retention", status: "completed", applied: 3, refused: 0, detail: "unused_retired", at: new Date().toISOString() },
      { source: "retention", status: "completed", applied: 12, refused: 0, detail: "proposals_expired", at: new Date().toISOString() },
    ]);
    mount();
    await flush();
    click(tab("Activity"));
    expect(container.textContent).toContain("Retired 3 unused memories");
    expect(container.textContent).toContain("12 unreviewed proposals expired");
  });

  it("shows already-known counts and nothing-new runs in Activity", async () => {
    vi.mocked(bridgeApi.memoryActivityLog).mockResolvedValue([
      { source: "extraction", status: "completed", applied: 2, refused: 0, known: 3, at: new Date().toISOString() },
      { source: "extraction", status: "cancelled", applied: 0, refused: 0, detail: "nothing_new", at: new Date().toISOString() },
    ]);
    mount();
    await flush();
    click(tab("Activity"));
    expect(container.textContent).toContain("2 proposals written, 3 already known");
    expect(container.textContent).toContain("Nothing new to read");
    expect(container.textContent).not.toContain("nothing_new");
  });
});

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
import { bodyLength, MemoryDialog, rememberAction } from "./MemoryDialog";

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
  vi.spyOn(bridgeApi, "updateExtractionSettings").mockImplementation(async (mode, harness, model) => {
    if (Boolean(harness) !== Boolean(model)) throw new Error("Pin both a helper and a model, or neither to run on each chat's own model.");
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
  const noDaily = Array<number>(14).fill(0);
  vi.spyOn(bridgeApi, "memoryRecallStats").mockResolvedValue({
    perRecord: [
      { id: "r-tabs", recalls: 3, lastRecalledDay: 12, inPacketRatio: 0.75, daily: [...noDaily] },
      { id: "r-tz", recalls: 0, lastRecalledDay: -1, inPacketRatio: 0, daily: [...noDaily] },
    ],
    injectionsPerDay: [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 2, 0, 4],
    budgetCharsUsed: 1234,
    budgetCharsMax: 4000,
  });
  vi.spyOn(bridgeApi, "memoryConsolidationLog").mockResolvedValue([
    { op: "merge", detail: "2 worktree notes folded into one", day: 12 },
    { op: "retire", detail: "r-old tombstoned", day: 2 },
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
    expect(container.textContent).toContain("remembers across your conversations");
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
    click(buttonByText("fact"));
    expect(container.textContent).toContain("Works in IST");
    expect(container.textContent).not.toContain("Prefers tabs over spaces");
    click(buttonByText("fact"));
    expect(container.textContent).toContain("Prefers tabs over spaces");
  });

  it("search filters pins client-side and clearing restores the list", async () => {
    mount();
    await flush();
    setSearch("ist");
    expect(container.textContent).toContain("Works in IST");
    expect(container.textContent).not.toContain("Prefers tabs over spaces");
    setSearch("zzz-no-match");
    expect(container.textContent).toContain("No pins match your search.");
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

  it("the Activity tab shows recall volume, the packet budget, and the consolidation log", async () => {
    mount();
    await flush();
    click(buttonByText("Activity"));
    expect(container.textContent).toContain("peak 4 injections / day");
    expect(container.textContent).toContain("1234 / 4000 chars");
    expect(container.textContent).toContain("Consolidation log");
    expect(container.textContent).toContain("2 worktree notes folded into one");
    expect(container.textContent).toContain("merge");
    expect(container.textContent).toContain("11d ago");
    const meter = document.querySelector('[role="meter"][aria-label="Packet budget"]')!;
    expect(meter.getAttribute("aria-valuenow")).toBe("31");
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

  it("accepts a body the ledger would accept, whatever JS length says", async () => {
    mount();
    await flush();
    setBody("😀".repeat(2001));
    expect(container.textContent).toContain("2001 / 4000");
    expect(buttonByText("Save pin").disabled).toBe(false);
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
    const toggle = document.querySelector<HTMLInputElement>('[aria-label="Use active memory in new chats"]')!;
    expect(toggle.checked).toBe(true);
    expect(container.textContent).toContain("Use active memory when starting a new conversation.");
    act(() => { toggle.click(); });
    await flush();
    expect(bridgeApi.setMemoryInjection).toHaveBeenCalledWith(false);
    expect(toggle.checked).toBe(false);
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

  it("offers explicit manual, review-first, and automatic modes with the safeguards explained", async () => {
    mount();
    await flush();
    click(buttonByText("Review queue"));
    expect(container.textContent).toContain("Manual only");
    expect(container.textContent).toContain("Review first");
    expect(container.textContent).toContain("Automatic");
    expect(container.textContent).toContain("90%+");
    expect(container.textContent).toContain("other validated candidates that fit the budget stay here for review");
    expect(container.textContent).toContain("Invalid, unsafe, duplicate, or over-budget output is refused");
    expect(container.textContent).toContain("existing active memory stays active");
  });

  it("review first without a pinned helper runs on the chat's own model", async () => {
    const onError = vi.fn();
    mount({ onError });
    await flush();
    click(buttonByText("Review queue"));
    expect(buttonByText("Manual only").getAttribute("aria-pressed")).toBe("true");
    click(buttonByText("Review first"));
    await flush();
    expect(onError).not.toHaveBeenCalled();
    expect(buttonByText("Review first").getAttribute("aria-pressed")).toBe("true");
    expect(extractionSettings.harness).toBeFalsy();
    expect(extractionSettings.model).toBeFalsy();
    expect(container.textContent).toContain("Chat's own model");
  });

  it("automatic is selectable and keeps the chat-own-model profile when unpinned", async () => {
    const onError = vi.fn();
    mount({ onError });
    await flush();
    click(buttonByText("Review queue"));
    click(buttonByText("Automatic"));
    await flush();
    expect(onError).not.toHaveBeenCalled();
    expect(bridgeApi.updateExtractionSettings).toHaveBeenCalledWith("auto_apply", "", "");
    expect(buttonByText("Automatic").getAttribute("aria-pressed")).toBe("true");
  });

  it("automatic forwards the selected extraction helper and model", async () => {
    mount({
      adapters: [{
        id: "claude",
        label: "Claude",
        available: true,
        authState: "signed_in",
        version: "test",
        unavailableReason: null,
        capabilities: [],
        models: [{ id: "sonnet", label: "Claude Sonnet", tier: "standard", defaultForTier: true }],
        defaultModel: "sonnet",
      }],
    });
    await flush();
    click(buttonByText("Review queue"));
    const harness = container.querySelector<HTMLSelectElement>('select[aria-label="Extraction harness"]')!;
    act(() => {
      const setter = Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype, "value")!.set!;
      setter.call(harness, "claude");
      harness.dispatchEvent(new Event("change", { bubbles: true }));
    });
    const model = container.querySelector<HTMLSelectElement>('select[aria-label="Extraction model"]')!;
    act(() => {
      const setter = Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype, "value")!.set!;
      setter.call(model, "sonnet");
      model.dispatchEvent(new Event("change", { bubbles: true }));
    });
    click(buttonByText("Automatic"));
    await flush();
    expect(bridgeApi.updateExtractionSettings).toHaveBeenCalledWith("auto_apply", "claude", "sonnet");
    expect(buttonByText("Automatic").getAttribute("aria-pressed")).toBe("true");
  });

  it("a helper pinned without a model is refused and the mode stays put", async () => {
    const onError = vi.fn();
    mount({
      onError,
      adapters: [{ id: "claude", label: "Claude", available: true, authState: "authenticated", capabilities: [], models: [] } as never],
    });
    await flush();
    click(buttonByText("Review queue"));
    const harness = container.querySelector<HTMLSelectElement>('select[aria-label="Extraction harness"]')!;
    await act(async () => {
      const setter = Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype, "value")!.set!;
      setter.call(harness, "claude");
      harness.dispatchEvent(new Event("change", { bubbles: true }));
    });
    click(buttonByText("Review first"));
    await flush();
    expect(onError).toHaveBeenCalledWith(expect.stringContaining("both a helper and a model"));
    expect(buttonByText("Manual only").getAttribute("aria-pressed")).toBe("true");
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
    expect(container.textContent).toContain("2 extracted");
    expect(container.textContent).toContain("$0.0017");
  });
});

describe("bodyLength", () => {
  it("measures what the ledger measures: trimmed, in code points", () => {
    // The ledger trims and counts chars; JS .length counts UTF-16 units, so
    // an emoji scored two and trailing whitespace scored at all.
    expect(bodyLength("😀".repeat(2001))).toBe(2001);
    expect(bodyLength(`${"x".repeat(4000)}   \n`)).toBe(4000);
    expect(rememberAction("😀".repeat(2001))).toBe("save");
    expect(rememberAction(`${"x".repeat(4000)}\n\n`)).toBe("save");
  });
});

describe("rememberAction", () => {
  it("saves at the cap and opens the dialog one character over it", () => {
    expect(rememberAction("z".repeat(4000))).toBe("save");
    expect(rememberAction("z".repeat(4001))).toBe("open-dialog");
  });
});

// @vitest-environment jsdom
// The dialog's contract (testing/feat-memory-ui.md): list, filter, save,
// forget under account:local; refetch on memory-changed behind a read
// generation; refuse over the cap instead of clipping; keep no state closed;
// and say plainly what this surface is not.
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { bridgeApi } from "../api";
import type { MemoryChangedPayload, MemoryRecord } from "../types";
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
  store = [record("r-tabs", "Prefers tabs over spaces"), record("r-tz", "Works in IST", "fact")];
  memoryHandler = undefined;
  vi.spyOn(bridgeApi, "listMemoryRecords").mockImplementation(async scopeKey => ({
    scopeKey,
    records: store.filter(item => item.status === "active"),
  }));
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

describe("rememberAction", () => {
  it("saves at the cap and opens the dialog one character over it", () => {
    expect(rememberAction("z".repeat(4000))).toBe("save");
    expect(rememberAction("z".repeat(4001))).toBe("open-dialog");
  });
});

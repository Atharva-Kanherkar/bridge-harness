// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { App } from "./App";
import { bridgeApi } from "./api";

vi.mock("@xterm/xterm", () => ({
  Terminal: class {
    options: Record<string, unknown> = {};
    open() {}
    loadAddon() {}
    dispose() {}
    onData() { return { dispose() {} }; }
    write() {}
    writeln() {}
  },
}));
vi.mock("@xterm/addon-fit", () => ({ FitAddon: class { fit() {} } }));

let container: HTMLDivElement;
let root: Root;

const settle = () => act(async () => { await new Promise(resolve => setTimeout(resolve, 50)); });

async function setComposer(value: string) {
  const field = container.querySelector<HTMLTextAreaElement>("textarea")!;
  const setter = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")!.set!;
  await act(async () => {
    setter.call(field, value);
    field.dispatchEvent(new Event("input", { bubbles: true }));
  });
  return field;
}

async function openWorkspaceSession() {
  const rows = [...container.querySelectorAll<HTMLButtonElement>('button[title*=" — "]')];
  for (const row of rows) {
    await act(async () => row.click());
    await settle();
    if (container.textContent?.includes("7 files")) return;
  }
  throw new Error("could not open the ready workspace session");
}

beforeEach(async () => {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  HTMLElement.prototype.scrollIntoView = vi.fn();
  const store = new Map<string, string>();
  Object.defineProperty(globalThis, "localStorage", {
    configurable: true,
    value: {
      getItem: (key: string) => store.get(key) ?? null,
      setItem: (key: string, value: string) => { store.set(key, String(value)); },
      removeItem: (key: string) => { store.delete(key); },
      clear: () => store.clear(),
      key: () => null,
      length: 0,
    },
  });
  container = document.createElement("div");
  document.body.append(container);
  await act(async () => {
    root = createRoot(container);
    root.render(<App />);
  });
  await settle();
  const recommended = [...container.querySelectorAll("button")]
    .find(button => button.textContent === "Use recommended defaults");
  if (recommended) {
    await act(async () => recommended.click());
    await settle();
  }
  await openWorkspaceSession();
});

afterEach(() => {
  vi.restoreAllMocks();
  act(() => root?.unmount());
  container?.remove();
});

describe("agent shortcut composer", () => {
  it("autocompletes an enabled specialist and dispatches without a parent provider turn", async () => {
    const dispatch = vi.spyOn(bridgeApi, "dispatchAgentShortcut").mockResolvedValue({
      disposition: "launched",
      childSessionId: "worker-verifier",
      agentId: "bridge-verification",
      agentName: "Verification agent",
      role: "verification",
      interceptions: [],
    });
    const prepare = vi.spyOn(bridgeApi, "prepareTurn");
    const start = vi.spyOn(bridgeApi, "startChat");
    const submit = vi.spyOn(bridgeApi, "submitInput");

    const field = await setComposer("#ver");
    expect(container.querySelector('[role="listbox"][aria-label="Specialist agents"]')).not.toBeNull();
    expect(container.textContent).toContain("#verifier");
    expect(container.textContent).toContain("verification · bridge / automatic model · read only · high effort");

    await act(async () => field.dispatchEvent(new KeyboardEvent("keydown", { key: "Tab", bubbles: true, cancelable: true })));
    expect(field.value).toBe("#verifier ");
    await setComposer("#verifier verify the release contract");
    await act(async () => field.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true })));
    await settle();

    expect(dispatch).toHaveBeenCalledWith("session-2", "verifier", "verify the release contract");
    expect(prepare).not.toHaveBeenCalled();
    expect(start).not.toHaveBeenCalled();
    expect(submit).not.toHaveBeenCalled();
    expect(container.textContent).toContain("Verification agent (verification) was launched.");
  });

  it("restores a rejected directive so it can be corrected", async () => {
    vi.spyOn(bridgeApi, "dispatchAgentShortcut").mockRejectedValue(new Error("Unknown agent shortcut #ghost"));
    const field = await setComposer("#ghost inspect the logs");

    await act(async () => field.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true })));
    await settle();

    expect(field.value).toBe("#ghost inspect the logs");
    expect(container.textContent).toContain("Unknown agent shortcut #ghost");
  });

  it.each([
    ["queued", "Research agent (research) is queued for the next worker slot."],
    ["awaitingApproval", "Implementation agent (implementation) is waiting for write-scope approval."],
  ] as const)("reports a %s reservation outcome", async (disposition, expected) => {
    vi.spyOn(bridgeApi, "dispatchAgentShortcut").mockResolvedValue({
      disposition,
      agentId: disposition === "queued" ? "bridge-research" : "bridge-implementation",
      agentName: disposition === "queued" ? "Research agent" : "Implementation agent",
      role: disposition === "queued" ? "research" : "implementation",
      interceptions: [],
    });
    const token = disposition === "queued" ? "researcher" : "implementer";
    const field = await setComposer(`#${token} do the focused task`);

    await act(async () => field.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true })));
    await settle();

    expect(container.textContent).toContain(expected);
  });
});

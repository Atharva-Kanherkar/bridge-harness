// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { bridgeApi } from "../../api";
import type { AdapterDescriptor } from "../../types";
import { ComposerPage } from "./ComposerPage";

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.restoreAllMocks();
});

const claude = { id: "claude", label: "Claude", available: true, models: [{ id: "sonnet", label: "Sonnet" }] } as unknown as AdapterDescriptor;

describe("Composer chat search settings", () => {
  it("persists the deep search switch and names the default model", async () => {
    const save = vi.spyOn(bridgeApi, "saveChatSearchSettings").mockImplementation(async settings => settings);
    vi.spyOn(bridgeApi, "chatSearchSettings").mockResolvedValue({ deepSearch: true, model: null });
    await act(async () => {
      root.render(<ComposerPage adapters={[claude]} onChange={() => {}} onError={() => {}} />);
    });
    await act(async () => { await Promise.resolve(); });
    expect(container.textContent).toContain("Chat search");
    expect(container.querySelector('[aria-label="Search model"]')?.textContent).toContain("Cheapest (Haiku)");
    const toggle = container.querySelector<HTMLButtonElement>('button[role="switch"][aria-label="Search deeper"]')!;
    expect(toggle.getAttribute("aria-checked")).toBe("true");
    await act(async () => { toggle.dispatchEvent(new MouseEvent("click", { bubbles: true })); });
    expect(save).toHaveBeenCalledWith({ deepSearch: false, model: null });
  });
});

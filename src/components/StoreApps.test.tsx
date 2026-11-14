// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { MarketplaceScreen } from "./MarketplaceScreen";
import { PIN_LIMIT_MESSAGE } from "../apps/pins";
import { enabledApps, loadApps, pinsOf, resetAppsStore, useApps } from "../apps/store";
import { resetMockApps } from "../appsMock";
import { bridgeApi } from "../api";

let host: HTMLDivElement;
let root: Root;

beforeEach(() => {
  localStorage.clear();
  resetMockApps();
  resetAppsStore();
  host = document.createElement("div");
  document.body.append(host);
  root = createRoot(host);
});

afterEach(async () => {
  await act(async () => root.unmount());
  host.remove();
  localStorage.clear();
  resetMockApps();
  resetAppsStore();
  vi.restoreAllMocks();
});

const button = (label: string) => host.querySelector<HTMLButtonElement>(`button[aria-label="${label}"]`);
const click = (element: HTMLElement | null) => act(async () => { element!.click(); });
const status = () => host.querySelector('[role="status"]')?.textContent;
const corePins = async () => pinsOf((await bridgeApi.listApps()).apps);

async function render(ui: React.ReactElement) {
  await act(async () => root.render(ui));
  await act(async () => { await loadApps(); });
}

describe("the Store's Apps tab", () => {
  it("opens first, lists every app but the Store, and loads no plugin catalog", async () => {
    const catalog = vi.spyOn(bridgeApi, "marketplaceCatalog");
    await render(<MarketplaceScreen />);
    const names = [...host.querySelectorAll('[aria-label="Installed apps"] h3')].map(node => node.textContent);
    expect(names).toEqual(["Notes", "Trails", "Pulls", "Workflows", "Memory", "Mission Control", "Terminals", "Usage", "Projects", "Agents", "Outgoing"]);
    expect(host.querySelector('[aria-label="Store sections"] [aria-pressed="true"]')?.textContent).toBe("Apps");
    expect(catalog).not.toHaveBeenCalled();
    expect(button("Pin Outgoing"), "a card-only app has nothing to pin").toBeNull();
    expect(button("Turn Outgoing off")).not.toBeNull();
  });

  it("pins and unpins through core, and shows core's refusal of a fourth pin", async () => {
    await render(<MarketplaceScreen />);
    await click(button("Pin Notes"));
    expect(await corePins()).toEqual(["notes"]);
    expect(status()).toBe("Pinned Notes to the sidebar.");
    expect(button("Unpin Notes")).not.toBeNull();
    await click(button("Pin Pulls"));
    await click(button("Pin Usage"));
    await click(button("Pin Memory"));
    expect(await corePins()).toEqual(["notes", "pulls", "usage"]);
    expect(status()).toBe(PIN_LIMIT_MESSAGE);
    await click(button("Unpin Pulls"));
    expect(await corePins()).toEqual(["notes", "usage"]);
  });

  it("turns an app off and on, which takes it out of the launcher and unpins it", async () => {
    let launcher: readonly string[] = [];
    function Launcher() {
      launcher = enabledApps(useApps()).map(app => app.id);
      return null;
    }
    await render(<><MarketplaceScreen /><Launcher /></>);
    await click(button("Pin Notes"));
    expect(launcher).toContain("notes");
    await click(button("Turn Notes off"));
    expect(status()).toBe("Turned Notes off. New chats no longer get its tools.");
    expect(launcher).not.toContain("notes");
    expect(await corePins()).toEqual([]);
    expect(button("Pin Notes"), "an app that is off cannot be pinned").toBeNull();
    expect(host.querySelector('[data-app="notes"]')?.getAttribute("data-enabled")).toBe("false");
    await click(button("Turn Notes on"));
    expect(launcher).toContain("notes");
    expect(button("Turn Store off"), "the Store cannot be turned off").toBeNull();
  });

  it("opens an app", async () => {
    const onOpenApp = vi.fn();
    await render(<MarketplaceScreen onOpenApp={onOpenApp} />);
    await click(button("Open Usage"));
    expect(onOpenApp).toHaveBeenCalledWith("usage");
  });
});

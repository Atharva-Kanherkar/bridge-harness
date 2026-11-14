// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { MarketplaceScreen } from "./MarketplaceScreen";
import { PIN_LIMIT_MESSAGE, readPins, resetPinsCache } from "../apps/pins";
import { bridgeApi } from "../api";

let host: HTMLDivElement;
let root: Root;

beforeEach(() => {
  localStorage.clear();
  resetPinsCache();
  host = document.createElement("div");
  document.body.append(host);
  root = createRoot(host);
});

afterEach(async () => {
  await act(async () => root.unmount());
  host.remove();
  localStorage.clear();
  resetPinsCache();
  vi.restoreAllMocks();
});

const button = (label: string) => host.querySelector<HTMLButtonElement>(`button[aria-label="${label}"]`);
const click = (element: HTMLElement | null) => act(async () => { element!.click(); });
const status = () => host.querySelector('[role="status"]')?.textContent;

describe("the Store's Apps tab", () => {
  it("opens first, lists every app but the Store, and loads no plugin catalog", async () => {
    const catalog = vi.spyOn(bridgeApi, "marketplaceCatalog");
    await act(async () => root.render(<MarketplaceScreen />));
    const names = [...host.querySelectorAll('[aria-label="Installed apps"] h3')].map(node => node.textContent);
    expect(names).toEqual(["Notes", "Pulls", "Workflows", "Memory", "Mission Control", "Terminals", "Usage", "Projects", "Agents"]);
    expect(host.querySelector('[aria-label="Store sections"] [aria-pressed="true"]')?.textContent).toBe("Apps");
    expect(catalog).not.toHaveBeenCalled();
  });

  it("pins and unpins, and says why a fourth pin is refused", async () => {
    await act(async () => root.render(<MarketplaceScreen />));
    await click(button("Pin Notes"));
    expect(readPins()).toEqual(["notes"]);
    expect(status()).toBe("Pinned Notes to the sidebar.");
    expect(button("Unpin Notes")).not.toBeNull();
    await click(button("Pin Pulls"));
    await click(button("Pin Usage"));
    await click(button("Pin Memory"));
    expect(readPins()).toEqual(["notes", "pulls", "usage"]);
    expect(status()).toBe(PIN_LIMIT_MESSAGE);
    await click(button("Unpin Pulls"));
    expect(readPins()).toEqual(["notes", "usage"]);
  });

  it("opens an app", async () => {
    const onOpenApp = vi.fn();
    await act(async () => root.render(<MarketplaceScreen onOpenApp={onOpenApp} />));
    await click(button("Open Usage"));
    expect(onOpenApp).toHaveBeenCalledWith("usage");
  });
});

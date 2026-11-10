// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { DEFAULT_ATTENTION_SETTINGS, readAttentionSettings } from "../../attentionSettings";
import { NotificationsPage } from "./NotificationsPage";
import { filterSettingsRows, STATIC_SETTINGS_ROWS } from "./settingsSearch";

const playAttentionSound = vi.hoisted(() => vi.fn());
vi.mock("../../attentionSound", () => ({ playAttentionSound }));

let root: Root | undefined;
let host: HTMLDivElement | undefined;

beforeEach(async () => {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  const store = new Map<string, string>();
  Object.defineProperty(globalThis, "localStorage", {
    configurable: true,
    value: { getItem: (key: string) => store.get(key) ?? null, setItem: (key: string, value: string) => { store.set(key, value); }, removeItem: (key: string) => { store.delete(key); } },
  });
  playAttentionSound.mockReset();
  host = document.createElement("div");
  document.body.append(host);
  root = createRoot(host);
  await act(async () => { root?.render(<NotificationsPage />); });
});

afterEach(async () => {
  await act(async () => { root?.unmount(); });
  host?.remove();
});

const control = (label: string) => host!.querySelector(`[aria-label="${label}"]`) as HTMLButtonElement & HTMLInputElement;

describe("NotificationsPage", () => {
  it("shows toast, banner and sound switches for each kind, set from the defaults", () => {
    for (const kind of ["Needs you", "An agent asks for you", "A result is waiting", "Turn failed", "Turn completed"]) {
      for (const channel of ["Toast", "Banner", "Sound"]) expect(control(`${channel} for ${kind}`), `${channel} for ${kind}`).toBeTruthy();
    }
    expect(control("Sound for Needs you").getAttribute("aria-checked")).toBe("true");
    expect(control("Sound for Turn completed").getAttribute("aria-checked")).toBe("false");
  });

  it("writes a toggle and shows it", async () => {
    await act(async () => { control("Toast for Turn completed").click(); });
    expect(readAttentionSettings().kinds.finished.toast).toBe(false);
    expect(control("Toast for Turn completed").getAttribute("aria-checked")).toBe("false");
    expect(readAttentionSettings().kinds.needsYou).toEqual(DEFAULT_ATTENTION_SETTINGS.kinds.needsYou);
  });

  it("previews a kind's sound", async () => {
    await act(async () => { control("Play the Needs you sound").click(); });
    expect(playAttentionSound).toHaveBeenCalledWith("needsYou");
  });

  it("turns quiet hours on and sets their times", async () => {
    expect(control("Quiet hours start").disabled).toBe(true);
    await act(async () => { control("Quiet hours").click(); });
    expect(readAttentionSettings().quietHours.enabled).toBe(true);
    const start = control("Quiet hours start");
    expect(start.disabled).toBe(false);
    await act(async () => {
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(start, "21:30");
      start.dispatchEvent(new Event("input", { bubbles: true }));
    });
    expect(readAttentionSettings().quietHours.start).toBe("21:30");
  });

  it("is found by the settings search", () => {
    const hits = filterSettingsRows("quiet hours", STATIC_SETTINGS_ROWS);
    expect(hits.some(group => group.section === "notifications")).toBe(true);
  });
});

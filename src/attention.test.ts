// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const isPermissionGranted = vi.hoisted(() => vi.fn());
const requestPermission = vi.hoisted(() => vi.fn());
const sendNotification = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/plugin-notification", () => ({ isPermissionGranted, requestPermission, sendNotification }));

async function loadAttention() {
  vi.resetModules();
  return import("./attention");
}

function setTauri(enabled: boolean) {
  if (enabled) {
    Object.defineProperty(window, "__TAURI_INTERNALS__", { value: {}, configurable: true });
  } else {
    Reflect.deleteProperty(window, "__TAURI_INTERNALS__");
  }
}

beforeEach(() => {
  isPermissionGranted.mockReset().mockResolvedValue(true);
  requestPermission.mockReset().mockResolvedValue("granted");
  sendNotification.mockReset();
  setTauri(true);
});

afterEach(() => {
  setTauri(false);
});

describe("focus tracking", () => {
  it("flips to unfocused on blur and back on focus", async () => {
    const { isBridgeFocused } = await loadAttention();
    window.dispatchEvent(new Event("blur"));
    expect(isBridgeFocused()).toBe(false);
    window.dispatchEvent(new Event("focus"));
    expect(isBridgeFocused()).toBe(true);
  });
});

describe("notifyAttention", () => {
  it("does not notify while Bridge is focused", async () => {
    const { notifyAttention } = await loadAttention();
    window.dispatchEvent(new Event("focus"));
    await notifyAttention("title", "body");
    expect(sendNotification).not.toHaveBeenCalled();
  });

  it("does not notify outside Tauri, even when unfocused", async () => {
    const { notifyAttention } = await loadAttention();
    window.dispatchEvent(new Event("blur"));
    setTauri(false);
    await notifyAttention("title", "body");
    expect(sendNotification).not.toHaveBeenCalled();
  });

  it("sends a notification when unfocused, in Tauri, and already permitted", async () => {
    const { notifyAttention } = await loadAttention();
    window.dispatchEvent(new Event("blur"));
    await notifyAttention("Bridge needs you", "chat-1 is waiting");
    expect(sendNotification).toHaveBeenCalledWith({ title: "Bridge needs you", body: "chat-1 is waiting" });
  });

  it("requests permission once when not yet granted, and skips sending if denied", async () => {
    isPermissionGranted.mockResolvedValue(false);
    requestPermission.mockResolvedValue("denied");
    const { notifyAttention } = await loadAttention();
    window.dispatchEvent(new Event("blur"));
    await notifyAttention("title", "body");
    await notifyAttention("title", "body");
    expect(requestPermission).toHaveBeenCalledTimes(1);
    expect(sendNotification).not.toHaveBeenCalled();
  });
});

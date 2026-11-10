// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const isPermissionGranted = vi.hoisted(() => vi.fn());
const requestPermission = vi.hoisted(() => vi.fn());
const sendNotification = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/plugin-notification", () => ({ isPermissionGranted, requestPermission, sendNotification }));
const emit = vi.hoisted(() => vi.fn(async () => undefined));
vi.mock("@tauri-apps/api/event", () => ({ emit }));
const activeWebDaemon = vi.hoisted(() => vi.fn(() => null as object | null));
vi.mock("./webDaemon", () => ({ activeWebDaemon }));
const notifyInBrowser = vi.hoisted(() => vi.fn(async () => undefined));
vi.mock("./webNotify", () => ({ notifyInBrowser }));

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
  emit.mockClear();
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
    await notifyAttention("title", "body", { sessionId: "chat-1" });
    expect(emit).not.toHaveBeenCalled();
  });

  it("does not notify outside Tauri without a daemon link, even when unfocused", async () => {
    const { notifyAttention } = await loadAttention();
    window.dispatchEvent(new Event("blur"));
    setTauri(false);
    activeWebDaemon.mockReturnValue(null);
    await notifyAttention("title", "body");
    expect(emit).not.toHaveBeenCalled();
    expect(notifyInBrowser).not.toHaveBeenCalled();
  });

  it.each(["MacIntel", "Linux x86_64"])("hands the banner and its ids to the shell on %s", async platform => {
    vi.spyOn(navigator, "platform", "get").mockReturnValue(platform);
    const { notifyAttention } = await loadAttention();
    window.dispatchEvent(new Event("blur"));
    await notifyAttention("Bridge needs you", "chat-1 is waiting", { sessionId: "chat-1", approvalEventId: 42 });
    expect(emit).toHaveBeenCalledWith("bridge-native-notify", { title: "Bridge needs you", body: "chat-1 is waiting", sessionId: "chat-1", approvalEventId: 42 });
    expect(sendNotification).not.toHaveBeenCalled();
    expect(requestPermission).not.toHaveBeenCalled();
  });

  it("sends null ids for a banner about nothing in particular", async () => {
    const { notifyAttention } = await loadAttention();
    window.dispatchEvent(new Event("blur"));
    await notifyAttention("Handled already", "That approval was answered somewhere else.");
    expect(emit).toHaveBeenCalledWith("bridge-native-notify", { title: "Handled already", body: "That approval was answered somewhere else.", sessionId: null, approvalEventId: null });
  });
});

describe("the web client", () => {
  beforeEach(() => {
    setTauri(false);
    notifyInBrowser.mockClear();
  });

  it("posts through the browser when a daemon link is active and Bridge is in the background", async () => {
    activeWebDaemon.mockReturnValue({});
    const { notifyAttention } = await loadAttention();
    window.dispatchEvent(new Event("blur"));
    await notifyAttention("Bridge needs you", "Fix login · Claude is waiting", { sessionId: "chat-1" });
    expect(notifyInBrowser).toHaveBeenCalledWith("Bridge needs you", "Fix login · Claude is waiting", "chat-1");
    expect(sendNotification).not.toHaveBeenCalled();
  });

  it("posts nothing while Bridge is focused, or in mock mode", async () => {
    activeWebDaemon.mockReturnValue({});
    const { notifyAttention } = await loadAttention();
    window.dispatchEvent(new Event("focus"));
    await notifyAttention("Bridge needs you", "x", { sessionId: "chat-1" });
    activeWebDaemon.mockReturnValue(null);
    window.dispatchEvent(new Event("blur"));
    await notifyAttention("Bridge needs you", "x", { sessionId: "chat-1" });
    expect(notifyInBrowser).not.toHaveBeenCalled();
  });
});


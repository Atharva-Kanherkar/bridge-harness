import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { bridgeApi } from "./api";
import type { CloneSnapshot } from "./protocol/generated/protocol";

const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));
beforeEach(() => {
  invoke.mockReset();
  vi.stubGlobal("window", { __TAURI_INTERNALS__: {} });
});
afterEach(() => vi.unstubAllGlobals());

const snapshot = (status: string, waitingReason?: string): CloneSnapshot => ({
  sessionId: "chat", cloneId: "clone", domain: "chrome.google.com", status,
  signInPath: "import", minutesLeft: 30, screenshot: null, screenshotRedactedRegions: 0,
  pendingRequest: null, pendingRequestId: null, extensionPath: null, additionalDomains: null,
  waitingReason,
});

it.each(["acting", "waiting_for_you", "taken_over"])("preserves native sign-in recovery guidance while %s", async status => {
  const message = "Google rejected this browser. Sign in in normal Chrome, then approve google.com.";
  invoke.mockResolvedValue(snapshot(status, message));
  expect((await bridgeApi.browserCloneState("chat")).waitingReason).toBe(message);
});

it("keeps ordinary waiting guidance and accepts older native snapshots", async () => {
  invoke.mockResolvedValue(snapshot("waiting_for_you"));
  expect((await bridgeApi.browserCloneState("chat")).waitingReason).toContain("Sign in");
  invoke.mockResolvedValue(snapshot("acting"));
  expect((await bridgeApi.browserCloneState("chat")).waitingReason).toBeNull();
  invoke.mockResolvedValue(null);
  expect((await bridgeApi.browserCloneState("chat")).status).toBe("none");
});

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { backendMode, bridgeApi } from "./api";
import { setActiveWebDaemon, type WebDaemonClient } from "./webDaemon";
import type { AgentEvent, MenuBarSettings } from "./types";

const invoke = vi.hoisted(() => vi.fn());
const listen = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));
vi.mock("@tauri-apps/api/event", () => ({ listen, emit: vi.fn() }));

/** A stand-in for the WebSocket pool: records calls, lets the test publish. */
function fakeDaemon(answer: (method: string, params: unknown) => unknown) {
  const listeners = new Map<string, Set<(payload: unknown) => void>>();
  const client = {
    call: vi.fn(async (method: string, params?: unknown) => answer(method, params)),
    subscribe: vi.fn((name: string, handler: (payload: unknown) => void) => {
      if (!listeners.has(name)) listeners.set(name, new Set());
      listeners.get(name)!.add(handler);
      return () => listeners.get(name)!.delete(handler);
    }),
  };
  const publish = (name: string, payload: unknown) => { for (const handler of listeners.get(name) ?? []) handler(payload); };
  return { client, publish, pin: () => setActiveWebDaemon(client as unknown as WebDaemonClient) };
}

beforeEach(() => { invoke.mockReset(); listen.mockReset(); });
afterEach(() => setActiveWebDaemon(undefined));

describe("api.ts over a web daemon", () => {
  it("routes calls to the daemon instead of mock data", async () => {
    const daemon = fakeDaemon(method => (method === "health/health" ? { ok: true, adapters: [], version: "daemon" } : null));
    daemon.pin();
    expect(backendMode()).toBe("web");
    const health = await bridgeApi.health();
    expect(health.version).toBe("daemon");
    expect(daemon.client.call).toHaveBeenCalledWith("health/health", undefined);
    expect(invoke).not.toHaveBeenCalled();
  });

  it("forwards params verbatim", async () => {
    const daemon = fakeDaemon(() => ({ workspaces: [], sessions: [] }));
    daemon.pin();
    await bridgeApi.workspaceChanges("w1");
    expect(daemon.client.call).toHaveBeenCalledWith("workspaces/workspace_changes", { workspaceId: "w1" });
  });

  it("subscribes protocol notifications on the daemon", async () => {
    const daemon = fakeDaemon(() => null);
    daemon.pin();
    const changed = vi.fn();
    const off = await bridgeApi.onStateChanged(changed);
    daemon.publish("state-changed", null);
    expect(changed).toHaveBeenCalledTimes(1);
    off();
    daemon.publish("state-changed", null);
    expect(changed).toHaveBeenCalledTimes(1);
    expect(listen).not.toHaveBeenCalled();
  });

  it("takes single agent-event frames, in order", async () => {
    const daemon = fakeDaemon(() => null);
    daemon.pin();
    const seen: string[] = [];
    await bridgeApi.onAgentEvent(event => seen.push(event.id));
    const frame = (id: string) => ({ id, sessionId: "s", kind: "assistant.delta", providerMeta: {} }) as unknown as AgentEvent;
    daemon.publish("agent-event", frame("a"));
    daemon.publish("agent-event", frame("b"));
    expect(seen).toEqual(["a", "b"]);
    expect(daemon.client.subscribe).toHaveBeenCalledWith("agent-event", expect.any(Function));
  });

  it("saves menu-bar settings and relays the hint locally, without Tauri", async () => {
    const settings = { enabled: true } as unknown as MenuBarSettings;
    const daemon = fakeDaemon((_method, params) => (params as { settings: MenuBarSettings }).settings);
    daemon.pin();
    const hinted = vi.fn();
    const off = await bridgeApi.onMenuBarSettingsChanged(hinted);
    await expect(bridgeApi.saveMenuBarSettings(settings)).resolves.toEqual(settings);
    expect(hinted).toHaveBeenCalledTimes(1);
    off();
    expect(listen).not.toHaveBeenCalled();
  });

  it("keeps shell-only channels silent in a browser", async () => {
    fakeDaemon(() => null).pin();
    const off = await bridgeApi.onMenuCommand(() => undefined);
    off();
    await bridgeApi.revealMainWindow();
    expect(listen).not.toHaveBeenCalled();
  });

  it("is mock mode with no daemon configured", async () => {
    setActiveWebDaemon(null);
    expect(backendMode()).toBe("mock");
    const health = await bridgeApi.health();
    expect(health.adapters.length).toBeGreaterThan(0);
    expect(invoke).not.toHaveBeenCalled();
  });
});

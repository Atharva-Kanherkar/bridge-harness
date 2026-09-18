// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { bridgeApi } from "./api";
import type { BridgeState, Session, SessionStatus } from "./types";

const notifyAttention = vi.hoisted(() => vi.fn());
vi.mock("./attention", () => ({ notifyAttention }));

function session(id: string, status: SessionStatus, overrides: Partial<Session> = {}): Session {
  return {
    continuationFidelity: "full",
    harness: "claude",
    id,
    kind: "chat",
    label: `Chat ${id}`,
    metricSource: "provider",
    restorationMode: "resume",
    status,
    title: `Title ${id}`,
    ...overrides,
  };
}

function stateWith(sessions: Session[]): BridgeState {
  return { projects: [], workspaces: [], sessions, events: [] };
}

describe("App attention wiring", () => {
  let container: HTMLDivElement;
  let root: Root;
  let stateHandler: (() => void) | undefined;
  let currentSessions: Session[];

  beforeEach(() => {
    (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
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
    notifyAttention.mockReset();
    currentSessions = [session("a", "working")];
    stateHandler = undefined;
    vi.spyOn(bridgeApi, "onStateChanged").mockImplementation(async handler => {
      stateHandler = handler;
      return () => { if (stateHandler === handler) stateHandler = undefined; };
    });
    vi.spyOn(bridgeApi, "state").mockImplementation(async () => stateWith(currentSessions));
    container = document.createElement("div");
    document.body.append(container);
    root = createRoot(container);
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
    vi.restoreAllMocks();
  });

  it("notifies once when a session moves working → waiting, and not again on an unchanged snapshot", async () => {
    const { App } = await import("./App");
    await act(async () => {
      root.render(<App />);
      await new Promise(resolve => setTimeout(resolve, 40));
    });
    expect(notifyAttention).not.toHaveBeenCalled();
    expect(stateHandler, "App subscribed to state changes").toBeTruthy();

    currentSessions = [session("a", "waiting")];
    await act(async () => {
      stateHandler!();
      await new Promise(resolve => setTimeout(resolve, 40));
    });
    expect(notifyAttention).toHaveBeenCalledTimes(1);
    expect(notifyAttention).toHaveBeenCalledWith(
      "Bridge needs you",
      "Title a is waiting for your input",
    );

    await act(async () => {
      stateHandler!();
      await new Promise(resolve => setTimeout(resolve, 40));
    });
    expect(notifyAttention).toHaveBeenCalledTimes(1);
  });
});

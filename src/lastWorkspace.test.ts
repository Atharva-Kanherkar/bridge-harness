import { describe, expect, it, beforeEach } from "vitest";
import { LAST_WORKSPACE_KEY, readLastWorkspaceId, resolveNewChatWorkspaceId, writeLastWorkspaceId } from "./lastWorkspace";

const workspaces = [{ id: "ws-1" }, { id: "ws-2" }];

beforeEach(() => {
  const store = new Map<string, string>();
  Object.defineProperty(globalThis, "localStorage", {
    configurable: true,
    value: {
      getItem: (key: string) => store.get(key) ?? null,
      setItem: (key: string, value: string) => { store.set(key, value); },
      removeItem: (key: string) => { store.delete(key); },
      clear: () => store.clear(),
    },
  });
});

describe("resolveNewChatWorkspaceId", () => {
  it("prefers the active session's workspace when it still exists", () => {
    expect(resolveNewChatWorkspaceId({
      activeWorkspaceId: "ws-2",
      lastWorkspaceId: "ws-1",
      workspaces,
    })).toBe("ws-2");
  });

  it("falls through a stale active id to the last-used repo", () => {
    expect(resolveNewChatWorkspaceId({
      activeWorkspaceId: "gone",
      lastWorkspaceId: "ws-1",
      workspaces,
    })).toBe("ws-1");
  });

  it("falls through a stale last-used id to the only remaining workspace", () => {
    expect(resolveNewChatWorkspaceId({
      activeWorkspaceId: null,
      lastWorkspaceId: "gone",
      workspaces: [{ id: "ws-1" }],
    })).toBe("ws-1");
  });

  it("returns null when nothing can be resolved", () => {
    expect(resolveNewChatWorkspaceId({
      activeWorkspaceId: null,
      lastWorkspaceId: "gone",
      workspaces,
    })).toBeNull();
    expect(resolveNewChatWorkspaceId({ workspaces: [] })).toBeNull();
  });
});

describe("last workspace persistence", () => {
  it("round-trips an id", () => {
    expect(readLastWorkspaceId()).toBeNull();
    writeLastWorkspaceId("ws-1");
    expect(readLastWorkspaceId()).toBe("ws-1");
    expect(localStorage.getItem(LAST_WORKSPACE_KEY)).toBe("ws-1");
  });
});

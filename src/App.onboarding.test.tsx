// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { App } from "./App";
import { bridgeApi } from "./api";

describe("fresh-install agent onboarding", () => {
  let container: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
    const values = new Map<string, string>();
    Object.defineProperty(globalThis, "localStorage", {
      configurable: true,
      value: {
        getItem: (key: string) => values.get(key) ?? null,
        setItem: (key: string, value: string) => values.set(key, String(value)),
      },
    });
    vi.spyOn(bridgeApi, "state").mockResolvedValue({ projects: [], workspaces: [], sessions: [], events: [] });
    container = document.createElement("div");
    document.body.append(container);
    root = createRoot(container);
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
    vi.restoreAllMocks();
  });

  it("opens agent discovery before the normal empty workspace", async () => {
    await act(async () => {
      root.render(<App />);
      await new Promise(resolve => setTimeout(resolve, 30));
    });

    expect(container.textContent).toContain("Bring your agents with you");
    expect(container.textContent).toContain("Codex");
    expect(container.textContent).not.toContain("Your workspace, ready");
  });
});

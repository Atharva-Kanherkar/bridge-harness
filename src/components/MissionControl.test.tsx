// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { Workspace } from "../types";
import { MissionControl } from "./MissionControl";

vi.mock("./TerminalWorkspace", () => ({ TerminalWorkspace: ({ workspaceId }: { workspaceId: string }) => <div data-workspace={workspaceId} /> }));
const workspaces = [{ id: "one", title: "Bridge", branch: "main" }, { id: "two", title: "Bridge worktree", branch: "feature" }] as Workspace[];
let host: HTMLDivElement;
let root: Root;
beforeEach(() => { const store = new Map<string, string>(); vi.stubGlobal("localStorage", { getItem: (key: string) => store.get(key) ?? null, setItem: (key: string, value: string) => store.set(key, value) }); (globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true; host = document.createElement("div"); document.body.append(host); root = createRoot(host); });
afterEach(() => { act(() => root.unmount()); host.remove(); });
it("opens the selected checkout and restores workspace selection", () => {
  act(() => root.render(<MissionControl workspaces={workspaces} initialWorkspaceId="two" onOpenProjects={vi.fn()} />));
  expect(host.querySelector("[data-workspace]")?.getAttribute("data-workspace")).toBe("two");
  const select = host.querySelector("select")!;
  act(() => { select.value = "one"; select.dispatchEvent(new Event("change", { bubbles: true })); });
  expect(host.querySelector("[data-workspace]")?.getAttribute("data-workspace")).toBe("one");
  expect(localStorage.getItem("bridge.mission-control.workspace")).toBe("one");
});
it("repairs a selection whose workspace was removed", () => {
  localStorage.setItem("bridge.mission-control.workspace", "deleted");
  act(() => root.render(<MissionControl workspaces={workspaces} onOpenProjects={vi.fn()} />));
  expect(host.querySelector("[data-workspace]")?.getAttribute("data-workspace")).toBe("one");
});
it("opens Projects when no checkout is connected", () => {
  const open = vi.fn();
  act(() => root.render(<MissionControl workspaces={[]} onOpenProjects={open} />));
  act(() => host.querySelector("button")!.click());
  expect(open).toHaveBeenCalledOnce();
  expect(host.querySelector("[data-workspace]")).toBeNull();
});

// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { bridgeApi } from "../api";
import { GITPLACE_REPO_KEY, GitplaceScreen, gitplaceRepos } from "./GitplaceScreen";
import type { Project, Workspace } from "../types";

const workspace = (id: string, overrides: Partial<Workspace> = {}): Workspace => ({
  id, title: id, path: `/repos/${id}`, status: "ready", additions: 0, deletions: 0, dirtyFiles: 0, createdAt: "2026-09-01T00:00:00Z", branch: "main", ...overrides,
});
const projects: Project[] = [{ id: "p1", name: "harness", path: "/repos/harness", createdAt: "2026-09-01T00:00:00Z" }];

let root: Root;
let host: HTMLDivElement;
const flush = async () => { for (let i = 0; i < 4; i += 1) await Promise.resolve(); };

beforeEach(() => {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  // jsdom here ships no Storage; a Map is all the screen needs.
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
  host = document.createElement("div");
  document.body.append(host);
  root = createRoot(host);
});

afterEach(async () => {
  await act(async () => root.unmount());
  host.remove();
  vi.restoreAllMocks();
});

describe("gitplaceRepos", () => {
  it("lists one entry per project, skips folderless workspaces, and names it after the project", () => {
    const repos = gitplaceRepos([
      workspace("a", { projectId: "p1", createdAt: "2026-09-01T00:00:00Z" }),
      workspace("b", { projectId: "p1", createdAt: "2026-09-03T00:00:00Z" }),
      workspace("c", { path: null }),
      workspace("d"),
    ], projects);
    expect(repos.map(repo => [repo.workspace.id, repo.label])).toEqual([["d", "d"], ["b", "harness"]]);
  });
});

describe("GitplaceScreen", () => {
  it("renders the GitHub pane at page width with no session, and switches repositories", async () => {
    const statusSpy = vi.spyOn(bridgeApi, "githubStatus");
    vi.spyOn(bridgeApi, "attributionSettings").mockResolvedValue({ hideAiAttribution: false });
    await act(async () => root.render(<GitplaceScreen workspaces={[workspace("alpha"), workspace("beta")]} projects={[]} onJumpToFile={() => {}} onAddProject={() => {}} />));
    await act(flush);
    expect(host.querySelector('section[aria-label="GitHub repository"]')).not.toBeNull();
    expect(statusSpy).toHaveBeenLastCalledWith("alpha", false);

    await act(async () => host.querySelector<HTMLButtonElement>('button[aria-label^="Repository:"]')!.click());
    const beta = [...document.querySelectorAll<HTMLButtonElement>('[role="menuitemradio"]')].find(item => item.textContent?.includes("beta"))!;
    await act(async () => beta.click());
    await act(flush);
    expect(statusSpy).toHaveBeenLastCalledWith("beta", false);
    expect(localStorage.getItem(GITPLACE_REPO_KEY)).toBe("beta");
  });

  it("reopens the repository it was last on", async () => {
    vi.spyOn(bridgeApi, "attributionSettings").mockResolvedValue({ hideAiAttribution: false });
    localStorage.setItem(GITPLACE_REPO_KEY, "beta");
    await act(async () => root.render(<GitplaceScreen workspaces={[workspace("alpha"), workspace("beta")]} projects={[]} onJumpToFile={() => {}} onAddProject={() => {}} />));
    await act(flush);
    expect(host.querySelector('button[aria-label="Repository: beta"]')).not.toBeNull();
  });

  it("asks for a project when no workspace has a folder", async () => {
    const add = vi.fn();
    await act(async () => root.render(<GitplaceScreen workspaces={[workspace("x", { path: null })]} projects={[]} onJumpToFile={() => {}} onAddProject={add} />));
    expect(host.textContent).toContain("Add a project with a GitHub remote");
    await act(async () => [...host.querySelectorAll("button")].find(button => button.textContent === "Add a project")!.click());
    expect(add).toHaveBeenCalledTimes(1);
  });

  it("renders the hide-attribution toggle and persists the switch", async () => {
    vi.spyOn(bridgeApi, "attributionSettings").mockResolvedValue({ hideAiAttribution: true });
    const save = vi.spyOn(bridgeApi, "saveAttributionSettings").mockResolvedValue({ hideAiAttribution: false });
    await act(async () => root.render(<GitplaceScreen workspaces={[workspace("alpha")]} projects={[]} onJumpToFile={() => {}} onAddProject={() => {}} />));
    await act(flush);
    const toggle = host.querySelector('button[role="switch"][aria-label="Hide AI attribution"]') as HTMLButtonElement | null;
    expect(toggle).not.toBeNull();
    expect(toggle!.getAttribute("aria-checked")).toBe("true");
    await act(async () => toggle!.click());
    await act(flush);
    expect(save).toHaveBeenCalledWith({ hideAiAttribution: false });
  });

  it("rolls the switch back and shows an alert when the save is rejected", async () => {
    vi.spyOn(bridgeApi, "attributionSettings").mockResolvedValue({ hideAiAttribution: false });
    vi.spyOn(bridgeApi, "saveAttributionSettings").mockRejectedValue(new Error("method_not_found"));
    await act(async () => root.render(<GitplaceScreen workspaces={[workspace("alpha")]} projects={[]} onJumpToFile={() => {}} onAddProject={() => {}} />));
    await act(flush);
    const toggle = host.querySelector('button[role="switch"][aria-label="Hide AI attribution"]') as HTMLButtonElement | null;
    expect(toggle!.getAttribute("aria-checked")).toBe("false");
    await act(async () => toggle!.click());
    await act(flush);
    expect(toggle!.getAttribute("aria-checked")).toBe("false");
    expect(host.querySelector('[role="alert"]')).not.toBeNull();
  });
});

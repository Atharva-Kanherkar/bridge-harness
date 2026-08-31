// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Workspace, WorkspaceChangesResult, WorkspaceFileChange } from "../types";
import { ChangesPanel, STATS_REFRESH_DEBOUNCE_MS } from "./ChangesPanel";
import { bridgeApi } from "../api";

// Contracts: testing/feat-dock-changes.md §1–§4 and
// testing/fix-issue-191-changes-audit.md §Unit Tests.

const workspace = (overrides: Partial<Workspace> = {}): Workspace => ({
  id: "demo-1",
  projectId: "demo-project",
  city: "Kyoto",
  title: "Build session supervisor",
  branch: "bridge/session-supervisor",
  path: "/Users/you/bridge/Kyoto",
  status: "working",
  dirtyFiles: 4,
  additions: 284,
  deletions: 31,
  createdAt: new Date().toISOString(),
  ...overrides,
});

const fileChange = (path: string, overrides: Partial<WorkspaceFileChange> = {}): WorkspaceFileChange => ({
  path,
  previousPath: null,
  changeKind: "modified",
  additions: 1,
  deletions: 0,
  patch: "@@ -1 +1 @@\n-old\n+new\n",
  patchTruncated: false,
  binary: false,
  importance: "low",
  labels: [],
  lowSignal: false,
  ...overrides,
});

const changesResult = (files: WorkspaceFileChange[], overrides: Partial<WorkspaceChangesResult> = {}): WorkspaceChangesResult => ({
  baseCommit: "a1b2c3d4",
  repositoryState: "normal",
  files,
  totalFiles: files.length,
  filesTruncated: false,
  ...overrides,
});

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}

let container: HTMLDivElement;
let root: Root;

async function mount(element: React.ReactElement) {
  await act(async () => root.render(element));
  await act(async () => {
    await new Promise(resolve => setTimeout(resolve, 0));
  });
}

const click = async (element: Element) => {
  await act(async () => {
    element.dispatchEvent(new MouseEvent("click", { bubbles: true }));
  });
};

const button = (label: string) => container.querySelector<HTMLButtonElement>(`button[aria-label="${label}"]`);
const scrollRoot = () => container.firstElementChild as HTMLElement;

beforeEach(() => {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});

afterEach(async () => {
  await act(async () => root.unmount());
  container.remove();
  vi.restoreAllMocks();
});

describe("ChangesPanel in the dock", () => {
  it("renders the review with its rank line, totals, and viewed counter", async () => {
    await mount(<ChangesPanel workspace={workspace()} />);
    expect(container.textContent).toContain("CHANGES");
    expect(container.textContent).toContain("4 files changed");
    expect(container.textContent).toContain("+303");
    expect(container.textContent).toContain("0/4 viewed");
  });

  it("fills its host instead of centering a reading column", async () => {
    await mount(<ChangesPanel workspace={workspace()} />);
    expect(scrollRoot().className).not.toContain("mx-auto");
    expect(scrollRoot().className).not.toContain("max-w-3xl");
    expect(scrollRoot().className).toContain("w-full");
  });

  it("states the diff basis: branch, uncommitted vs HEAD, base commit", async () => {
    await mount(<ChangesPanel workspace={workspace()} />);
    expect(container.textContent).toContain("bridge/session-supervisor");
    expect(container.textContent).toContain("uncommitted vs HEAD");
    expect(container.textContent).toContain("a1b2c3d4");
  });

  it("reloads once, in place, after stats drift settles", async () => {
    const spy = vi.spyOn(bridgeApi, "workspaceChanges");
    await mount(<ChangesPanel workspace={workspace()} />);
    expect(spy).toHaveBeenCalledTimes(1);

    const row = button("Mark src-tauri/bridge-core/src/policy.rs as viewed")!;
    await click(row);
    const expander = [...container.querySelectorAll<HTMLButtonElement>("button[aria-expanded]")][0];
    await click(expander);
    expect(expander.getAttribute("aria-expanded")).toBe("true");
    const rootBefore = scrollRoot();
    rootBefore.scrollTop = 120;

    await mount(<ChangesPanel workspace={workspace({ dirtyFiles: 5, additions: 300 })} />);
    expect(spy).toHaveBeenCalledTimes(1);
    await act(async () => {
      await new Promise(resolve => setTimeout(resolve, STATS_REFRESH_DEBOUNCE_MS + 40));
    });
    expect(spy).toHaveBeenCalledTimes(2);

    expect(scrollRoot()).toBe(rootBefore);
    expect(scrollRoot().scrollTop).toBe(120);
    expect([...container.querySelectorAll("button[aria-expanded]")][0].getAttribute("aria-expanded")).toBe("true");
    expect(button("Mark src-tauri/bridge-core/src/policy.rs as not viewed")).not.toBeNull();
  });

  it("does not refetch for identical stats", async () => {
    const spy = vi.spyOn(bridgeApi, "workspaceChanges");
    await mount(<ChangesPanel workspace={workspace()} />);
    await mount(<ChangesPanel workspace={workspace()} />);
    await act(async () => {
      await new Promise(resolve => setTimeout(resolve, STATS_REFRESH_DEBOUNCE_MS + 40));
    });
    expect(spy).toHaveBeenCalledTimes(1);
  });

  it("keeps the newest overlapping refresh result", async () => {
    const older = deferred<WorkspaceChangesResult>();
    const newer = deferred<WorkspaceChangesResult>();
    vi.spyOn(bridgeApi, "workspaceChanges")
      .mockReturnValueOnce(older.promise)
      .mockReturnValueOnce(newer.promise);

    await mount(<ChangesPanel workspace={workspace()} />);
    await mount(<ChangesPanel workspace={workspace({ dirtyFiles: 5 })} />);
    await act(async () => {
      await new Promise(resolve => setTimeout(resolve, STATS_REFRESH_DEBOUNCE_MS + 40));
    });

    await act(async () => {
      newer.resolve(changesResult([fileChange("newer.ts")]));
      await newer.promise;
    });
    expect(container.textContent).toContain("newer.ts");

    await act(async () => {
      older.resolve(changesResult([fileChange("older.ts")]));
      await older.promise;
    });
    expect(container.textContent).toContain("newer.ts");
    expect(container.textContent).not.toContain("older.ts");
  });

  it("ignores a stale request error after a newer refresh succeeds", async () => {
    const older = deferred<WorkspaceChangesResult>();
    vi.spyOn(bridgeApi, "workspaceChanges")
      .mockReturnValueOnce(older.promise)
      .mockResolvedValueOnce(changesResult([fileChange("current.ts")]));

    await mount(<ChangesPanel workspace={workspace()} />);
    await mount(<ChangesPanel workspace={workspace({ additions: 285 })} />);
    await act(async () => {
      await new Promise(resolve => setTimeout(resolve, STATS_REFRESH_DEBOUNCE_MS + 40));
    });
    expect(container.textContent).toContain("current.ts");

    await act(async () => {
      older.reject(new Error("stale failure"));
      await older.promise.catch(() => undefined);
    });
    expect(container.textContent).toContain("current.ts");
    expect(container.textContent).not.toContain("stale failure");
  });

  it("invalidates an old request when the workspace changes", async () => {
    const oldWorkspace = deferred<WorkspaceChangesResult>();
    vi.spyOn(bridgeApi, "workspaceChanges")
      .mockReturnValueOnce(oldWorkspace.promise)
      .mockResolvedValueOnce(changesResult([fileChange("second.ts")]));

    await mount(<ChangesPanel workspace={workspace()} />);
    await mount(<ChangesPanel workspace={workspace({ id: "demo-2", title: "Second" })} />);
    expect(container.textContent).toContain("second.ts");

    await act(async () => {
      oldWorkspace.resolve(changesResult([fileChange("first.ts")]));
      await oldWorkspace.promise;
    });
    expect(container.textContent).toContain("second.ts");
    expect(container.textContent).not.toContain("first.ts");
  });

  it("renders non-Git and unborn repository guidance", async () => {
    vi.spyOn(bridgeApi, "workspaceChanges")
      .mockResolvedValueOnce(changesResult([], { repositoryState: "not_git", baseCommit: null }))
      .mockResolvedValueOnce(changesResult([fileChange("first.txt", { changeKind: "added" })], { repositoryState: "unborn", baseCommit: null }));

    await mount(<ChangesPanel workspace={workspace()} />);
    expect(container.textContent).toContain("No Git repository");
    expect(container.textContent).toContain("Changes needs a Git repository");

    await mount(<ChangesPanel workspace={workspace({ id: "unborn", branch: "main" })} />);
    expect(container.textContent).toContain("before first commit");
    expect(container.textContent).toContain("first.txt");
  });

  it("discloses bounded file lists and truncated patches", async () => {
    vi.spyOn(bridgeApi, "workspaceChanges").mockResolvedValue(changesResult([
      fileChange("large.ts", { patchTruncated: true }),
    ], { totalFiles: 12, filesTruncated: true }));

    await mount(<ChangesPanel workspace={workspace()} />);
    expect(container.textContent).toContain("Showing 1 of 12 changed files");
    expect(container.textContent).toContain("0/1 shown viewed");
    await click(container.querySelector("button[aria-expanded]")!);
    expect(container.textContent).toContain("Patch preview truncated");
    expect(container.textContent).toContain("additional diff content exists");
  });

  it("renders every path label on its file row", async () => {
    vi.spyOn(bridgeApi, "workspaceChanges").mockResolvedValue(changesResult([
      fileChange("src/auth/policy.rs", { importance: "high", labels: ["auth", "rust"] }),
    ]));

    await mount(<ChangesPanel workspace={workspace()} />);
    expect(container.querySelector('[title="Path label: auth"]')?.textContent).toBe("auth");
    expect(container.querySelector('[title="Path label: rust"]')?.textContent).toBe("rust");
  });

  it("renders a rename as one old-to-new row", async () => {
    vi.spyOn(bridgeApi, "workspaceChanges").mockResolvedValue(changesResult([
      fileChange("src/new.ts", { previousPath: "src/old.ts", changeKind: "renamed" }),
    ]));

    await mount(<ChangesPanel workspace={workspace()} />);
    expect(container.querySelectorAll("button[aria-expanded]")).toHaveLength(1);
    expect(container.textContent).toContain("src/old.ts → src/new.ts");
    expect(container.textContent).toContain("renamed");
  });

  it("does not offer Edit for a deleted file", async () => {
    vi.spyOn(bridgeApi, "workspaceChanges").mockResolvedValue(changesResult([
      fileChange("removed.ts", { changeKind: "deleted", additions: 0, deletions: 1 }),
    ]));

    await mount(<ChangesPanel workspace={workspace()} />);
    await click(container.querySelector("button[aria-expanded]")!);
    const modeButtons = [...container.querySelectorAll<HTMLButtonElement>("button[aria-pressed]")]
      .map(item => item.textContent);
    expect(modeButtons).not.toContain("edit");
    expect(container.textContent).toContain("old");
  });

  it("keeps importance ordering and reversible low-signal disclosure", async () => {
    await mount(<ChangesPanel workspace={workspace()} />);
    const rows = [...container.querySelectorAll<HTMLButtonElement>("button[aria-expanded]")];
    expect(rows[0].textContent).toContain("src-tauri/bridge-core/src/policy.rs");
    expect(container.textContent).not.toContain("bun.lock");

    const reveal = [...container.querySelectorAll<HTMLButtonElement>("button")]
      .find(item => item.textContent?.includes("low-signal file"))!;
    await click(reveal);
    expect(container.textContent).toContain("bun.lock");
  });

  it("offers a file quote action to its callback", async () => {
    const onQuote = vi.fn();
    await mount(<ChangesPanel workspace={workspace()} onQuote={onQuote} />);
    await click(button("Reference src-tauri/bridge-core/src/policy.rs in the composer")!);
    expect(onQuote).toHaveBeenCalledWith("src-tauri/bridge-core/src/policy.rs");
  });

  it("offers a hunk quote with the range from its own header", async () => {
    const onQuote = vi.fn();
    await mount(<ChangesPanel workspace={workspace()} onQuote={onQuote} />);
    const expander = [...container.querySelectorAll<HTMLButtonElement>("button[aria-expanded]")][0];
    await click(expander);
    const hunkQuote = button("Reference lines 10-30 in the composer")!;
    await click(hunkQuote);
    expect(onQuote).toHaveBeenCalledWith("src-tauri/bridge-core/src/policy.rs", { start: 10, end: 30 });
  });

  it("renders no quote or open affordance without the callbacks", async () => {
    await mount(<ChangesPanel workspace={workspace()} />);
    expect(button("Reference src-tauri/bridge-core/src/policy.rs in the composer")).toBeNull();
    expect(button("Open src-tauri/bridge-core/src/policy.rs in the Code pane")).toBeNull();
  });

  it("hands a file to the Code pane through its callback", async () => {
    const onOpenFile = vi.fn();
    await mount(<ChangesPanel workspace={workspace()} onOpenFile={onOpenFile} />);
    await click(button("Open src-tauri/bridge-core/src/policy.rs in the Code pane")!);
    expect(onOpenFile).toHaveBeenCalledWith("src-tauri/bridge-core/src/policy.rs");
  });
});

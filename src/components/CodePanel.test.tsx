// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { bridgeApi } from "../api";
import { CodePanel } from "./CodePanel";

// CodeMirror owns a real DOM and its own measurement loop, neither of which
// jsdom provides usefully. The document itself is covered by `fileBuffer`
// tests; what this file checks is the panel's wiring around it.
vi.mock("./editor/CodeEditor", () => ({
  CodeEditor: ({ docKey, doc, onChange, onSave }: {
    docKey: string; doc: string; onChange: (value: string) => void; onSave: () => void;
  }) => <textarea
    data-testid="editor"
    data-dockey={docKey}
    defaultValue={doc}
    onChange={event => onChange(event.target.value)}
    onKeyDown={event => { if (event.key === "s") onSave(); }}
  />,
}));

vi.mock("../api", () => ({
  bridgeApi: {
    listWorkspaceTree: vi.fn(),
    readWorkspaceFile: vi.fn(),
    writeWorkspaceFile: vi.fn(),
  },
}));

const tree = vi.mocked(bridgeApi.listWorkspaceTree);
const read = vi.mocked(bridgeApi.readWorkspaceFile);
const write = vi.mocked(bridgeApi.writeWorkspaceFile);

let container: HTMLDivElement;
let root: Root;

const render = async (props: Partial<Parameters<typeof CodePanel>[0]> = {}) => {
  await act(async () => { root.render(<CodePanel workspaceId="w" {...props} />); });
};

const click = async (element: Element | null | undefined) => {
  expect(element, "element to click").toBeTruthy();
  await act(async () => { (element as HTMLElement).click(); });
};

/** React listens for `input` via its own value tracker, so a bare
 *  `element.value = x` is invisible to it. */
const typeInto = async (element: HTMLTextAreaElement, value: string) => {
  const setter = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")!.set!;
  await act(async () => {
    setter.call(element, value);
    element.dispatchEvent(new Event("input", { bubbles: true }));
  });
};

const rows = () => [...container.querySelectorAll("aside button")].filter(node => node.textContent?.trim());
const rowNamed = (name: string) => rows().find(node => node.textContent?.trim() === name);
const text = () => container.textContent ?? "";

beforeEach(() => {
  vi.resetAllMocks();
  tree.mockResolvedValue(["README.md", "src/App.tsx", "src/components/ui/button.tsx"]);
  read.mockResolvedValue({ path: "src/App.tsx", content: "const a = 1;", sha256: "sha1", tooLarge: false, binary: false, sizeBytes: 12 });
  write.mockResolvedValue({ sha256: "sha2" });
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});

afterEach(async () => {
  await act(async () => root.unmount());
  container.remove();
});

describe("CodePanel", () => {
  it("honours a reveal request: the file opens and its tab is active", async () => {
    await render({ reveal: { path: "src/App.tsx", nonce: 1 } });
    const tab = [...container.querySelectorAll("button")].find(node => node.getAttribute("title") === "src/App.tsx");
    expect(tab).toBeTruthy();
    expect(read).toHaveBeenCalledWith("w", "src/App.tsx");
  });

  it("fires one activation per reveal nonce", async () => {
    read.mockImplementation(async (_workspace, path) => ({ path, content: "const a = 1;", sha256: `sha-${path}`, tooLarge: false, binary: false, sizeBytes: 12 }));
    await render({ reveal: { path: "src/App.tsx", nonce: 1 } });
    expect(read).toHaveBeenCalledTimes(1);

    await click(rowNamed("README.md"));
    const activeTab = () => [...container.querySelectorAll('[class*="group/tab"]')].find(node => node.className.includes("bg-code"))?.querySelector("button[title]")?.getAttribute("title");
    expect(activeTab()).toBe("README.md");

    await render({ reveal: { path: "src/App.tsx", nonce: 1 } });
    expect(activeTab()).toBe("README.md");

    await render({ reveal: { path: "src/App.tsx", nonce: 2 } });
    expect(activeTab()).toBe("src/App.tsx");
  });

  it("builds a tree from the workspace file list, directories first", async () => {
    await render();
    expect(rows().map(node => node.textContent?.trim())).toEqual(["src", "README.md"]);
  });

  it("opens a file into a tab and shows it as saved", async () => {
    await render();
    await click(rowNamed("src"));
    await click(rowNamed("App.tsx"));
    expect(read).toHaveBeenCalledWith("w", "src/App.tsx");
    expect(container.querySelector("[data-testid=editor]")).toBeTruthy();
    expect(text()).toContain("src/App.tsx");
    expect(text()).toContain("Saved");
  });

  it("collapses single-child directory chains", async () => {
    tree.mockResolvedValue(["src/components/ui/button.tsx"]);
    await render();
    expect(rows().map(node => node.textContent?.trim())).toEqual(["src/components/ui"]);
  });

  it("marks a buffer unsaved on edit and writes it with the hash it was read at", async () => {
    await render();
    await click(rowNamed("src"));
    await click(rowNamed("App.tsx"));
    await typeInto(container.querySelector<HTMLTextAreaElement>("[data-testid=editor]")!, "const a = 2;");
    expect(text()).toContain("Unsaved changes");

    await click([...container.querySelectorAll("button")].find(node => node.textContent?.includes("Save")));
    expect(write).toHaveBeenCalledWith("w", "src/App.tsx", "const a = 2;", "sha1");
    expect(text()).toContain("Saved");
  });

  it("offers reload and overwrite when the file changed on disk", async () => {
    write.mockRejectedValue(new Error("src/App.tsx changed on disk since it was opened"));
    await render();
    await click(rowNamed("src"));
    await click(rowNamed("App.tsx"));
    await typeInto(container.querySelector<HTMLTextAreaElement>("[data-testid=editor]")!, "mine");
    await click([...container.querySelectorAll("button")].find(node => node.textContent?.includes("Save")));

    expect(text()).toContain("changed on disk");
    const actions = [...container.querySelectorAll("button")].map(node => node.textContent);
    expect(actions).toContain("Reload");
    expect(actions).toContain("Overwrite");
  });

  it("reports a read failure instead of opening an empty document", async () => {
    read.mockRejectedValue(new Error("src/App.tsx is not a file"));
    await render();
    await click(rowNamed("src"));
    await click(rowNamed("App.tsx"));
    expect(text()).toContain("src/App.tsx is not a file");
    expect(container.querySelector("[data-testid=editor]")).toBeNull();
  });

  it("refuses to open a binary file for editing", async () => {
    read.mockResolvedValue({ path: "src/App.tsx", content: "", sha256: "s", tooLarge: false, binary: true, sizeBytes: 9 });
    await render();
    await click(rowNamed("src"));
    await click(rowNamed("App.tsx"));
    expect(text()).toContain("binary");
    expect(container.querySelector("[data-testid=editor]")).toBeNull();
  });

  it("surfaces a tree listing failure", async () => {
    tree.mockRejectedValue(new Error("Workspace has no folder connected"));
    await render();
    expect(text()).toContain("Workspace has no folder connected");
  });

  it("does not let a second open of one path reset what you already typed", async () => {
    // Two opens in flight for the same path: tree click, then ⌘P. The slower
    // load must not put the on-disk text back over the live buffer.
    let release: (value: unknown) => void = () => undefined;
    const slow = new Promise(resolve => { release = resolve; });
    read.mockImplementation(async () => {
      await slow;
      return { path: "src/App.tsx", content: "const a = 1;", sha256: "sha1", tooLarge: false, binary: false, sizeBytes: 12 };
    });
    await render();
    await click(rowNamed("src"));
    await act(async () => { (rowNamed("App.tsx") as HTMLElement).click(); });
    await act(async () => { (rowNamed("App.tsx") as HTMLElement).click(); });
    await act(async () => { release(undefined); await slow; });
    expect(read).toHaveBeenCalledTimes(1);

    await typeInto(container.querySelector<HTMLTextAreaElement>("[data-testid=editor]")!, "typed by hand");
    await click([...container.querySelectorAll("button")].find(node => node.textContent?.includes("Save")));
    expect(write).toHaveBeenCalledWith("w", "src/App.tsx", "typed by hand", "sha1");
  });

  it("drops a reload that finishes after its tab was closed", async () => {
    // The reachable version of the stale-load race: the tab exists, a conflict
    // reload is in flight, and the tab is closed before it lands.
    write.mockRejectedValue(new Error("src/App.tsx changed on disk since it was opened"));
    await render();
    await click(rowNamed("src"));
    await click(rowNamed("App.tsx"));
    await typeInto(container.querySelector<HTMLTextAreaElement>("[data-testid=editor]")!, "mine");
    await click([...container.querySelectorAll("button")].find(node => node.textContent?.includes("Save")));

    let release: (value: unknown) => void = () => undefined;
    const slow = new Promise(resolve => { release = resolve; });
    read.mockImplementation(async () => {
      await slow;
      return { path: "src/App.tsx", content: "theirs", sha256: "sha9", tooLarge: false, binary: false, sizeBytes: 6 };
    });
    await click([...container.querySelectorAll("button")].find(node => node.textContent === "Reload"));
    await click([...container.querySelectorAll("button")].find(node => node.getAttribute("aria-label")?.startsWith("Close")));
    await act(async () => { release(undefined); await slow; });
    expect(container.querySelector("[data-testid=editor]")).toBeNull();
    expect(text()).not.toContain("theirs");
  });

  it("does not claim ⌘P while another tab is showing", async () => {
    await render({ visible: false });
    await act(async () => {
      window.dispatchEvent(new KeyboardEvent("keydown", { key: "p", metaKey: true, bubbles: true }));
    });
    expect(container.querySelector("input[aria-label='Go to file']")).toBeNull();
  });
});

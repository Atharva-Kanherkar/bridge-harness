// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { bridgeApi } from "../api";
import { CodePanel, OPEN_FILE_PROBE_MS } from "./CodePanel";
import { STATS_REFRESH_DEBOUNCE_MS } from "./ChangesPanel";

// CodeMirror owns a real DOM and its own measurement loop, neither of which
// jsdom provides usefully. The document itself is covered by `fileBuffer`
// tests; what this file checks is the panel's wiring around it.
vi.mock("./editor/CodeEditor", async () => {
  const { useEffect, useRef } = await vi.importActual<typeof import("react")>("react");
  return {
    CodeEditor: function MockEditor({ docKey, doc, path, revealLine, syncDoc, onChange, onSave }: {
      docKey: string; doc: string; path: string; revealLine?: { line: number; nonce: number };
      syncDoc?: { text: string; version: number }; onChange: (value: string) => void; onSave: () => void;
    }) {
      const node = useRef<HTMLTextAreaElement>(null);
      const seen = useRef(syncDoc?.version ?? 0);
      // The real editor applies `syncDoc` in place and reports the change.
      useEffect(() => {
        if (!syncDoc || syncDoc.version === seen.current || !node.current) return;
        seen.current = syncDoc.version;
        if (node.current.value === syncDoc.text) return;
        node.current.value = syncDoc.text;
        onChange(syncDoc.text);
      }, [syncDoc, onChange]);
      return <textarea
        ref={node}
        data-testid="editor"
        data-path={path}
        data-dockey={docKey}
        data-revealline={revealLine ? `${revealLine.line}:${revealLine.nonce}` : undefined}
        defaultValue={doc}
        onChange={event => onChange(event.target.value)}
        onKeyDown={event => { if (event.key === "s") onSave(); }}
      />;
    },
  };
});

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
  vi.restoreAllMocks();
  vi.useRealTimers();
});


const settleDrift = async () => {
  await act(async () => {
    await new Promise(resolve => setTimeout(resolve, STATS_REFRESH_DEBOUNCE_MS + 40));
  });
};

describe("CodePanel", () => {
  it("draws depth as indent guides rather than computed padding", async () => {
    // C15. Guides *are* the indent, so a row at depth n has exactly n of
    // them; padding alone gave a deep tree no visible trunk to hang off.
    await render();
    const guides = (name: string) =>
      rowNamed(name)!.querySelectorAll("span[aria-hidden].border-r").length;
    expect(guides("README.md")).toBe(0);
    expect(guides("src")).toBe(0);
    await click(rowNamed("src"));
    expect(guides("App.tsx")).toBe(1);
    expect(guides("components/ui")).toBe(1);
    await click(rowNamed("components/ui"));
    expect(guides("button.tsx")).toBe(2);
    // And no row falls back to an inline padding offset.
    for (const row of rows()) expect((row as HTMLElement).style.paddingLeft).toBe("");
  });

  it("gives a directory a folder glyph that follows its expanded state", async () => {
    // C13/C16. The glyph is the only cue that distinguishes a directory from
    // a file at the same depth, so it has to actually change on toggle.
    await render();
    const glyph = (name: string) => rowNamed(name)!.querySelector("svg:not(.lucide-chevron-right)")?.getAttribute("class") ?? "";
    const collapsed = glyph("src");
    await click(rowNamed("src"));
    expect(glyph("src")).not.toBe(collapsed);
    // A file's glyph is tinted from the syntax ramp; a directory's is not.
    expect(glyph("App.tsx")).toMatch(/text-syn-/);
    expect(glyph("src")).not.toMatch(/text-syn-/);
    // Every glyph is decorative — the row's text is the accessible name.
    for (const svg of rowNamed("App.tsx")!.querySelectorAll("svg")) {
      expect(svg.getAttribute("aria-hidden")).toBe("true");
    }
  });

  it("marks only directories as expandable", async () => {
    // C17, restated: the glyph work must not put aria-expanded on files.
    await render();
    expect(rowNamed("src")!.getAttribute("aria-expanded")).toBe("false");
    expect(rowNamed("README.md")!.hasAttribute("aria-expanded")).toBe(false);
    await click(rowNamed("src"));
    expect(rowNamed("src")!.getAttribute("aria-expanded")).toBe("true");
  });

  it("passes a reveal line to the active file's editor, and only there", async () => {
    await render({ reveal: { path: "src/App.tsx", line: 42, nonce: 7 } });
    const editor = () => container.querySelector<HTMLTextAreaElement>('[data-testid="editor"]');
    expect(editor()!.getAttribute("data-revealline")).toBe("42:7");

    read.mockResolvedValue({ path: "README.md", content: "# readme", sha256: "sha-r", tooLarge: false, binary: false, sizeBytes: 8 });
    await click(rowNamed("README.md"));
    expect(editor()!.getAttribute("data-revealline")).toBeNull();
  });

  it("passes no reveal line when the request has none", async () => {
    await render({ reveal: { path: "src/App.tsx", nonce: 1 } });
    expect(container.querySelector('[data-testid="editor"]')!.getAttribute("data-revealline")).toBeNull();
  });

  it("reloads a clean open buffer when the disk moves under it", async () => {
    await render({ reveal: { path: "src/App.tsx", nonce: 1 }, driftSignal: "a" });
    expect(container.querySelector<HTMLTextAreaElement>('[data-testid="editor"]')!.defaultValue).toBe("const a = 1;");

    read.mockResolvedValue({ path: "src/App.tsx", content: "const a = 2;", sha256: "sha-agent", tooLarge: false, binary: false, sizeBytes: 12 });
    const keyBefore = container.querySelector('[data-testid="editor"]')!.getAttribute("data-dockey");
    await render({ reveal: { path: "src/App.tsx", nonce: 1 }, driftSignal: "b" });
    await settleDrift();
    const editor = container.querySelector<HTMLTextAreaElement>('[data-testid="editor"]')!;
    expect(editor.value).toBe("const a = 2;");
    // In place: the same editor instance, so caret, scroll and undo survive.
    expect(editor.getAttribute("data-dockey")).toBe(keyBefore);
    expect(text()).toContain("Saved");
    expect(text()).not.toContain("Changed on disk");
  });

  it("turns a dirty open buffer to conflict instead of clobbering it", async () => {
    await render({ reveal: { path: "src/App.tsx", nonce: 1 }, driftSignal: "a" });
    await typeInto(container.querySelector('[data-testid="editor"]')!, "my unsaved edit");

    read.mockResolvedValue({ path: "src/App.tsx", content: "agent version", sha256: "sha-agent", tooLarge: false, binary: false, sizeBytes: 13 });
    await render({ reveal: { path: "src/App.tsx", nonce: 1 }, driftSignal: "b" });
    await settleDrift();
    expect(text()).toContain("Changed on disk while you were editing");
    const editor = container.querySelector<HTMLTextAreaElement>('[data-testid="editor"]')!;
    expect(editor.value).toBe("my unsaved edit");
    expect([...container.querySelectorAll("button")].some(button => button.textContent === "Reload")).toBe(true);
    expect([...container.querySelectorAll("button")].some(button => button.textContent === "Overwrite")).toBe(true);
  });

  it("leaves an unchanged file alone on drift", async () => {
    await render({ reveal: { path: "src/App.tsx", nonce: 1 }, driftSignal: "a" });
    const keyBefore = container.querySelector('[data-testid="editor"]')!.getAttribute("data-dockey");
    await render({ reveal: { path: "src/App.tsx", nonce: 1 }, driftSignal: "b" });
    await settleDrift();
    expect(container.querySelector('[data-testid="editor"]')!.getAttribute("data-dockey")).toBe(keyBefore);
    expect(text()).not.toContain("Changed on disk");
  });

  it("issues no reads on drift with nothing open", async () => {
    await render({ driftSignal: "a" });
    read.mockClear();
    await render({ driftSignal: "b" });
    await settleDrift();
    expect(read).not.toHaveBeenCalled();
  });

  it("honours a reveal request: the file opens and its tab is active", async () => {
    await render({ reveal: { path: "src/App.tsx", nonce: 1 } });
    const tab = [...container.querySelectorAll("button")].find(node => node.getAttribute("data-tip") === "src/App.tsx");
    expect(tab).toBeTruthy();
    expect(read).toHaveBeenCalledWith("w", "src/App.tsx");
  });

  it("fires one activation per reveal nonce", async () => {
    read.mockImplementation(async (_workspace, path) => ({ path, content: "const a = 1;", sha256: `sha-${path}`, tooLarge: false, binary: false, sizeBytes: 12 }));
    await render({ reveal: { path: "src/App.tsx", nonce: 1 } });
    expect(read).toHaveBeenCalledTimes(1);

    await click(rowNamed("README.md"));
    const activeTab = () => [...container.querySelectorAll('[class*="group/tab"]')].find(node => node.className.includes("bg-code"))?.querySelector("button[data-tip]")?.getAttribute("data-tip");
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
    vi.spyOn(window, "confirm").mockReturnValue(true);
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

  const editorFor = (path: string) => container.querySelector<HTMLTextAreaElement>(`[data-testid="editor"][data-path="${path}"]`)!;
  const button = (label: string) => [...container.querySelectorAll("button")].find(node => node.textContent === label || node.textContent?.startsWith(label));
  const readFor = () => read.mockImplementation(async (_workspace, path) => ({ path, content: `disk ${path}`, sha256: `sha-${path}`, tooLarge: false, binary: false, sizeBytes: 9 }));

  it("keeps unsaved text on screen across a tab switch", async () => {
    // One editor re-seeded per switch showed the last *saved* text, while ⌘S
    // still wrote the hidden edit.
    readFor();
    await render({ reveal: { path: "src/App.tsx", nonce: 1 } });
    await typeInto(editorFor("src/App.tsx"), "unsaved work");
    await click(rowNamed("README.md"));
    await click([...container.querySelectorAll("button[data-tip='src/App.tsx']")][0]);
    expect(editorFor("src/App.tsx").value).toBe("unsaved work");
    expect(text()).toContain("Unsaved changes");
  });

  it("stays unsaved when you type while a save is in flight", async () => {
    let finish: (value: { sha256: string }) => void = () => undefined;
    write.mockImplementation(() => new Promise(resolve => { finish = resolve; }));
    await render({ reveal: { path: "src/App.tsx", nonce: 1 } });
    await typeInto(editorFor("src/App.tsx"), "first");
    await click(button("Save"));
    await typeInto(editorFor("src/App.tsx"), "first and more");
    await act(async () => { finish({ sha256: "sha2" }); });
    expect(write).toHaveBeenCalledWith("w", "src/App.tsx", "first", "sha1");
    expect(text()).toContain("Unsaved changes");
    expect(container.querySelector("[aria-label*='discarding unsaved changes']")).toBeTruthy();
  });

  it("writes once when two saves land in the same tick", async () => {
    await render({ reveal: { path: "src/App.tsx", nonce: 1 } });
    await typeInto(editorFor("src/App.tsx"), "edit");
    const save = button("Save") as HTMLElement;
    await act(async () => { save.click(); save.click(); });
    expect(write).toHaveBeenCalledTimes(1);
    expect(text()).not.toContain("changed on disk");
  });

  it("does not rewrite a clean file on ⌘S", async () => {
    await render({ reveal: { path: "src/App.tsx", nonce: 1 } });
    await act(async () => {
      window.dispatchEvent(new KeyboardEvent("keydown", { key: "s", metaKey: true, bubbles: true }));
    });
    expect(write).not.toHaveBeenCalled();
  });

  it("still counts a failed save as unsaved work", async () => {
    write.mockRejectedValue(new Error("permission denied"));
    await render({ reveal: { path: "src/App.tsx", nonce: 1 } });
    await typeInto(editorFor("src/App.tsx"), "mine");
    await click(button("Save"));
    expect(text()).toContain("permission denied");
    expect(container.querySelector("[aria-label*='discarding unsaved changes']")).toBeTruthy();
    expect(button("Save")).toBeTruthy();
  });

  it("asks before closing a tab with unsaved changes", async () => {
    await render({ reveal: { path: "src/App.tsx", nonce: 1 } });
    await typeInto(editorFor("src/App.tsx"), "mine");
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(false);
    await click(container.querySelector("[aria-label^='Close src/App.tsx']"));
    expect(confirm).toHaveBeenCalledOnce();
    expect(editorFor("src/App.tsx").value).toBe("mine");

    confirm.mockReturnValue(true);
    await click(container.querySelector("[aria-label^='Close src/App.tsx']"));
    expect(container.querySelector("[data-testid=editor]")).toBeNull();
  });

  it("closes a clean tab without asking and focuses its neighbour", async () => {
    readFor();
    tree.mockResolvedValue(["a.ts", "b.ts", "c.ts"]);
    const confirm = vi.spyOn(window, "confirm");
    await render();
    await click(rowNamed("a.ts"));
    await click(rowNamed("b.ts"));
    await click(rowNamed("c.ts"));
    await click([...container.querySelectorAll("button[data-tip='b.ts']")][0]);
    await click(container.querySelector("[aria-label='Close b.ts']"));
    expect(confirm).not.toHaveBeenCalled();
    expect(editorFor("c.ts").parentElement!.className).not.toContain("hidden");
  });

  it("still probes when a tab changes inside the drift debounce", async () => {
    // The probe effect used to depend on the tab list: any tab update in the
    // debounce window cancelled the timer, and the signal was already spent.
    await render({ reveal: { path: "src/App.tsx", nonce: 1 }, driftSignal: "a" });
    read.mockResolvedValue({ path: "src/App.tsx", content: "agent wrote", sha256: "sha-agent", tooLarge: false, binary: false, sizeBytes: 11 });
    await render({ reveal: { path: "src/App.tsx", nonce: 1 }, driftSignal: "b" });
    await typeInto(editorFor("src/App.tsx"), "typed during the debounce");
    await settleDrift();
    expect(text()).toContain("Changed on disk while you were editing");
  });

  it("picks up files the agent created when the stats drift", async () => {
    await render({ driftSignal: "a" });
    tree.mockResolvedValue(["README.md", "NEW.md", "src/App.tsx", "src/components/ui/button.tsx"]);
    await render({ driftSignal: "b" });
    await settleDrift();
    expect(rowNamed("NEW.md")).toBeTruthy();
  });

  it("re-reads open files on a slow clock while on screen, even with no drift", async () => {
    // "+1 −1" stays "+1 −1" when an agent rewrites an already-changed line.
    vi.useFakeTimers();
    await render({ reveal: { path: "src/App.tsx", nonce: 1 } });
    read.mockResolvedValue({ path: "src/App.tsx", content: "rewritten", sha256: "sha-new", tooLarge: false, binary: false, sizeBytes: 9 });
    await act(async () => { await vi.advanceTimersByTimeAsync(OPEN_FILE_PROBE_MS + 10); });
    expect(editorFor("src/App.tsx").value).toBe("rewritten");
  });

  it("does not poll while hidden", async () => {
    vi.useFakeTimers();
    await render({ reveal: { path: "src/App.tsx", nonce: 1 }, visible: false });
    read.mockClear();
    await act(async () => { await vi.advanceTimersByTimeAsync(OPEN_FILE_PROBE_MS * 3); });
    expect(read).not.toHaveBeenCalled();
  });

  it("offers the file list as a drawer when the pane is too narrow for the sidebar", async () => {
    await render();
    const toggle = container.querySelector<HTMLButtonElement>("[aria-label='Show file list']")!;
    expect(toggle.getAttribute("aria-expanded")).toBe("false");
    await click(button("Browse files"));
    expect(toggle.getAttribute("aria-expanded")).toBe("true");
    expect(container.querySelector("aside")!.className).toContain("absolute");
    await click(rowNamed("README.md"));
    expect(toggle.getAttribute("aria-expanded")).toBe("false");
  });

  it("shows the size notice for a file too large to edit", async () => {
    read.mockResolvedValue({ path: "src/App.tsx", content: "", sha256: "", tooLarge: true, binary: false, sizeBytes: 3 * 1024 * 1024 });
    await render({ reveal: { path: "src/App.tsx", nonce: 1 } });
    expect(text()).toContain("too large to open in the editor");
    expect(container.querySelector("[data-testid=editor]")).toBeNull();
  });
});

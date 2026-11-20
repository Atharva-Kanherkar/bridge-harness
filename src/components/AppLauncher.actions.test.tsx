// @vitest-environment jsdom
// Contract: testing/feat-t3-inspired-chat-ui.md §G. Typing in the launcher
// finds the keymap's commands, each with its real chord, and runs them through
// the dispatcher the chord uses.
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { Code2, FileCode2, ListChecks, Monitor, TerminalSquare } from "lucide-react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { AppLauncher, LAUNCHER_COMMANDS, LAUNCHER_SKIPPED, launcherSections, type AppLauncherProps, type LauncherDockPane } from "./AppLauncher";
import { type PinResult } from "../apps/pins";
import { BUILTIN_APPS } from "../apps/registry";
import { DOCK_PANES } from "../dockLayout";
import { chordLabel, matchShortcut, SHORTCUTS, type KeyStroke } from "../keymap";

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(async () => {
  await act(async () => root.unmount());
  container.remove();
  document.body.innerHTML = "";
});

const chats = [
  { id: "c1", title: "Fix the deploy script", detail: "infra" },
  { id: "c2", title: "Polish the Deck shell", detail: "bridge" },
];

// Strip order, the way App hands it over: Work first, past the digit chords.
const dockPanes: LauncherDockPane[] = [
  { id: "work", label: "Work", icon: ListChecks },
  { id: "changes", label: "Changes", icon: FileCode2 },
  { id: "code", label: "Code", icon: Code2 },
  { id: "terminal", label: "Terminal", icon: TerminalSquare },
  { id: "browser", label: "Browser", icon: Monitor },
];

function props(overrides: Partial<AppLauncherProps> = {}): AppLauncherProps {
  return {
    open: true,
    onClose: vi.fn(),
    apps: BUILTIN_APPS,
    badges: {},
    pins: [],
    chats,
    chatOpen: true,
    onOpenApp: vi.fn(),
    onOpenChat: vi.fn(),
    onSearchThisChat: vi.fn(),
    onSearchChats: vi.fn(),
    onTogglePin: vi.fn(async (): Promise<PinResult> => ({ ok: true })),
    onRunCommand: vi.fn(),
    dockPanes,
    numberedChats: chats,
    ...overrides,
  };
}

async function mount(overrides: Partial<AppLauncherProps> = {}) {
  const value = props(overrides);
  await act(async () => { root.render(<AppLauncher {...value} />); });
  return value;
}

const input = () => document.querySelector<HTMLInputElement>('input[role="combobox"]')!;
const sections = () => [...document.querySelectorAll("[data-launcher-section]")].map(node => node.getAttribute("data-launcher-section"));
const section = (id: string) => document.querySelector(`[data-launcher-section="${id}"]`);
const selected = () => document.querySelector('[role="option"][aria-selected="true"]');
const rows = (id: string) => [...(section(id)?.querySelectorAll('[role="option"]') ?? [])].map(node => node.textContent);

async function type(value: string) {
  const setter = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, "value")!.set!;
  await act(async () => {
    setter.call(input(), value);
    input().setSelectionRange(value.length, value.length);
    input().dispatchEvent(new Event("input", { bubbles: true }));
  });
}

async function press(key: string, init: KeyboardEventInit = {}) {
  await act(async () => {
    input().dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true, ...init }));
  });
}

async function moveTo(text: string) {
  for (let step = 0; step < 40 && !selected()?.textContent?.startsWith(text); step++) await press("ArrowDown");
  expect(selected()?.textContent).toContain(text);
}

describe("launcher actions", () => {
  it("lists matching actions with their chords once something is typed", async () => {
    await mount();
    expect(sections(), "nothing typed, no actions").not.toContain("commands");
    await type("dock");
    expect(section("commands")?.querySelector("p")?.textContent).toBe("Actions");
    expect(rows("commands")).toEqual([
      `Toggle the dock${chordLabel("toggle-dock")}`,
      `Expand the dock${chordLabel("expand-dock")}`,
      `Open dock pane${"Work, Changes, Code, Terminal, Browser"}${chordLabel("open-dock-pane")}`,
    ]);
    expect(chordLabel("toggle-dock")).toBe("⌥⌘0");
    expect(chordLabel("open-dock-pane")).toBe("⌥⌘1-9");
    // The sheet lists those chords too, and says where.
    expect(rows("help")).toEqual([`Keyboard shortcutsView › Toggle the dock${chordLabel("show-shortcuts")}`]);
    expect(sections().at(-1)).toBe("help");
  });

  it("runs the highlighted action through the prop and closes", async () => {
    const value = await mount();
    await type("sidebar");
    expect(selected()?.textContent).toContain("Toggle the sidebar");
    await press("Enter");
    expect(value.onRunCommand).toHaveBeenCalledWith("toggle-sidebar", undefined);
    expect(value.onClose).toHaveBeenCalled();
  });

  it("runs an action on click", async () => {
    const value = await mount();
    await type("settings");
    const row = document.querySelector<HTMLElement>('[data-command="open-settings"]')!;
    await act(async () => { row.click(); });
    expect(value.onRunCommand).toHaveBeenCalledWith("open-settings", undefined);
    expect(value.onClose).toHaveBeenCalled();
  });

  it("keeps an app ahead of an action, so an app's name and ↵ still opens it", async () => {
    const value = await mount();
    await type("project");
    expect(sections().indexOf("apps")).toBeLessThan(sections().indexOf("commands"));
    expect(rows("commands")).toContain(`New project${chordLabel("new-project")}`);
    await press("Enter");
    expect(value.onOpenApp).toHaveBeenCalledWith("projects");
    expect(value.onRunCommand).not.toHaveBeenCalled();
  });

  it("finds commands from the start of a word, by label or keyword", () => {
    const find = (query: string) => launcherSections({ query, apps: [], badges: {}, chats: [], chatOpen: true, actions: { dockPanes, numberedChats: chats } })
      .find(item => item.id === "commands")?.items.map(item => (item.kind === "command" ? item.action.id : ""));
    expect(find("stop")).toEqual(["interrupt-turn"]);
    expect(find("the dock")).toEqual(["toggle-dock", "expand-dock"]);
    // Inside a word is not a start of one.
    expect(find("ock")).toBeUndefined();
  });

  it("lists the dock and the turn only while a chat is on screen", async () => {
    await mount({ chatOpen: false });
    await type("dock");
    expect(section("commands")).toBeNull();
    await type("stop");
    expect(section("commands")).toBeNull();
  });

  it("leaves the resting layout exactly as it was", async () => {
    const base = { query: "", apps: BUILTIN_APPS, badges: { workflows: { count: 2 } }, chats, chatOpen: true };
    const ids = (value: ReturnType<typeof launcherSections>) => value.map(item => `${item.id}:${item.items.map(entry => entry.key).join(",")}`);
    expect(ids(launcherSections({ ...base, actions: { dockPanes, numberedChats: chats } }))).toEqual(ids(launcherSections(base)));
    await mount({ badges: { workflows: { count: 2 } } });
    expect(sections()).toEqual(["needs", "apps", "chats", "actions"]);
    expect(document.body.textContent).not.toContain("Toggle the dock");
  });

  it("keeps Search this chat and the search across chats beside the actions", async () => {
    await mount();
    await type("search");
    expect(rows("actions")).toEqual([`Search this chat${chordLabel("open-recall")}`, "Search all chats for “search”"]);
  });

  it("teaches the keys in the footer", async () => {
    await mount();
    const footer = document.querySelector('[role="status"]')!.parentElement!.textContent;
    for (const hint of ["↑↓Move", "↵Run", "→Open submenu", "⌫Back", `${chordLabel("open-launcher")}Close`]) expect(footer).toContain(hint);
    // ⌘P pins an app, so it is taught while an app is listed.
    expect(footer).toContain("Pin to sidebar");
    await type("dock");
    expect(document.querySelector('[role="status"]')!.parentElement!.textContent).not.toContain("Pin to sidebar");
  });
});

describe("launcher submenus", () => {
  it("opens Open dock pane with →, lists the panes with their chords, and runs one", async () => {
    const value = await mount();
    await type("dock");
    await moveTo("Open dock pane");
    await press("ArrowRight");
    expect(value.onRunCommand).not.toHaveBeenCalled();
    expect(sections()).toEqual(["submenu"]);
    expect(input().value).toBe("");
    expect(input().getAttribute("aria-label")).toBe("Search Open dock pane");
    // Strip order; the chord is the pane's place in DOCK_PANES, and Work,
    // past the ninth, has none.
    expect(rows("submenu")).toEqual(["Work", "Changes⌥⌘1", "Code⌥⌘2", "Terminal⌥⌘3", "Browser⌥⌘4"]);
    await type("term");
    await press("Enter");
    expect(value.onRunCommand).toHaveBeenCalledWith("open-dock-pane", DOCK_PANES.indexOf("terminal"));
    expect(value.onClose).toHaveBeenCalled();
  });

  it("opens Work, which no digit reaches, by its index", async () => {
    const value = await mount();
    await type("pane");
    await press("Enter");
    expect(sections()).toEqual(["submenu"]);
    expect(selected()?.textContent).toBe("Work");
    await press("Enter");
    expect(value.onRunCommand).toHaveBeenCalledWith("open-dock-pane", DOCK_PANES.indexOf("work"));
  });

  it("goes back with ⌫ on an empty box, restoring the query and the row", async () => {
    const value = await mount();
    await type("dock");
    await moveTo("Open dock pane");
    await press("ArrowRight");
    await press("ArrowDown");
    await press("Backspace");
    expect(input().value).toBe("dock");
    expect(sections()).toContain("commands");
    expect(selected()?.textContent).toContain("Open dock pane");
    expect(value.onClose).not.toHaveBeenCalled();
  });

  it("goes back with ← from the start of the box, and ⌫ still edits a typed filter", async () => {
    await mount();
    await type("dock");
    await moveTo("Open dock pane");
    await press("ArrowRight");
    await type("co");
    const backspace = new KeyboardEvent("keydown", { key: "Backspace", bubbles: true, cancelable: true });
    await act(async () => { input().dispatchEvent(backspace); });
    expect(backspace.defaultPrevented, "a typed filter is edited, not left").toBe(false);
    expect(sections()).toEqual(["submenu"]);
    await act(async () => { input().setSelectionRange(0, 0); });
    await press("ArrowLeft");
    expect(input().value).toBe("dock");
    expect(sections()).toContain("commands");
  });

  it("lands on the pane a query named", async () => {
    await mount();
    await type("terminal");
    const parent = document.querySelector('[data-command="open-dock-pane"]')!;
    expect(parent.textContent).toBe(`Open dock paneTerminal${chordLabel("open-dock-pane")}`);
    await moveTo("Open dock pane");
    await press("ArrowRight");
    expect(selected()?.textContent).toBe("Terminal⌥⌘3");
  });

  it("opens a submenu on click and keeps the launcher open, the box focused", async () => {
    const value = await mount();
    await type("dock");
    await act(async () => { document.querySelector<HTMLElement>('[data-command="open-dock-pane"]')!.click(); });
    expect(sections()).toEqual(["submenu"]);
    expect(value.onClose).not.toHaveBeenCalled();
    expect(document.activeElement).toBe(input());
    await act(async () => { document.querySelector<HTMLButtonElement>('button[aria-label="Back from Open dock pane"]')!.click(); });
    expect(input().value).toBe("dock");
  });

  it("lists Jump to chat with the digit each chat answers to", async () => {
    const value = await mount();
    await type("jump");
    await moveTo("Jump to chat");
    await press("ArrowRight");
    expect(rows("submenu")).toEqual([`Fix the deploy scriptinfra${chordLabel("jump-to-chat", 1)}`, `Polish the Deck shellbridge${chordLabel("jump-to-chat", 2)}`]);
    expect(chordLabel("jump-to-chat", 2)).toBe("⌘2");
    await press("ArrowDown");
    await press("Enter");
    expect(value.onRunCommand).toHaveBeenCalledWith("jump-to-chat", 1);
  });

  it("hides a submenu with nothing in it", async () => {
    await mount({ dockPanes: [], numberedChats: [] });
    await type("pane");
    expect(document.querySelector('[data-command="open-dock-pane"]')).toBeNull();
    await type("chat");
    expect(document.querySelector('[data-command="jump-to-chat"]')).toBeNull();
    expect(document.querySelector('[data-command="next-chat"]'), "nowhere to step to").toBeNull();
  });

  it("says when nothing in a submenu matches", async () => {
    await mount();
    await type("pane");
    await press("ArrowRight");
    await type("zzz");
    expect(document.body.textContent).toContain("No pane matches “zzz”.");
  });
});

// The launcher lists commands; the keymap declares them and App's dispatcher
// runs them. A command added to one and not the others is the drift this
// guards: a row that runs nothing, or a command the launcher silently forgot.
describe("launcher actions against the keymap and the dispatcher", () => {
  const listed = LAUNCHER_COMMANDS.map(command => command.id as string);
  const skipped = Object.keys(LAUNCHER_SKIPPED);

  it("lists only real keymap commands, and accounts for every one", () => {
    const declared = SHORTCUTS.map(shortcut => shortcut.id as string);
    for (const id of listed) expect(declared, `${id} is a keymap command`).toContain(id);
    expect([...listed, ...skipped].sort()).toEqual([...declared].sort());
    expect(listed.filter(id => skipped.includes(id))).toEqual([]);
    expect(skipped).toContain("open-launcher");
  });

  it("lists only commands App's dispatcher handles, and hands the launcher that dispatcher", () => {
    const source = readFileSync(join(process.cwd(), "src/App.tsx"), "utf8");
    const start = source.indexOf("commandRef.current = (id, index) => {");
    expect(start, "App's command dispatcher").toBeGreaterThan(0);
    const body = source.slice(start, source.indexOf("\n  };\n", start));
    const handled = new Set([...body.matchAll(/case "([a-z-]+)":/g)].map(match => match[1]));
    for (const id of listed) expect(handled.has(id), `${id} has a case in the dispatcher`).toBe(true);
    expect(source).toMatch(/onRunCommand=\{\(id, index\) => commandRef\.current\(id, index\)\}/);
  });

  it("claims no key a keymap chord answers to", () => {
    // ↑↓ ← → ↵ ⌫ are the launcher's own, local to its box. None may also be a
    // command, or the window dispatcher would answer it first.
    const stroke = (key: string): KeyStroke => ({ key, code: key, metaKey: false, ctrlKey: false, altKey: false, shiftKey: false });
    for (const key of ["ArrowUp", "ArrowDown", "ArrowLeft", "ArrowRight", "Enter", "Backspace"]) {
      expect(matchShortcut(stroke(key)), key).toBeUndefined();
      expect(matchShortcut(stroke(key), true), `${key} while typing`).toBeUndefined();
    }
  });
});

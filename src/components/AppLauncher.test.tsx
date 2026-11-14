// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { AppLauncher, launcherSections, type AppLauncherProps } from "./AppLauncher";
import { PIN_LIMIT_MESSAGE, type PinResult } from "../apps/pins";
import type { AppId } from "../apps/registry";

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

function props(overrides: Partial<AppLauncherProps> = {}): AppLauncherProps {
  return {
    open: true,
    onClose: vi.fn(),
    badges: {},
    pins: [],
    chats,
    chatOpen: false,
    onOpenApp: vi.fn(),
    onOpenChat: vi.fn(),
    onSearchThisChat: vi.fn(),
    onSearchChats: vi.fn(),
    onTogglePin: vi.fn((): PinResult => ({ ok: true, pins: [] })),
    ...overrides,
  };
}

async function mount(overrides: Partial<AppLauncherProps> = {}) {
  const value = props(overrides);
  await act(async () => { root.render(<AppLauncher {...value} />); });
  return value;
}

const input = () => document.querySelector<HTMLInputElement>('input[aria-label="Search apps and chats"]')!;
const sections = () => [...document.querySelectorAll("[data-launcher-section]")].map(node => node.getAttribute("data-launcher-section"));
const selected = () => document.querySelector('[role="option"][aria-selected="true"]');

async function type(value: string) {
  const setter = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, "value")!.set!;
  await act(async () => {
    setter.call(input(), value);
    input().dispatchEvent(new Event("input", { bubbles: true }));
  });
}

async function press(key: string, init: KeyboardEventInit = {}) {
  await act(async () => {
    input().dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true, ...init }));
  });
}

describe("launcherSections", () => {
  it("puts needs-you first and hides empty sections", () => {
    const empty = launcherSections({ query: "", badges: {}, chats: [], chatOpen: false });
    expect(empty.map(section => section.id)).toEqual(["apps"]);
    const full = launcherSections({ query: "", badges: { workflows: { count: 2 } }, chats, chatOpen: true });
    expect(full.map(section => section.id)).toEqual(["needs", "apps", "chats", "actions"]);
  });

  it("offers a search across chats only past the minimum length", () => {
    const short = launcherSections({ query: "a", badges: {}, chats, chatOpen: false });
    expect(short.find(section => section.id === "actions")).toBeUndefined();
    const long = launcherSections({ query: "deploy", badges: {}, chats, chatOpen: false });
    expect(long.find(section => section.id === "actions")?.items.map(item => item.kind)).toEqual(["search-all"]);
  });

  it("shows at most five chats", () => {
    const many = Array.from({ length: 9 }, (_, index) => ({ id: `c${index}`, title: `Chat ${index}` }));
    expect(launcherSections({ query: "", badges: {}, chats: many, chatOpen: false }).find(section => section.id === "chats")?.items).toHaveLength(5);
  });
});

describe("AppLauncher", () => {
  it("renders nothing while closed", async () => {
    await mount({ open: false });
    expect(input()).toBeNull();
  });

  it("lists sections in order, with what needs you first", async () => {
    await mount({ badges: { workflows: { count: 2 }, memory: { count: 3 } }, chatOpen: true });
    expect(sections()).toEqual(["needs", "apps", "chats", "actions"]);
    const needs = document.querySelector('[data-launcher-section="needs"]')!.textContent;
    expect(needs).toContain("Workflows");
    expect(needs).toContain("Memory");
    expect(selected()?.textContent).toContain("Workflows");
    expect(document.activeElement).toBe(input());
  });

  it("filters apps and chats, and says when nothing matches", async () => {
    await mount();
    await type("deploy");
    expect(document.querySelector('[data-launcher-section="chats"]')?.textContent).toContain("Fix the deploy script");
    expect(document.querySelector('[data-launcher-section="chats"]')?.textContent).not.toContain("Polish");
    await type("quota");
    expect(document.querySelector('[data-launcher-section="apps"]')?.textContent).toBe("AppsUsage");
    await type("zzzz");
    expect(document.body.textContent).toContain("No app or chat matches “zzzz”.");
    // The content search is still offered: titles missed, words may not.
    expect(document.body.textContent).toContain("Search all chats for “zzzz”");
  });

  it("moves through every item, wraps, and opens the highlighted one", async () => {
    const value = await mount();
    expect(selected()?.textContent).toContain("Notes");
    await press("ArrowDown");
    expect(selected()?.textContent).toContain("Pulls");
    await press("ArrowUp");
    await press("ArrowUp");
    // Wrapped to the last item: the oldest recent chat.
    expect(selected()?.textContent).toContain("Polish the Deck shell");
    await press("ArrowDown");
    await press("ArrowDown");
    await press("Enter");
    expect(value.onOpenApp).toHaveBeenCalledWith("pulls");
    expect(value.onClose).toHaveBeenCalled();
  });

  it("opens a chat on click", async () => {
    const value = await mount();
    const row = [...document.querySelectorAll('[role="option"]')].find(node => node.textContent?.includes("Fix the deploy script")) as HTMLElement;
    await act(async () => { row.click(); });
    expect(value.onOpenChat).toHaveBeenCalledWith("c1");
  });

  it("pins the highlighted app with ⌘P and says so", async () => {
    let pins: AppId[] = [];
    const onTogglePin = vi.fn((id: AppId): PinResult => { pins = [...pins, id]; return { ok: true, pins }; });
    await mount({ onTogglePin });
    await press("p", { metaKey: true });
    expect(onTogglePin).toHaveBeenCalledWith("notes");
    expect(document.querySelector('[role="status"]')?.textContent).toBe("Pinned Notes to the sidebar.");
  });

  it("refuses a fourth pin with the limit message", async () => {
    const onTogglePin = vi.fn((): PinResult => ({ ok: false, reason: "full" }));
    await mount({ pins: ["pulls", "usage", "memory"], onTogglePin });
    await press("p", { metaKey: true });
    expect(document.querySelector('[role="status"]')?.textContent).toBe(PIN_LIMIT_MESSAGE);
  });

  it("marks pinned apps", async () => {
    await mount({ pins: ["usage"] });
    expect(document.querySelector('[role="option"][aria-label="Usage, pinned"]')).not.toBeNull();
  });

  it("hands a long query to the sidebar's chat search", async () => {
    const value = await mount();
    await type("z");
    expect(document.body.textContent).not.toContain("Search all chats");
    await type("deploy rollback");
    const row = [...document.querySelectorAll('[role="option"]')].find(node => node.textContent?.includes("Search all chats")) as HTMLElement;
    await act(async () => { row.click(); });
    expect(value.onSearchChats).toHaveBeenCalledWith("deploy rollback");
  });

  it("offers Search this chat only while a chat is open", async () => {
    await mount({ chatOpen: false });
    expect(document.body.textContent).not.toContain("Search this chat");
    await act(async () => root.unmount());
    root = createRoot(container);
    const value = await mount({ chatOpen: true });
    const row = [...document.querySelectorAll('[role="option"]')].find(node => node.textContent?.includes("Search this chat")) as HTMLElement;
    await act(async () => { row.click(); });
    expect(value.onSearchThisChat).toHaveBeenCalled();
  });
});

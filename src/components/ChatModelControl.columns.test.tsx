// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ChatModelControl, type ChatModelControlProps } from "./ChatModelControl";
import { AccessControl } from "./AccessControl";
import { MODEL_FAVORITES_STORAGE_KEY } from "../modelFavorites";
import type { AdapterDescriptor } from "../types";

// The three-column picker: a harness rail, the model list, and a Thinking
// column in the user's effort selector style, with speed and access in the
// footer. The rail jumps between harness sections of one list, so every
// harness stays one search away.

const adapters: AdapterDescriptor[] = [
  {
    id: "claude", label: "Claude", available: true, authState: "signed_in", version: "test", capabilities: ["messages", "reasoning"], unavailableReason: null,
    models: [
      { id: "sonnet", label: "Claude Sonnet", tier: "standard", defaultForTier: true, supportedEffortLevels: ["low", "high", "xhigh"] },
      { id: "opus", label: "Claude Opus", tier: "strong", defaultForTier: true, supportedEffortLevels: ["low", "medium", "high", "xhigh", "max"] },
    ],
    defaultModel: "sonnet",
  },
  {
    id: "codex", label: "Codex", available: true, authState: "signed_in", version: "test", capabilities: ["messages", "reasoning"], unavailableReason: null,
    models: [
      { id: "gpt-luna", label: "GPT Luna", tier: "fast", defaultForTier: true, supportedEffortLevels: ["low", "medium", "high", "xhigh", "max", "ultra"] },
      { id: "gpt-terra", label: "GPT Terra", tier: "standard", defaultForTier: true, supportedEffortLevels: ["low", "medium", "high"] },
    ],
    defaultModel: "gpt-luna",
  },
];

let container: HTMLDivElement;
let root: Root;
const store = new Map<string, string>();
let original: PropertyDescriptor | undefined;

beforeEach(() => {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  // Favorites and the effort style both live in localStorage; a stub keeps
  // each case's stars and style its own.
  original = Object.getOwnPropertyDescriptor(globalThis, "localStorage");
  Object.defineProperty(globalThis, "localStorage", {
    configurable: true,
    value: {
      getItem: (key: string) => store.get(key) ?? null,
      setItem: (key: string, value: string) => { store.set(key, value); },
      removeItem: (key: string) => { store.delete(key); },
      clear: () => store.clear(),
    },
  });
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  store.clear();
  if (original) Object.defineProperty(globalThis, "localStorage", original);
  else Reflect.deleteProperty(globalThis, "localStorage");
  vi.restoreAllMocks();
});

const baseProps = (overrides: Partial<ChatModelControlProps> = {}): ChatModelControlProps => ({
  adapters, harness: "claude", model: "opus", onChange: vi.fn(), effort: "high", onEffortChange: vi.fn(), ...overrides,
});

/** Mounts the control inside a composer frame, the way both composers host it. */
async function mount(props: ChatModelControlProps) {
  await act(async () => root.render(
    <div data-composer-frame="">
      <textarea aria-label="Message Bridge" />
      <ChatModelControl {...props} />
    </div>,
  ));
}

const trigger = () => container.querySelector<HTMLButtonElement>('button[aria-haspopup="dialog"]')!;
const panel = () => container.querySelector<HTMLElement>('[role="dialog"][aria-label="Choose a model"]');
const zone = (name: string) => panel()!.querySelector<HTMLElement>(`[data-picker-zone="${name}"]`);
const rail = () => [...panel()!.querySelectorAll<HTMLButtonElement>("[data-rail]")];
const railItem = (label: string) => rail().find(item => item.getAttribute("aria-label") === label)!;
const lit = () => rail().filter(item => item.getAttribute("aria-current") === "true").map(item => item.getAttribute("aria-label"));
const search = () => panel()!.querySelector<HTMLInputElement>('input[aria-label="Search models"]')!;
const row = (text: string, within: ParentNode = panel()!) => [...within.querySelectorAll<HTMLButtonElement>('button[role="option"]')].find(button => button.textContent?.includes(text))!;
const groups = () => [...panel()!.querySelectorAll<HTMLElement>('[role="listbox"] [role="group"]')].map(group => group.getAttribute("aria-label"));
const active = () => document.activeElement as HTMLElement | null;
const press = async (key: string, options: KeyboardEventInit = {}) => {
  const event = new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true, ...options });
  await act(async () => { (document.activeElement ?? document.body).dispatchEvent(event); });
  return event;
};
const typeSearch = async (text: string) => {
  const input = search();
  await act(async () => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input, text);
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
};

describe("ChatModelControl columns", () => {
  it("opens on a harness rail, the model list and a Thinking column, with keys in the footer", async () => {
    await mount(baseProps());
    await act(async () => trigger().click());
    expect(rail().map(item => item.getAttribute("aria-label"))).toEqual(["Favorites", "Claude", "Codex"]);
    expect(lit()).toEqual(["Claude"]);
    expect(zone("models")!.querySelector('[role="listbox"]')).not.toBeNull();
    expect(groups()).toEqual(["Claude", "Codex"]);
    expect(zone("thinking")!.textContent).toContain("Thinking effort");
    expect(zone("thinking")!.querySelector('[data-testid="effort-control"]')).not.toBeNull();
    expect(zone("footer")!.textContent).toContain("column");
    expect(zone("footer")!.textContent).toContain("use");
    // Typing filters at once: focus starts in the search box.
    expect(active()).toBe(search());
  });

  it("keeps to the rail and the list on a surface that does not do effort", async () => {
    await mount(baseProps({ effort: undefined, onEffortChange: undefined }));
    await act(async () => trigger().click());
    expect(zone("rail")).not.toBeNull();
    expect(zone("thinking")).toBeNull();
    expect(panel()!.className).toContain("w-[340px]");
  });

  it("lights a harness from the rail, and the arrows move the rail's one tab stop", async () => {
    await mount(baseProps());
    await act(async () => trigger().click());
    await act(async () => railItem("Codex").click());
    expect(lit()).toEqual(["Codex"]);
    expect(rail().filter(item => item.tabIndex === 0).map(item => item.getAttribute("aria-label"))).toEqual(["Codex"]);
    await act(async () => railItem("Codex").focus());
    await press("ArrowUp");
    expect(lit()).toEqual(["Claude"]);
    expect(active()).toBe(railItem("Claude"));
    await press("Home");
    expect(lit()).toEqual(["Favorites"]);
    expect(panel()!.textContent).toContain("No favorites yet. Star a model to keep it here.");
    await press("End");
    expect(lit()).toEqual(["Codex"]);
    expect(panel()!.textContent).not.toContain("No favorites yet");
  });

  it("goes from a rail mark into that harness's models with Enter", async () => {
    await mount(baseProps());
    await act(async () => trigger().click());
    await act(async () => railItem("Codex").focus());
    await press("Enter");
    expect(lit()).toEqual(["Codex"]);
    expect(active()?.textContent).toContain("GPT Luna");
  });

  it("searches every harness and dims the rail marks with nothing to show", async () => {
    await mount(baseProps());
    await act(async () => trigger().click());
    await typeSearch("terra");
    expect(groups()).toEqual(["Codex"]);
    expect(lit()).toEqual(["Codex"]);
    expect(railItem("Claude").className).toContain("opacity-40");
    expect(railItem("Codex").className).not.toContain("opacity-40");
    await typeSearch("");
    expect(groups()).toEqual(["Claude", "Codex"]);
    expect(lit()).toEqual(["Claude"]);
  });
});

describe("ChatModelControl column keys", () => {
  const access = <AccessControl policy={{ autoApproveProviderPermissions: false }} onChange={vi.fn()} />;

  it("cycles rail, models, thinking and footer with Tab, and back with Shift+Tab", async () => {
    await mount(baseProps({ accessControl: access }));
    await act(async () => trigger().click());
    expect(active()).toBe(search());
    await press("Tab");
    expect(active()?.getAttribute("role")).toBe("slider");
    expect(zone("thinking")!.contains(active())).toBe(true);
    await press("Tab");
    expect(active()?.getAttribute("aria-label")).toBe("Access: User approval");
    expect(zone("footer")!.contains(active())).toBe(true);
    await press("Tab");
    expect(active()).toBe(railItem("Claude"));
    await press("Tab");
    expect(active()).toBe(search());
    await press("Tab", { shiftKey: true });
    expect(active()).toBe(railItem("Claude"));
    await press("Tab", { shiftKey: true });
    expect(active()?.getAttribute("aria-label")).toBe("Access: User approval");
  });

  it("continues from a row reached with the arrows to the next column", async () => {
    await mount(baseProps());
    await act(async () => trigger().click());
    await press("ArrowDown");
    expect(active()?.textContent).toContain("Claude Opus");
    await press("Tab");
    expect(active()?.getAttribute("role")).toBe("slider");
    await act(async () => row("Claude Opus").focus());
    await press("Tab", { shiftKey: true });
    expect(active()).toBe(search());
  });

  it("moves through rows across harnesses with the arrows and picks with Enter", async () => {
    const onChange = vi.fn();
    await mount(baseProps({ onChange }));
    await act(async () => trigger().click());
    // From the search box the arrows land on the chat's own model first.
    await press("ArrowDown");
    expect(active()).toBe(row("Claude Opus"));
    await press("ArrowDown");
    expect(active()).toBe(row("GPT Luna"));
    await press("ArrowUp");
    await press("ArrowUp");
    expect(active()).toBe(row("Claude Sonnet"));
    await press("ArrowUp");
    expect(active()).toBe(search());
    await press("ArrowDown");
    await press("End");
    expect(active()).toBe(row("GPT Terra"));
    const enter = await press("Enter");
    expect(enter.defaultPrevented).toBe(true);
    expect(onChange).toHaveBeenCalledWith("codex", "gpt-terra", null);
    // A surface that does effort stays open for the new model's thinking.
    expect(panel()).not.toBeNull();
  });

  it("picks the first match with Enter in the search box, and never lets Enter reach the composer form", async () => {
    const onChange = vi.fn();
    await mount(baseProps({ onChange }));
    await act(async () => trigger().click());
    const idle = await press("Enter");
    expect(idle.defaultPrevented).toBe(true);
    expect(onChange).not.toHaveBeenCalled();
    await typeSearch("luna");
    await press("Enter");
    expect(onChange).toHaveBeenCalledWith("codex", "gpt-luna", null);
  });

  it("closes on Escape and hands focus back to the composer", async () => {
    await mount(baseProps());
    await act(async () => trigger().click());
    await press("ArrowDown");
    await press("Escape");
    expect(panel()).toBeNull();
    expect(active()?.getAttribute("aria-label")).toBe("Message Bridge");
  });

  it("returns focus to the pill outside a composer", async () => {
    await act(async () => root.render(<ChatModelControl {...baseProps()} placement="down" />));
    await act(async () => trigger().click());
    await press("Escape");
    expect(panel()).toBeNull();
    expect(active()).toBe(trigger());
  });

  it("closes a plain picker on pick and returns focus to the composer", async () => {
    const onChange = vi.fn();
    await mount(baseProps({ effort: undefined, onEffortChange: undefined, onChange }));
    await act(async () => trigger().click());
    await press("ArrowDown");
    await press("ArrowDown");
    await press("Enter");
    expect(onChange).toHaveBeenCalledWith("codex", "gpt-luna", null);
    expect(panel()).toBeNull();
    expect(active()?.getAttribute("aria-label")).toBe("Message Bridge");
  });

  it("keeps the level and closes with Enter in the Thinking column", async () => {
    store.set("bridge.effortSelector", "list");
    const onEffortChange = vi.fn();
    await mount(baseProps({ onEffortChange }));
    await act(async () => trigger().click());
    await press("Tab");
    expect(active()?.getAttribute("data-effort")).toBe("high");
    await press("ArrowDown");
    expect(onEffortChange).toHaveBeenLastCalledWith("xhigh");
    // A row that is only focused is chosen on Enter, then the picker closes.
    await act(async () => zone("thinking")!.querySelector<HTMLElement>('[data-effort="max"]')!.focus());
    await press("Enter");
    expect(onEffortChange).toHaveBeenLastCalledWith("max");
    expect(panel()).toBeNull();
    expect(active()?.getAttribute("aria-label")).toBe("Message Bridge");
  });

  it("lets the footer's access menu take its own Escape", async () => {
    await mount(baseProps({ accessControl: access }));
    await act(async () => trigger().click());
    const button = zone("footer")!.querySelector<HTMLButtonElement>('[aria-label="Access: User approval"]')!;
    await act(async () => button.click());
    const menu = document.body.querySelector<HTMLElement>('[role="menu"][aria-label="Access mode"]')!;
    expect(menu).not.toBeNull();
    const item = menu.querySelector<HTMLElement>('[role="menuitemradio"]')!;
    await act(async () => item.focus());
    await press("Escape");
    expect(document.body.querySelector('[role="menu"][aria-label="Access mode"]')).toBeNull();
    expect(panel()).not.toBeNull();
  });
});

describe("ChatModelControl Thinking column styles", () => {
  it.each(["slider", "sentence", "list"] as const)("draws the %s style in the Thinking column", async style => {
    store.set("bridge.effortSelector", style);
    await mount(baseProps());
    await act(async () => trigger().click());
    const thinking = zone("thinking")!;
    expect(thinking.querySelector(`[data-effort-style="${style}"]`)).not.toBeNull();
    expect([...thinking.querySelectorAll("[data-effort]")].map(node => node.getAttribute("data-effort"))).toEqual(["low", "medium", "high", "xhigh", "max"]);
    // One form at a time.
    expect(panel()!.querySelectorAll("[data-effort-style]")).toHaveLength(1);
  });

  it("swaps the column to the picked model's ladder", async () => {
    await mount(baseProps());
    await act(async () => trigger().click());
    await act(async () => row("GPT Luna").click());
    expect([...zone("thinking")!.querySelectorAll("[data-effort]")].map(node => node.getAttribute("data-effort"))).toEqual(["low", "medium", "high", "xhigh", "max", "ultra"]);
    expect(zone("thinking")!.textContent).toContain("GPT Luna");
  });
});

describe("ChatModelControl favorites", () => {
  const star = (label: string, within: ParentNode = panel()!) => within.querySelector<HTMLButtonElement>(`button[aria-label="Add ${label} to favorites"], button[aria-label="Remove ${label} from favorites"]`)!;

  it("stars a model into a favorites section on top, remembers it, and picks from there", async () => {
    const onChange = vi.fn();
    await mount(baseProps({ onChange }));
    await act(async () => trigger().click());
    expect(groups()).toEqual(["Claude", "Codex"]);
    await act(async () => star("GPT Luna").click());
    expect(groups()).toEqual(["Favorites", "Claude", "Codex"]);
    expect(star("GPT Luna").getAttribute("aria-pressed")).toBe("true");
    expect(JSON.parse(store.get(MODEL_FAVORITES_STORAGE_KEY)!)).toEqual([{ harness: "codex", model: "gpt-luna", account: null }]);

    // Still there on the next open of a fresh picker.
    await act(async () => root.unmount());
    root = createRoot(container);
    await mount(baseProps({ onChange }));
    await act(async () => trigger().click());
    const favorites = panel()!.querySelector<HTMLElement>('[role="group"][aria-label="Favorites"]')!;
    await act(async () => row("GPT Luna", favorites).click());
    expect(onChange).toHaveBeenCalledWith("codex", "gpt-luna", null);

    await act(async () => star("GPT Luna", favorites).click());
    expect(groups()).toEqual(["Claude", "Codex"]);
    expect(JSON.parse(store.get(MODEL_FAVORITES_STORAGE_KEY)!)).toEqual([]);
  });

  it("keeps the account a model was starred under", async () => {
    const onChange = vi.fn();
    const accounts = [{ harness: "claude" as const, id: "personal", label: "Personal" }];
    await mount(baseProps({ onChange, accounts }));
    await act(async () => trigger().click());
    const personal = panel()!.querySelector<HTMLElement>('[role="group"][aria-label="Claude · Personal"]')!;
    await act(async () => star("Claude Sonnet", personal).click());
    const favorites = panel()!.querySelector<HTMLElement>('[role="group"][aria-label="Favorites"]')!;
    expect(row("Claude Sonnet", favorites).textContent).toContain("Personal");
    await act(async () => row("Claude Sonnet", favorites).click());
    expect(onChange).toHaveBeenCalledWith("claude", "sonnet", "personal");
  });

  it("skips a favorite whose model or account is gone", async () => {
    store.set(MODEL_FAVORITES_STORAGE_KEY, JSON.stringify([
      { harness: "codex", model: "gpt-retired", account: null },
      { harness: "claude", model: "opus", account: "left-the-team" },
    ]));
    await mount(baseProps());
    await act(async () => trigger().click());
    expect(groups()).toEqual(["Claude", "Codex"]);
  });

  it("finds favorites with the search like any other row", async () => {
    store.set(MODEL_FAVORITES_STORAGE_KEY, JSON.stringify([{ harness: "codex", model: "gpt-luna", account: null }, { harness: "claude", model: "sonnet", account: null }]));
    await mount(baseProps());
    await act(async () => trigger().click());
    await typeSearch("sonnet");
    const favorites = panel()!.querySelector<HTMLElement>('[role="group"][aria-label="Favorites"]')!;
    expect([...favorites.querySelectorAll('button[role="option"]')].map(button => button.textContent)).toEqual(["Claude Sonnet"]);
  });
});

// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ChatModelControl, cleanModelLabel, modelDisplayName } from "./ChatModelControl";
import type { AdapterDescriptor } from "../types";

let container: HTMLDivElement;
let root: Root;

const adapters: AdapterDescriptor[] = [
  {
    id: "codex", label: "Codex", available: true, authState: "signed_in", version: "test", capabilities: ["messages"], unavailableReason: null,
    models: [{ id: "gpt-balanced", label: "GPT Balanced", tier: "standard", defaultForTier: true }], defaultModel: "gpt-balanced",
  },
  {
    id: "opencode", label: "OpenCode", available: true, authState: "signed_in", version: "test", capabilities: ["messages"], unavailableReason: null,
    models: [{ id: "ox-alpha-free", label: "Ox Alpha Free (Unlimited)", tier: "fast", defaultForTier: true }], defaultModel: "ox-alpha-free",
  },
  {
    id: "cursor", label: "Cursor", available: true, authState: "signed_in", version: "test", capabilities: ["messages"], unavailableReason: null,
    models: [
      { id: "cursor/claude-opus-4.1", label: "Claude Opus 4.1", tier: "standard", defaultForTier: true },
      // Cursor reports its fallback entry with the variant brackets left empty.
      { id: "cursor/default", label: "default[]", tier: "fast", defaultForTier: false },
    ], defaultModel: "cursor/claude-opus-4.1",
  },
];

const claudeAdapters: AdapterDescriptor[] = [
  {
    id: "claude", label: "Claude", available: true, authState: "signed_in", version: "test", capabilities: ["messages", "reasoning"], unavailableReason: null,
    models: [
      { id: "haiku", label: "Claude Haiku", tier: "fast", defaultForTier: true },
      { id: "sonnet", label: "Claude Sonnet", tier: "standard", defaultForTier: true },
      { id: "fable-5", label: "Claude Fable 5", tier: "strong", defaultForTier: false },
      { id: "fable-5-1", label: "Claude Fable 5.1", tier: "strong", defaultForTier: true },
    ],
    defaultModel: "sonnet",
  },
];

// A live Claude catalog that reports per-model effort levels: sonnet takes a
// ladder, haiku has no effort knob at all.
const effortAwareAdapters: AdapterDescriptor[] = [
  {
    id: "claude", label: "Claude", available: true, authState: "signed_in", version: "test", capabilities: ["messages", "reasoning"], unavailableReason: null,
    models: [
      { id: "haiku", label: "Claude Haiku", tier: "fast", defaultForTier: true, supportedEffortLevels: [] },
      { id: "sonnet", label: "Claude Sonnet", tier: "standard", defaultForTier: true, supportedEffortLevels: ["low", "high", "xhigh"] },
      { id: "opus", label: "Claude Opus", tier: "strong", defaultForTier: true, supportedEffortLevels: ["low", "medium", "high", "xhigh", "max"] },
    ],
    defaultModel: "sonnet",
  },
];

beforeEach(() => {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.restoreAllMocks();
});

const trigger = () => container.querySelector<HTMLButtonElement>("button")!;
const panel = () => container.querySelector<HTMLElement>(".u-glass-popover")!;

describe("ChatModelControl accounts", () => {
  const accounts = [{ harness: "claude" as const, id: "personal", label: "Personal" }, { harness: "codex" as const, id: "work", label: "Work" }];
  const groups = () => [...panel().querySelectorAll<HTMLElement>('[role="group"]')].map(group => group.getAttribute("aria-label"));
  const optionIn = (group: string, text: string) => [...panel().querySelectorAll<HTMLElement>(`[role="group"][aria-label="${group}"] button[role="option"]`)].find(button => button.textContent?.includes(text))!;

  it("offers each account as its own group and emits the account picked", async () => {
    const onChange = vi.fn();
    await act(async () => root.render(<ChatModelControl adapters={[...claudeAdapters, adapters[0]]} accounts={accounts} harness="claude" model="sonnet" onChange={onChange} />));
    await act(async () => trigger().click());
    expect(groups()).toEqual(["Claude", "Claude · Personal", "Codex", "Codex · Work"]);
    expect(optionIn("Claude", "Sonnet").getAttribute("aria-selected")).toBe("true");
    expect(optionIn("Claude · Personal", "Sonnet").getAttribute("aria-selected")).toBe("false");
    await act(async () => optionIn("Claude · Personal", "Haiku").click());
    expect(onChange).toHaveBeenCalledWith("claude", "haiku", "personal");
  });

  it("marks the chat's account and names it on the pill", async () => {
    const onChange = vi.fn();
    await act(async () => root.render(<ChatModelControl adapters={claudeAdapters} accounts={accounts} account="personal" harness="claude" model="sonnet" onChange={onChange} />));
    expect(trigger().textContent).toBe("Personal · Claude Sonnet");
    expect(trigger().getAttribute("aria-label")).toContain("Personal");
    await act(async () => trigger().click());
    expect(optionIn("Claude · Personal", "Sonnet").getAttribute("aria-selected")).toBe("true");
    expect(optionIn("Claude", "Sonnet").getAttribute("aria-selected")).toBe("false");
    await act(async () => optionIn("Claude", "Sonnet").click());
    expect(onChange).toHaveBeenCalledWith("claude", "sonnet", null);
  });

  it("treats the harness id as the user's own login", async () => {
    await act(async () => root.render(<ChatModelControl adapters={claudeAdapters} accounts={accounts} account="claude" harness="claude" model="sonnet" onChange={vi.fn()} />));
    expect(trigger().textContent).not.toContain("Personal");
    await act(async () => trigger().click());
    expect(optionIn("Claude", "Sonnet").getAttribute("aria-selected")).toBe("true");
  });

  it("finds an account group by its name", async () => {
    await act(async () => root.render(<ChatModelControl adapters={claudeAdapters} accounts={accounts} harness="claude" model="sonnet" onChange={vi.fn()} />));
    await act(async () => trigger().click());
    const search = panel().querySelector<HTMLInputElement>('input[aria-label="Search models"]')!;
    await act(async () => {
      const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!;
      setter.call(search, "personal");
      search.dispatchEvent(new Event("input", { bubbles: true }));
    });
    expect(groups()).toEqual(["Claude · Personal"]);
  });

  it("looks exactly as before with no accounts", async () => {
    await act(async () => root.render(<ChatModelControl adapters={claudeAdapters} harness="claude" model="sonnet" onChange={vi.fn()} />));
    await act(async () => trigger().click());
    expect(groups()).toEqual(["Claude"]);
  });
});

describe("ChatModelControl", () => {
  it("keeps distinct provider releases in the full catalogue", async () => {
    const onChange = vi.fn();
    await act(async () => root.render(<ChatModelControl adapters={claudeAdapters} harness="claude" model="sonnet" onChange={onChange} />));
    await act(async () => trigger().click());
    expect(panel().textContent).toContain("Claude Fable 5");
    expect(panel().textContent).toContain("Claude Fable 5.1");
    const latest = [...panel().querySelectorAll<HTMLButtonElement>('button[role="option"]')].find(button => button.textContent?.includes("5.1"))!;
    await act(async () => latest.click());
    expect(onChange).toHaveBeenCalledWith("claude", "fable-5-1", null);
  });

  it("refreshes provider catalogues from the picker", async () => {
    const onRefresh = vi.fn().mockResolvedValue(undefined);
    await act(async () => root.render(<ChatModelControl adapters={adapters} harness="codex" model="gpt-balanced" onChange={vi.fn()} onRefresh={onRefresh} />));
    await act(async () => trigger().click());
    const refresh = panel().querySelector<HTMLButtonElement>('button[aria-label="Refresh model catalogues"]')!;
    await act(async () => refresh.click());
    expect(onRefresh).toHaveBeenCalledOnce();
  });
  it("caps the compact pill's width and ellipsizes when constrained", async () => {
    await act(async () => root.render(
      <ChatModelControl adapters={adapters} harness="codex" model="gpt-balanced" compact maxWidthClassName="max-w-[190px]" onChange={vi.fn()} />,
    ));
    expect(trigger().parentElement!.className).toContain("max-w-[190px]");
    const label = trigger().querySelector("span")!;
    expect(label.className).toContain("overflow-hidden");
    expect(label.className).toContain("text-ellipsis");
  });

  it("exposes the full, unpolluted label via title even while ellipsized", async () => {
    await act(async () => root.render(
      <ChatModelControl adapters={adapters} harness="codex" model="gpt-balanced" compact maxWidthClassName="max-w-[190px]" onChange={vi.fn()} />,
    ));
    expect(trigger().dataset.tip).toBe("Codex · GPT Balanced");
  });

  it("strips the trailing (Unlimited) suffix from the displayed label", async () => {
    await act(async () => root.render(
      <ChatModelControl adapters={adapters} harness="opencode" model="ox-alpha-free" compact onChange={vi.fn()} />,
    ));
    expect(trigger().textContent).toContain("Ox Alpha Free");
    expect(trigger().textContent).not.toContain("Unlimited");
    expect(trigger().title).not.toContain("Unlimited");
  });

  // Cursor's catalog reported "default[]" and the pill rendered it verbatim:
  // "Cursor · default[]", which reads as a rendering bug rather than a name.
  it("strips a trailing empty bracket pair from the displayed label", async () => {
    await act(async () => root.render(
      <ChatModelControl adapters={adapters} harness="cursor" model="cursor/default" compact onChange={vi.fn()} />,
    ));
    expect(trigger().textContent).toContain("Cursor · default");
    expect(trigger().textContent).not.toContain("[");
    expect(trigger().dataset.tip).toBe("Cursor · default");
    expect(trigger().getAttribute("aria-label")).toBe("Chat model: Cursor default");
  });

  it("cleans the label in the picker rows too", async () => {
    await act(async () => root.render(
      <ChatModelControl adapters={adapters} harness="cursor" model="cursor/default" compact onChange={vi.fn()} />,
    ));
    await act(async () => trigger().click());
    const rows = [...panel().querySelectorAll("button")];
    expect(rows.some(row => row.textContent?.includes("default["))).toBe(false);
    const row = rows.find(button => button.textContent?.includes("default"))!;
    expect(row.textContent).toContain("default");
  });

  // The id is the wire value; only the rendered text is cleaned.
  it("emits the untouched model id for a cleaned label", async () => {
    const onChange = vi.fn();
    await act(async () => root.render(
      <ChatModelControl adapters={adapters} harness="codex" model="gpt-balanced" compact onChange={onChange} />,
    ));
    await act(async () => trigger().click());
    const row = [...panel().querySelectorAll("button")].find(button => button.textContent?.startsWith("default"))!;
    await act(async () => row.click());
    expect(onChange).toHaveBeenCalledWith("cursor", "cursor/default", null);
  });

  it("opens the panel upward by default, where the composer sits at the bottom of the view", async () => {
    await act(async () => root.render(
      <ChatModelControl adapters={adapters} harness="codex" model="gpt-balanced" compact onChange={vi.fn()} />,
    ));
    await act(async () => trigger().click());
    expect(panel().className).toContain("bottom-full");
    expect(panel().className).not.toContain("top-full");
  });

  it("opens the panel downward when placed in a top chrome row", async () => {
    await act(async () => root.render(
      <ChatModelControl adapters={adapters} harness="codex" model="gpt-balanced" compact placement="down" onChange={vi.fn()} />,
    ));
    await act(async () => trigger().click());
    expect(panel().className).toContain("top-full");
    expect(panel().className).not.toContain("bottom-full");
  });

  it("drops the harness prefix when the model label already opens with it", async () => {
    const stuttering: AdapterDescriptor[] = [{
      id: "opencode", label: "OpenCode", available: true, authState: "signed_in", version: "test", capabilities: [], unavailableReason: null,
      models: [{ id: "oc-go-mimo", label: "OpenCode Go · MiMo V2.5", tier: "fast", defaultForTier: true }], defaultModel: "oc-go-mimo",
    }];
    await act(async () => root.render(
      <ChatModelControl adapters={stuttering} harness="opencode" model="oc-go-mimo" compact onChange={vi.fn()} />,
    ));
    expect(trigger().dataset.tip).toBe("OpenCode Go · MiMo V2.5");
    expect(trigger().title).not.toContain("OpenCode · OpenCode");
  });

  it("keeps the harness prefix when the model label does not carry it", async () => {
    await act(async () => root.render(
      <ChatModelControl adapters={adapters} harness="codex" model="gpt-balanced" compact onChange={vi.fn()} />,
    ));
    expect(trigger().dataset.tip).toBe("Codex · GPT Balanced");
  });

  it("opens the picker and calls onChange when a model is chosen", async () => {
    const onChange = vi.fn();
    await act(async () => root.render(
      <ChatModelControl adapters={adapters} harness="codex" model="gpt-balanced" compact onChange={onChange} />,
    ));
    await act(async () => trigger().click());
    const option = [...container.querySelectorAll("button")].find(button => button.textContent?.includes("Ox Alpha Free"))!;
    await act(async () => option.click());
    expect(onChange).toHaveBeenCalledWith("opencode", "ox-alpha-free", null);
  });

  it("derives Cursor from descriptors and returns its exact ACP model id", async () => {
    const onChange = vi.fn();
    await act(async () => root.render(
      <ChatModelControl adapters={adapters} harness="codex" model="gpt-balanced" compact onChange={onChange} />,
    ));
    await act(async () => trigger().click());
    const option = [...container.querySelectorAll("button")].find(button => button.textContent?.includes("Claude Opus 4.1"))!;
    await act(async () => option.click());
    expect(onChange).toHaveBeenCalledWith("cursor", "cursor/claude-opus-4.1", null);
  });

  it("closes an open picker on Escape without letting the key reach modal hosts", async () => {
    const hostEscape = vi.fn();
    window.addEventListener("keydown", hostEscape);
    try {
      await act(async () => root.render(
        <ChatModelControl adapters={adapters} harness="codex" model="gpt-balanced" compact onChange={vi.fn()} />,
      ));
      await act(async () => trigger().click());
      expect(panel()).toBeTruthy();
      await act(async () => { trigger().dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true })); });
      expect(container.querySelector(".u-glass-popover")).toBeNull();
      expect(hostEscape).not.toHaveBeenCalled();
      await act(async () => { trigger().dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true })); });
      expect(hostEscape).toHaveBeenCalledTimes(1);
    } finally {
      window.removeEventListener("keydown", hostEscape);
    }
  });

  it("keeps internal routing tiers out of the model picker", async () => {
    await act(async () => root.render(
      <ChatModelControl adapters={claudeAdapters} harness="claude" model="sonnet" onChange={vi.fn()} />,
    ));
    await act(async () => trigger().click());
    expect(panel().querySelector('[data-tier]')).toBeNull();
    expect(panel().textContent).not.toMatch(/strong|standard|fast/i);
  });

  // Group headers get a divider once search narrows the list to fewer groups
  // than the full catalog — only the first *visible* group should go
  // undecorated, not literally the first adapter in the catalog.
  it("puts the divider on the first visible group, not the first catalog group", async () => {
    await act(async () => root.render(
      <ChatModelControl adapters={adapters} harness="codex" model="gpt-balanced" onChange={vi.fn()} />,
    ));
    await act(async () => trigger().click());
    const search = panel().querySelector<HTMLInputElement>('input[aria-label="Search models"]')!;
    const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!;
    // "Codex" is the first adapter in the catalog; filtering it out entirely
    // must not leave "OpenCode" carrying a divider meant for a group that no
    // longer renders.
    await act(async () => { setter.call(search, "opencode"); search.dispatchEvent(new Event("input", { bubbles: true })); });
    const headers = [...panel().querySelectorAll('[role="listbox"] > div > div:first-child')];
    expect(headers).toHaveLength(1);
    expect(headers[0].className).not.toContain("border-t");
  });

  it("highlights the current effort in the footer segmented control", async () => {
    await act(async () => root.render(
      <ChatModelControl adapters={effortAwareAdapters} harness="claude" model="sonnet" onChange={vi.fn()} effort="high" />,
    ));
    await act(async () => trigger().click());
    const control = panel().querySelector('[data-testid="effort-control"]');
    expect(control).not.toBeNull();
    const active = control!.querySelectorAll('[aria-checked="true"]');
    // Exactly one segment reads as the active effort, and it is the one passed in.
    expect(active).toHaveLength(1);
    expect(active[0].textContent).toBe("High");
  });

  it("omits the effort footer when effort is null and no setter is wired", async () => {
    await act(async () => root.render(
      <ChatModelControl adapters={claudeAdapters} harness="claude" model="sonnet" onChange={vi.fn()} effort={null} />,
    ));
    await act(async () => trigger().click());
    expect(panel().querySelector('[data-testid="effort-control"]')).toBeNull();
  });

  it("offers only the selected model's effort levels when the catalog reports them", async () => {
    await act(async () => root.render(
      <ChatModelControl adapters={effortAwareAdapters} harness="claude" model="sonnet" onChange={vi.fn()} effort="high" onEffortChange={vi.fn()} />,
    ));
    await act(async () => trigger().click());
    const labels = [...panel().querySelectorAll('[data-testid="effort-control"] button')].map(button => button.textContent);
    // Sonnet advertises low/high/xhigh — medium is not offered.
    expect(labels).toEqual(["Low", "High", "XHigh"]);
  });

  it("shows the wider ladder, including Max, for a model that supports it", async () => {
    await act(async () => root.render(
      <ChatModelControl adapters={effortAwareAdapters} harness="claude" model="opus" onChange={vi.fn()} effort="high" onEffortChange={vi.fn()} />,
    ));
    await act(async () => trigger().click());
    const efforts = [...panel().querySelectorAll('[data-testid="effort-control"] button')].map(button => button.getAttribute("data-effort"));
    expect(efforts).toEqual(["low", "medium", "high", "xhigh", "max"]);
  });

  it("hides the effort control for a model with no effort support", async () => {
    await act(async () => root.render(
      <ChatModelControl adapters={effortAwareAdapters} harness="claude" model="haiku" onChange={vi.fn()} effort="high" onEffortChange={vi.fn()} />,
    ));
    await act(async () => trigger().click());
    expect(panel().querySelector('[data-testid="effort-control"]')).toBeNull();
  });

  it("does not invent effort levels when discovery has no capability data", async () => {
    await act(async () => root.render(
      <ChatModelControl adapters={claudeAdapters} harness="claude" model="sonnet" onChange={vi.fn()} effort="high" onEffortChange={vi.fn()} />,
    ));
    await act(async () => trigger().click());
    expect(panel().querySelector('[data-testid="effort-control"]')).toBeNull();
  });

  it("moves effort selection off the rows into the footer control", async () => {
    const onEffortChange = vi.fn();
    await act(async () => root.render(
      <ChatModelControl adapters={effortAwareAdapters} harness="claude" model="sonnet" onChange={vi.fn()} effort="high" onEffortChange={onEffortChange} />,
    ));
    await act(async () => trigger().click());
    // The model rows no longer carry an effort badge — effort lives in one place.
    expect(panel().querySelector('[data-testid="effort-badge"]')).toBeNull();
    const low = [...panel().querySelectorAll('[data-testid="effort-control"] button')].find(button => button.textContent === "Low")!;
    await act(async () => (low as HTMLButtonElement).click());
    expect(onEffortChange).toHaveBeenCalledWith("low");
  });
});

describe("modelDisplayName", () => {
  it("strips a trailing (Unlimited) suffix case-insensitively", () => {
    expect(modelDisplayName(adapters, "opencode", "ox-alpha-free")).toBe("Ox Alpha Free");
  });

  it("strips a trailing empty bracket pair", () => {
    expect(modelDisplayName(adapters, "cursor", "cursor/default")).toBe("default");
  });

  it("falls back to Automatic when no model is configured", () => {
    expect(modelDisplayName(adapters, "codex", null)).toBe("Automatic");
  });
});

describe("cleanModelLabel", () => {
  it("strips an empty bracket pair however the vendor spaced it", () => {
    expect(cleanModelLabel("default[]")).toBe("default");
    expect(cleanModelLabel("default []")).toBe("default");
    expect(cleanModelLabel("default[ ]")).toBe("default");
    expect(cleanModelLabel("default [ ] ")).toBe("default");
  });

  it("strips the (Unlimited) plan marker, alone or beside empty brackets", () => {
    expect(cleanModelLabel("Ox Alpha Free (Unlimited)")).toBe("Ox Alpha Free");
    expect(cleanModelLabel("Ox Alpha Free (unlimited) []")).toBe("Ox Alpha Free");
  });

  it("leaves every other label exactly as the catalog reported it", () => {
    expect(cleanModelLabel("Claude Opus 4.1")).toBe("Claude Opus 4.1");
    expect(cleanModelLabel("OpenCode Go · MiMo V2.5")).toBe("OpenCode Go · MiMo V2.5");
    // Only an empty pair is noise; a bracket that names something stays.
    expect(cleanModelLabel("Sonnet [thinking]")).toBe("Sonnet [thinking]");
  });
  it("uses the provider default model for automatic thinking capabilities", async () => {
    await act(async () => root.render(<ChatModelControl adapters={effortAwareAdapters} harness="claude" model={null} onChange={vi.fn()} onEffortChange={vi.fn()} />));
    await act(async () => trigger().click());
    expect(trigger().textContent).toContain("Sonnet");
    expect([...panel().querySelectorAll('[data-effort]')].map(el => el.getAttribute('data-effort'))).toEqual(["low", "high", "xhigh"]);
  });

  it("does not borrow another model's capabilities for a stale model", async () => {
    await act(async () => root.render(<ChatModelControl adapters={effortAwareAdapters} harness="claude" model="retired" onChange={vi.fn()} onEffortChange={vi.fn()} />));
    await act(async () => trigger().click());
    expect(panel().querySelector('[data-testid="effort-control"]')).toBeNull();
  });

  it("closes an open picker when a turn disables changes", async () => {
    const props = { adapters: effortAwareAdapters, harness: "claude" as const, model: "sonnet", onChange: vi.fn(), onEffortChange: vi.fn() };
    await act(async () => root.render(<ChatModelControl {...props} />));
    await act(async () => trigger().click());
    await act(async () => root.render(<ChatModelControl {...props} disabled />));
    expect(panel()).toBeNull();
  });

  it.each([false, true])("handles refresh failure without an unhandled rejection (sync=%s)", async sync => {
    const onRefresh = () => { if (sync) throw new Error("offline"); return Promise.reject(new Error("offline")); };
    await act(async () => root.render(<ChatModelControl adapters={adapters} harness="codex" model="gpt-balanced" onChange={vi.fn()} onRefresh={onRefresh} />));
    await act(async () => trigger().click());
    const refresh = panel().querySelector<HTMLButtonElement>('[aria-label="Refresh model catalogues"]')!;
    await act(async () => refresh.click());
    expect(panel().querySelector('[role="alert"]')?.textContent).toContain("Could not refresh");
    expect(refresh.disabled).toBe(false);
  });

});

// The Thinking row: one segment per level, a click or an arrow key picks one.
describe("ChatModelControl effort control", () => {
  const codexAdapters: AdapterDescriptor[] = [
    {
      id: "codex", label: "Codex", available: true, authState: "signed_in", version: "test", capabilities: ["messages", "reasoning"], unavailableReason: null,
      models: [{ id: "gpt-luna", label: "GPT Luna", tier: "strong", defaultForTier: true, supportedEffortLevels: ["low", "medium", "high", "xhigh", "max", "ultra"] }],
      defaultModel: "gpt-luna",
    },
  ];
  const control = () => panel().querySelector<HTMLElement>('[data-testid="effort-control"]')!;
  const key = (target: Element, key: string) => target.dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true }));
  /** The level the row shows as current, or null for the provider default. */
  const checked = () => control().querySelector('[role="radio"][aria-checked="true"]')?.getAttribute("data-effort") ?? null;
  const segment = (level: string) => control().querySelector<HTMLButtonElement>(`[role="radio"][data-effort="${level}"]`)!;

  // jsdom here has an opaque origin and no storage; favorites are read from
  // localStorage during render, so give it a minimal stub.
  const store = new Map<string, string>();
  let original: PropertyDescriptor | undefined;
  beforeEach(() => {
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
  });
  afterEach(() => {
    store.clear();
    if (original) Object.defineProperty(globalThis, "localStorage", original);
    else Reflect.deleteProperty(globalThis, "localStorage");
  });

  describe("segments", () => {
    it("picks a level in one click, checks it at once, and steps with the keyboard", async () => {
      const onEffortChange = vi.fn();
      await act(async () => root.render(<ChatModelControl adapters={effortAwareAdapters} harness="claude" model="opus" onChange={vi.fn()} effort="high" onEffortChange={onEffortChange} />));
      await act(async () => trigger().click());
      expect(control().querySelector('[role="radiogroup"]')!.getAttribute("aria-label")).toBe("Reasoning effort");
      expect(checked()).toBe("high");
      // The current level is the group's one tab stop.
      expect([...control().querySelectorAll<HTMLButtonElement>('[role="radio"]')].filter(radio => radio.tabIndex === 0).map(radio => radio.dataset.effort)).toEqual(["high"]);
      await act(async () => segment("max").click());
      expect(onEffortChange).toHaveBeenLastCalledWith("max");
      expect(checked()).toBe("max");
      await act(async () => { key(segment("max"), "ArrowLeft"); });
      expect(onEffortChange).toHaveBeenLastCalledWith("xhigh");
      await act(async () => { key(segment("xhigh"), "Home"); });
      expect(onEffortChange).toHaveBeenLastCalledWith("low");
      await act(async () => { key(segment("low"), "End"); });
      expect(onEffortChange).toHaveBeenLastCalledWith("max");
      // Clicking the level already set sends nothing.
      onEffortChange.mockClear();
      await act(async () => segment("max").click());
      expect(onEffortChange).not.toHaveBeenCalled();
    });

    it("captions the row with what the current level means", async () => {
      await act(async () => root.render(<ChatModelControl adapters={effortAwareAdapters} harness="claude" model="opus" onChange={vi.fn()} effort="xhigh" onEffortChange={vi.fn()} />));
      await act(async () => trigger().click());
      const thinking = panel().querySelector('[data-picker-zone="thinking"]')!;
      expect(thinking.textContent).toContain("Thinking");
      expect(thinking.textContent).toContain("Long reasoning chains.");
      expect(segment("max").dataset.tip).toBe("Longest Claude allows.");
    });

    it("fits Codex's six levels in one row", async () => {
      await act(async () => root.render(<ChatModelControl adapters={codexAdapters} harness="codex" model="gpt-luna" onChange={vi.fn()} effort="ultra" onEffortChange={vi.fn()} />));
      await act(async () => trigger().click());
      expect([...control().querySelectorAll('[role="radio"]')].map(radio => radio.textContent)).toEqual(["Low", "Med", "High", "XHigh", "Max", "Ultra"]);
      expect(checked()).toBe("ultra");
      expect(control().querySelectorAll('[role="radiogroup"]')).toHaveLength(1);
    });

    it("checks nothing while the session has no effort yet, and says it is the provider's default", async () => {
      const onEffortChange = vi.fn();
      await act(async () => root.render(<ChatModelControl adapters={effortAwareAdapters} harness="claude" model="opus" onChange={vi.fn()} effort={null} onEffortChange={onEffortChange} />));
      await act(async () => trigger().click());
      expect(checked()).toBeNull();
      expect(panel().textContent).toContain("The provider's own default.");
      // The first segment is the tab stop; the first arrow lands on it.
      expect(segment("low").tabIndex).toBe(0);
      await act(async () => { key(segment("low"), "ArrowRight"); });
      expect(onEffortChange).toHaveBeenLastCalledWith("low");
    });

    it("keeps the popover open on a model pick, highlights it at once, and swaps to its ladder", async () => {
      const onChange = vi.fn();
      await act(async () => root.render(<ChatModelControl adapters={effortAwareAdapters} harness="claude" model="sonnet" onChange={onChange} effort="high" onEffortChange={vi.fn()} />));
      await act(async () => trigger().click());
      expect([...control().querySelectorAll("[data-effort]")].map(el => el.getAttribute("data-effort"))).toEqual(["low", "high", "xhigh"]);
      const opus = [...panel().querySelectorAll<HTMLButtonElement>('button[role="option"]')].find(button => button.textContent?.includes("Opus"))!;
      await act(async () => opus.click());
      expect(onChange).toHaveBeenCalledWith("claude", "opus", null);
      // Still open, the picked row already reads as selected, and the footer
      // is Opus's five-step ladder — before the host has confirmed anything.
      expect(panel()).not.toBeNull();
      expect(opus.getAttribute("aria-selected")).toBe("true");
      expect([...control().querySelectorAll("[data-effort]")].map(el => el.getAttribute("data-effort"))).toEqual(["low", "medium", "high", "xhigh", "max"]);
      // high is on Opus's ladder too, so the session's level carries over.
      expect(checked()).toBe("high");
    });

    it("queues a level chosen while the host is still switching and sends it once the switch settles", async () => {
      const onEffortChange = vi.fn();
      const props = { adapters: effortAwareAdapters, harness: "claude" as const, model: "sonnet", onChange: vi.fn(), effort: "high", onEffortChange };
      await act(async () => root.render(<ChatModelControl {...props} />));
      await act(async () => trigger().click());
      const opus = [...panel().querySelectorAll<HTMLButtonElement>('button[role="option"]')].find(button => button.textContent?.includes("Opus"))!;
      await act(async () => opus.click());
      // The live session goes busy while the provider restarts. The effort
      // control stays live regardless…
      await act(async () => root.render(<ChatModelControl {...props} disabled />));
      expect(panel()).not.toBeNull();
      expect(segment("max").disabled).toBe(false);
      // …a level picked now is shown at once but not sent yet…
      await act(async () => segment("max").click());
      expect(onEffortChange).not.toHaveBeenCalled();
      expect(checked()).toBe("max");
      // …and goes out exactly once when the switch settles.
      await act(async () => root.render(<ChatModelControl {...props} model="opus" />));
      expect(onEffortChange).toHaveBeenCalledTimes(1);
      expect(onEffortChange).toHaveBeenCalledWith("max");
      expect(panel()).not.toBeNull();
    });

    it("sends a queued level after the popover was closed mid-switch", async () => {
      const onEffortChange = vi.fn();
      const props = { adapters: effortAwareAdapters, harness: "claude" as const, model: "sonnet", onChange: vi.fn(), effort: "high", onEffortChange };
      await act(async () => root.render(<ChatModelControl {...props} />));
      await act(async () => trigger().click());
      const opus = [...panel().querySelectorAll<HTMLButtonElement>('button[role="option"]')].find(button => button.textContent?.includes("Opus"))!;
      await act(async () => opus.click());
      await act(async () => root.render(<ChatModelControl {...props} disabled />));
      await act(async () => segment("max").click());
      // "I'm done": dismiss while the provider is still restarting.
      await act(async () => container.querySelector<HTMLElement>(".fixed.inset-0")!.click());
      expect(panel()).toBeNull();
      expect(onEffortChange).not.toHaveBeenCalled();
      await act(async () => root.render(<ChatModelControl {...props} model="opus" />));
      expect(onEffortChange).toHaveBeenCalledTimes(1);
      expect(onEffortChange).toHaveBeenCalledWith("max");
    });

    it("keeps showing a sent level, live, until the host confirms it", async () => {
      const onEffortChange = vi.fn();
      const props = { adapters: effortAwareAdapters, harness: "claude" as const, model: "opus", onChange: vi.fn(), effort: "high", onEffortChange };
      await act(async () => root.render(<ChatModelControl {...props} />));
      await act(async () => trigger().click());
      await act(async () => segment("max").click());
      expect(onEffortChange).toHaveBeenCalledWith("max");
      // The host goes busy for the effort round-trip; the prop is still stale.
      await act(async () => root.render(<ChatModelControl {...props} disabled />));
      expect(checked()).toBe("max");
      expect(segment("max").disabled).toBe(false);
      await act(async () => root.render(<ChatModelControl {...props} effort="max" />));
      expect(checked()).toBe("max");
    });

    it("reverts to the host's level when it settles without adopting the sent one", async () => {
      const props = { adapters: effortAwareAdapters, harness: "claude" as const, model: "opus", onChange: vi.fn(), effort: "high", onEffortChange: vi.fn() };
      await act(async () => root.render(<ChatModelControl {...props} />));
      await act(async () => trigger().click());
      await act(async () => segment("max").click());
      await act(async () => root.render(<ChatModelControl {...props} disabled />));
      await act(async () => root.render(<ChatModelControl {...props} />));
      expect(checked()).toBe("high");
    });

    it("drops the queue and the highlight when the switch fails", async () => {
      const onEffortChange = vi.fn();
      const props = { adapters: effortAwareAdapters, harness: "claude" as const, model: "sonnet", onChange: vi.fn(), effort: "high", onEffortChange };
      await act(async () => root.render(<ChatModelControl {...props} />));
      await act(async () => trigger().click());
      const opus = [...panel().querySelectorAll<HTMLButtonElement>('button[role="option"]')].find(button => button.textContent?.includes("Opus"))!;
      await act(async () => opus.click());
      await act(async () => root.render(<ChatModelControl {...props} disabled />));
      await act(async () => segment("max").click());
      // The host settles still on Sonnet: the switch did not happen.
      await act(async () => root.render(<ChatModelControl {...props} />));
      expect(onEffortChange).not.toHaveBeenCalled();
      const sonnet = [...panel().querySelectorAll<HTMLButtonElement>('button[role="option"]')].find(button => button.textContent?.includes("Sonnet"))!;
      expect(sonnet.getAttribute("aria-selected")).toBe("true");
      expect([...control().querySelectorAll("[data-effort]")].map(el => el.getAttribute("data-effort"))).toEqual(["low", "high", "xhigh"]);
      expect(checked()).toBe("high");
    });

    it("ignores a second pick while its own switch is in flight", async () => {
      const onChange = vi.fn();
      const props = { adapters: effortAwareAdapters, harness: "claude" as const, model: "sonnet", onChange, effort: "high", onEffortChange: vi.fn() };
      await act(async () => root.render(<ChatModelControl {...props} />));
      await act(async () => trigger().click());
      const row = (name: string) => [...panel().querySelectorAll<HTMLButtonElement>('button[role="option"]')].find(button => button.textContent?.includes(name))!;
      await act(async () => row("Opus").click());
      await act(async () => root.render(<ChatModelControl {...props} disabled />));
      expect(panel().querySelector('[role="listbox"]')!.getAttribute("aria-busy")).toBe("true");
      await act(async () => row("Haiku").click());
      expect(onChange).toHaveBeenCalledTimes(1);
      // Once settled, picking is possible again.
      await act(async () => root.render(<ChatModelControl {...props} model="opus" />));
      await act(async () => row("Haiku").click());
      expect(onChange).toHaveBeenCalledTimes(2);
    });

    it("stays in flight when the host confirms the model before clearing busy", async () => {
      const onChange = vi.fn();
      const onEffortChange = vi.fn();
      const props = { adapters: effortAwareAdapters, harness: "claude" as const, model: "sonnet", onChange, effort: "high", onEffortChange };
      await act(async () => root.render(<ChatModelControl {...props} />));
      await act(async () => trigger().click());
      const row = (name: string) => [...panel().querySelectorAll<HTMLButtonElement>('button[role="option"]')].find(button => button.textContent?.includes(name))!;
      await act(async () => row("Opus").click());
      // The host confirms the new model while still busy with the restart.
      await act(async () => root.render(<ChatModelControl {...props} model="opus" disabled />));
      expect(row("Opus").getAttribute("aria-selected")).toBe("true");
      // Still our request: a second pick is still blocked…
      await act(async () => row("Haiku").click());
      expect(onChange).toHaveBeenCalledTimes(1);
      expect(panel().querySelector('[role="listbox"]')!.getAttribute("aria-busy")).toBe("true");
      // …and the effort control is still live, queueing for the settle.
      expect(segment("max").disabled).toBe(false);
      await act(async () => segment("max").click());
      expect(onEffortChange).not.toHaveBeenCalled();
      await act(async () => root.render(<ChatModelControl {...props} model="opus" />));
      expect(onEffortChange).toHaveBeenCalledTimes(1);
      expect(onEffortChange).toHaveBeenCalledWith("max");
    });

    it("does not mistake a later turn for its own request after a pick that never went busy", async () => {
      const props = { adapters: effortAwareAdapters, harness: "claude" as const, model: "sonnet", onChange: vi.fn(), effort: "high", onEffortChange: vi.fn() };
      await act(async () => root.render(<ChatModelControl {...props} />));
      await act(async () => trigger().click());
      const opus = [...panel().querySelectorAll<HTMLButtonElement>('button[role="option"]')].find(button => button.textContent?.includes("Opus"))!;
      await act(async () => opus.click());
      // A draft host applies the pick at once, with no busy window.
      await act(async () => root.render(<ChatModelControl {...props} model="opus" />));
      expect(panel()).not.toBeNull();
      // A turn starting later is not ours: the popover closes.
      await act(async () => root.render(<ChatModelControl {...props} model="opus" disabled />));
      expect(panel()).toBeNull();
    });

    it("forgets anything in flight when a turn, not the picker, disables it", async () => {
      const onEffortChange = vi.fn();
      const props = { adapters: effortAwareAdapters, harness: "claude" as const, model: "opus", onChange: vi.fn(), effort: "high", onEffortChange };
      await act(async () => root.render(<ChatModelControl {...props} />));
      await act(async () => trigger().click());
      // Nothing of ours is in flight when the turn starts, so it closes…
      await act(async () => root.render(<ChatModelControl {...props} disabled />));
      expect(panel()).toBeNull();
      // …and settling afterwards sends nothing.
      await act(async () => root.render(<ChatModelControl {...props} />));
      expect(onEffortChange).not.toHaveBeenCalled();
    });

    it("still closes on pick for a plain surface that does not do effort", async () => {
      await act(async () => root.render(<ChatModelControl adapters={effortAwareAdapters} harness="claude" model="sonnet" onChange={vi.fn()} />));
      await act(async () => trigger().click());
      const opus = [...panel().querySelectorAll<HTMLButtonElement>('button[role="option"]')].find(button => button.textContent?.includes("Opus"))!;
      await act(async () => opus.click());
      expect(panel()).toBeNull();
    });

    it("stays inert without a setter but still shows the level", async () => {
      await act(async () => root.render(<ChatModelControl adapters={effortAwareAdapters} harness="claude" model="opus" onChange={vi.fn()} effort="max" />));
      await act(async () => trigger().click());
      expect(checked()).toBe("max");
      expect([...control().querySelectorAll("button")].every(button => button.disabled)).toBe(true);
    });
  });

  describe("fit to the window", () => {
    const box = (top: number, height = 28) => ({ top, bottom: top + height, left: 0, right: 200, width: 200, height, x: 0, y: top, toJSON: () => ({}) }) as DOMRect;

    it("flips down when the composer sits too close to the top and clamps to the room it has", async () => {
      vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockReturnValue(box(100));
      Object.defineProperty(window, "innerHeight", { configurable: true, value: 800 });
      await act(async () => root.render(<ChatModelControl adapters={effortAwareAdapters} harness="claude" model="opus" onChange={vi.fn()} effort="high" onEffortChange={vi.fn()} />));
      await act(async () => trigger().click());
      expect(panel().className).toContain("top-full");
      expect(panel().style.maxHeight).toBe("520px");
    });

    it("stays up and shortens itself when the room above is between the bounds", async () => {
      vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockReturnValue(box(450));
      Object.defineProperty(window, "innerHeight", { configurable: true, value: 800 });
      await act(async () => root.render(<ChatModelControl adapters={effortAwareAdapters} harness="claude" model="opus" onChange={vi.fn()} effort="high" onEffortChange={vi.fn()} />));
      await act(async () => trigger().click());
      expect(panel().className).toContain("bottom-full");
      expect(panel().style.maxHeight).toBe("438px");
    });

    it("keeps the search header inside a clipping pane, even when neither side fits the preferred minimum", async () => {
      let paneTop = 44;
      container.style.overflowY = "hidden";
      vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (this: HTMLElement) {
        return this === container ? box(paneTop, 400 - paneTop) : box(300);
      });
      Object.defineProperty(window, "innerHeight", { configurable: true, value: 400 });
      await act(async () => root.render(<ChatModelControl adapters={effortAwareAdapters} harness="claude" model="opus" onChange={vi.fn()} effort="high" onEffortChange={vi.fn()} />));
      await act(async () => trigger().click());
      expect(panel().style.maxHeight).toBe("244px");
      paneTop = 140;
      await act(async () => window.dispatchEvent(new Event("resize")));
      expect(panel().className).toContain("bottom-full");
      expect(panel().style.maxHeight).toBe("148px");
    });

    it("never crushes the list under the Thinking row on a short window", async () => {
      vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockReturnValue(box(330));
      Object.defineProperty(window, "innerHeight", { configurable: true, value: 380 });
      await act(async () => root.render(<ChatModelControl adapters={effortAwareAdapters} harness="claude" model="opus" onChange={vi.fn()} effort="high" onEffortChange={vi.fn()} />));
      await act(async () => trigger().click());
      expect(panel().style.maxHeight).toBe("318px");
      // The list keeps a floor of rows; whatever does not fit scrolls with
      // the popover as a whole, and the switch caption gives up its line.
      const list = panel().querySelector('[role="listbox"]')!;
      expect(list.className).toContain("min-h-36");
      expect(list.className).not.toContain("min-h-0");
      expect(panel().className).toContain("overflow-y-auto");
      expect(panel().textContent).not.toContain("Switching restarts the provider session");
    });

    it("leaves the defaults alone when the trigger has no box", async () => {
      await act(async () => root.render(<ChatModelControl adapters={effortAwareAdapters} harness="claude" model="opus" onChange={vi.fn()} effort="high" onEffortChange={vi.fn()} />));
      await act(async () => trigger().click());
      expect(panel().className).toContain("bottom-full");
      expect(panel().style.maxHeight).toBe("");
    });
  });

  describe("across a model switch", () => {
    it("keeps the popover open across a model switch, with the level row live", async () => {
      const onChange = vi.fn();
      const onEffortChange = vi.fn();
      await act(async () => root.render(<ChatModelControl adapters={effortAwareAdapters} harness="claude" model="opus" onChange={onChange} effort="high" onEffortChange={onEffortChange} />));
      await act(async () => trigger().click());
      expect(panel().className).toContain("w-[380px]");
      const radios = control().querySelectorAll('[role="radio"]');
      expect(radios).toHaveLength(5);
      expect(control().querySelector('[role="radio"][aria-checked="true"]')!.getAttribute("data-effort")).toBe("high");
      await act(async () => (radios[4] as HTMLButtonElement).click());
      expect(onEffortChange).toHaveBeenLastCalledWith("max");

      const sonnet = [...panel().querySelectorAll<HTMLButtonElement>('button[role="option"]')].find(button => button.textContent?.includes("Sonnet"))!;
      await act(async () => sonnet.click());
      expect(onChange).toHaveBeenCalledWith("claude", "sonnet", null);
      expect(panel()).not.toBeNull();
    });

    it("rides through the disabled flip its own model switch causes, but still closes for a turn", async () => {
      const props = { adapters: effortAwareAdapters, harness: "claude" as const, model: "opus", onChange: vi.fn(), effort: "high", onEffortChange: vi.fn() };
      await act(async () => root.render(<ChatModelControl {...props} />));
      await act(async () => trigger().click());
      const sonnet = [...panel().querySelectorAll<HTMLButtonElement>('button[role="option"]')].find(button => button.textContent?.includes("Sonnet"))!;
      await act(async () => sonnet.click());
      // The live session goes busy while the provider restarts…
      await act(async () => root.render(<ChatModelControl {...props} model="sonnet" disabled />));
      expect(panel()).not.toBeNull();
      // …and the popover is editable again once it settles.
      await act(async () => root.render(<ChatModelControl {...props} model="sonnet" />));
      expect(panel()).not.toBeNull();
      expect([...panel().querySelectorAll<HTMLButtonElement>('[role="radio"]')].every(radio => !radio.disabled)).toBe(true);
      // A disable that is not our switch — a turn starting — still closes it.
      await act(async () => root.render(<ChatModelControl {...props} model="sonnet" disabled />));
      expect(panel()).toBeNull();
    });

    it("keeps the pane, and its width, for a model with no effort knob", async () => {
      await act(async () => root.render(<ChatModelControl adapters={effortAwareAdapters} harness="claude" model="haiku" onChange={vi.fn()} effort="high" onEffortChange={vi.fn()} />));
      await act(async () => trigger().click());
      expect(panel().className).toContain("w-[380px]");
      expect(panel().querySelector('[data-testid="effort-control"]')).toBeNull();
      expect(panel().textContent).toContain("No thinking control for this model.");
    });

    it("stays single-pane on a surface that does not do effort", async () => {
      await act(async () => root.render(<ChatModelControl adapters={effortAwareAdapters} harness="claude" model="opus" onChange={vi.fn()} />));
      await act(async () => trigger().click());
      expect(panel().className).toContain("w-[380px]");
      expect(panel().querySelector('[data-picker-zone="thinking"]')).toBeNull();
    });
  });

  describe("Codex", () => {
    const codexLadder = ["low", "medium", "high", "xhigh", "max", "ultra"];
    const codexThinking: AdapterDescriptor[] = [
      {
        id: "codex", label: "Codex", available: true, authState: "signed_in", version: "test", capabilities: ["messages", "reasoning"], unavailableReason: null,
        models: [
          { id: "gpt-5.6-sol", label: "GPT Sol", tier: "strong", defaultForTier: true, supportedEffortLevels: codexLadder },
          { id: "gpt-spark", label: "GPT Spark", tier: "fast", defaultForTier: true, supportedEffortLevels: [] },
        ],
        defaultModel: "gpt-5.6-sol",
      },
    ];

    it("shows an interactive effort control for a Codex model that advertises a ladder", async () => {
      const onEffortChange = vi.fn();
      await act(async () => root.render(<ChatModelControl adapters={codexThinking} harness="codex" model="gpt-5.6-sol" onChange={vi.fn()} effort="high" onEffortChange={onEffortChange} />));
      await act(async () => trigger().click());
      const badge = [...panel().querySelectorAll("span")].find(node => node.textContent === "thinking");
      expect(badge).toBeTruthy();
      const effort = panel().querySelector<HTMLElement>('[data-testid="effort-control"]')!;
      expect(effort).not.toBeNull();
      expect([...effort.querySelectorAll("[data-effort]")].map(node => node.getAttribute("data-effort"))).toEqual(codexLadder);
      await act(async () => effort.querySelector<HTMLButtonElement>('[data-effort="xhigh"]')!.click());
      expect(onEffortChange).toHaveBeenCalledWith("xhigh");
      expect(panel().textContent).toContain("Switching restarts the provider session.");
    });

    it("omits the effort footer when a Codex model advertises no levels", async () => {
      await act(async () => root.render(<ChatModelControl adapters={codexThinking} harness="codex" model="gpt-spark" onChange={vi.fn()} effort="high" onEffortChange={vi.fn()} />));
      await act(async () => trigger().click());
      expect(panel().querySelector('[data-testid="effort-control"]')).toBeNull();
      expect([...panel().querySelectorAll("span")].some(node => node.textContent === "thinking")).toBe(true);
      expect(panel().textContent).toContain("Switching restarts the provider session.");
    });

    it("keeps Claude's own ladder when Codex is also in the catalog", async () => {
      const mixed = [...codexThinking, ...effortAwareAdapters];
      await act(async () => root.render(<ChatModelControl adapters={mixed} harness="claude" model="sonnet" onChange={vi.fn()} effort="high" onEffortChange={vi.fn()} />));
      await act(async () => trigger().click());
      expect([...panel().querySelectorAll('[data-testid="effort-control"] [data-effort]')].map(node => node.getAttribute("data-effort"))).toEqual(["low", "high", "xhigh"]);
    });

    describe("Thinking row", () => {
      it("names the missing control instead of leaving the effort pane blank", async () => {
        await act(async () => root.render(<ChatModelControl adapters={codexThinking} harness="codex" model="gpt-spark" onChange={vi.fn()} effort="high" onEffortChange={vi.fn()} />));
        await act(async () => trigger().click());
        expect(panel().querySelector('[data-testid="effort-control"]')).toBeNull();
        expect(panel().textContent).toContain("No thinking control for this model.");
        expect(panel().querySelector('[data-picker-zone="thinking"]')!.textContent).toContain("Thinking");
      });

      it("changes effort from the row for GPT Sol", async () => {
        const onEffortChange = vi.fn();
        await act(async () => root.render(<ChatModelControl adapters={codexThinking} harness="codex" model="gpt-5.6-sol" onChange={vi.fn()} effort="medium" onEffortChange={onEffortChange} />));
        await act(async () => trigger().click());
        const effort = panel().querySelector<HTMLElement>('[data-testid="effort-control"]')!;
        expect(effort).not.toBeNull();
        await act(async () => effort.querySelector<HTMLButtonElement>('[data-effort="ultra"]')!.click());
        expect(onEffortChange).toHaveBeenCalledWith("ultra");
      });
    });
  });
});

// A new chat's model is resolved by the host, so the first frames have no model
// and no catalog entry. The pill must not name a guess it is about to replace.
describe("ChatModelControl while the host resolves the model", () => {
  const discovering: AdapterDescriptor[] = [{
    id: "opencode", label: "OpenCode", available: true, authState: "signed_in", version: "test", capabilities: ["messages"], unavailableReason: null,
    models: [], defaultModel: null,
  }];

  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it("TestChatModelControl_ShowsLoadingStateAfterGracePeriod", async () => {
    await act(async () => root.render(<ChatModelControl adapters={discovering} harness="opencode" model={null} loading onChange={vi.fn()} />));
    expect(trigger().textContent).not.toContain("Loading models");
    await act(async () => { vi.advanceTimersByTime(600); });
    expect(trigger().dataset.tip).toBe("Loading models");
    expect(trigger().textContent).toBe("");
    expect(trigger().getAttribute("aria-busy")).toBe("true");
    expect(trigger().getAttribute("aria-label")).toBe("Chat model: loading");
  });

  it("TestChatModelControl_NoLoadingFlashBeforeGracePeriod", async () => {
    await act(async () => root.render(<ChatModelControl adapters={discovering} harness="opencode" model={null} loading onChange={vi.fn()} />));
    await act(async () => { vi.advanceTimersByTime(300); });
    expect(trigger().textContent).not.toContain("Loading models");
    expect(trigger().getAttribute("aria-busy")).toBeNull();
  });

  it("TestChatModelControl_ShowsKnownLabelInsteadOfLoading", async () => {
    await act(async () => root.render(<ChatModelControl adapters={discovering} harness="opencode" model="opencode-go/kimi" knownLabel="Kimi K2.5" loading onChange={vi.fn()} />));
    await act(async () => { vi.advanceTimersByTime(1_000); });
    expect(trigger().textContent).toBe("OpenCode · Kimi K2.5");
    expect(trigger().textContent).not.toContain("Loading models");
    expect(trigger().getAttribute("aria-busy")).toBeNull();
  });

  it("TestChatModelControl_LoadingPickerHasNoFakeDefaultRow", async () => {
    await act(async () => root.render(<ChatModelControl adapters={discovering} harness="opencode" model={null} loading onChange={vi.fn()} />));
    await act(async () => { vi.advanceTimersByTime(600); });
    await act(async () => trigger().click());
    expect(panel().querySelector('[role="status"][aria-label="Loading models"]')).not.toBeNull();
    expect(panel().querySelector('[role="option"]')).toBeNull();
    expect(panel().textContent).not.toContain("Default");
  });
});

describe("ChatModelControl speed", () => {
  // Live catalogs: Claude Opus lists fast mode; Codex lists `priority` as Fast
  // and, where offered, Ultrafast. Sonnet lists nothing faster.
  const speedAdapters: AdapterDescriptor[] = [
    {
      id: "claude", label: "Claude", available: true, authState: "signed_in", version: "test", capabilities: ["messages"], unavailableReason: null,
      models: [
        { id: "claude-opus-5-5", label: "Opus 5.5", tier: "strong", defaultForTier: true, supportedEffortLevels: ["high", "xhigh", "max", "ultracode"], speedTiers: [{ id: "fast", label: "Fast", description: "Faster output on the same model" }] },
        { id: "claude-sonnet-5-5", label: "Sonnet 5.5", tier: "standard", defaultForTier: true, supportedEffortLevels: ["high"] },
      ],
      defaultModel: "claude-sonnet-5-5",
    },
    {
      id: "codex", label: "Codex", available: true, authState: "signed_in", version: "test", capabilities: ["messages"], unavailableReason: null,
      models: [{ id: "gpt-6-astra", label: "GPT-6 Astra", tier: "strong", defaultForTier: true, speedTiers: [{ id: "priority", label: "Fast" }, { id: "ultrafast", label: "Ultrafast" }] }],
      defaultModel: "gpt-6-astra",
    },
  ];
  const speedRadios = () => [...panel().querySelectorAll<HTMLButtonElement>('[role="radiogroup"][aria-label="Speed"] [role="radio"]')];

  it("offers standard plus each listed tier and emits the tier id", async () => {
    const onSpeedChange = vi.fn();
    await act(async () => root.render(
      <ChatModelControl adapters={speedAdapters} harness="codex" model="gpt-6-astra" compact onChange={vi.fn()} speed={null} onSpeedChange={onSpeedChange} />,
    ));
    await act(async () => trigger().click());
    expect(speedRadios().map(radio => radio.textContent)).toEqual(["Standard", "Fast", "Ultrafast"]);
    expect(speedRadios().find(radio => radio.getAttribute("aria-checked") === "true")?.dataset.speed).toBe("standard");
    await act(async () => speedRadios().find(radio => radio.dataset.speed === "ultrafast")!.click());
    expect(onSpeedChange).toHaveBeenCalledWith("ultrafast");
  });

  it("marks Claude fast mode and goes back to standard explicitly", async () => {
    const onSpeedChange = vi.fn();
    await act(async () => root.render(
      <ChatModelControl adapters={speedAdapters} harness="claude" model="claude-opus-5-5" compact onChange={vi.fn()} speed="fast" onSpeedChange={onSpeedChange} />,
    ));
    await act(async () => trigger().click());
    expect(speedRadios().find(radio => radio.getAttribute("aria-checked") === "true")?.dataset.speed).toBe("fast");
    expect(panel().textContent).toContain("Faster output on the same model");
    await act(async () => speedRadios().find(radio => radio.dataset.speed === "standard")!.click());
    expect(onSpeedChange).toHaveBeenCalledWith("standard");
  });

  it("shows nothing for a model without faster tiers or a surface that cannot change speed", async () => {
    await act(async () => root.render(
      <ChatModelControl adapters={speedAdapters} harness="claude" model="claude-sonnet-5-5" compact onChange={vi.fn()} speed={null} onSpeedChange={vi.fn()} />,
    ));
    await act(async () => trigger().click());
    expect(container.querySelector('[data-testid="speed-control"]')).toBeNull();
    await act(async () => root.render(
      <ChatModelControl adapters={speedAdapters} harness="claude" model="claude-opus-5-5" compact onChange={vi.fn()} speed="fast" />,
    ));
    expect(container.querySelector('[data-testid="speed-control"]')).toBeNull();
  });
});

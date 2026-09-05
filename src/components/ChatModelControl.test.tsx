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

describe("ChatModelControl", () => {
  it("keeps distinct provider releases in the full catalogue", async () => {
    const onChange = vi.fn();
    await act(async () => root.render(<ChatModelControl adapters={claudeAdapters} harness="claude" model="sonnet" onChange={onChange} />));
    await act(async () => trigger().click());
    expect(panel().textContent).toContain("Claude Fable 5");
    expect(panel().textContent).toContain("Claude Fable 5.1");
    const latest = [...panel().querySelectorAll<HTMLButtonElement>('button[role="option"]')].find(button => button.textContent?.includes("5.1"))!;
    await act(async () => latest.click());
    expect(onChange).toHaveBeenCalledWith("claude", "fable-5-1");
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
    expect(trigger().className).toContain("max-w-[190px]");
    const label = trigger().querySelector("span")!;
    expect(label.className).toContain("overflow-hidden");
    expect(label.className).toContain("text-ellipsis");
  });

  it("exposes the full, unpolluted label via title even while ellipsized", async () => {
    await act(async () => root.render(
      <ChatModelControl adapters={adapters} harness="codex" model="gpt-balanced" compact maxWidthClassName="max-w-[190px]" onChange={vi.fn()} />,
    ));
    expect(trigger().title).toBe("Codex · GPT Balanced");
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
    expect(trigger().title).toBe("Cursor · default");
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
    expect(onChange).toHaveBeenCalledWith("cursor", "cursor/default");
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
    expect(trigger().title).toBe("OpenCode Go · MiMo V2.5");
    expect(trigger().title).not.toContain("OpenCode · OpenCode");
  });

  it("keeps the harness prefix when the model label does not carry it", async () => {
    await act(async () => root.render(
      <ChatModelControl adapters={adapters} harness="codex" model="gpt-balanced" compact onChange={vi.fn()} />,
    ));
    expect(trigger().title).toBe("Codex · GPT Balanced");
  });

  it("opens the picker and calls onChange when a model is chosen", async () => {
    const onChange = vi.fn();
    await act(async () => root.render(
      <ChatModelControl adapters={adapters} harness="codex" model="gpt-balanced" compact onChange={onChange} />,
    ));
    await act(async () => trigger().click());
    const option = [...container.querySelectorAll("button")].find(button => button.textContent?.includes("Ox Alpha Free"))!;
    await act(async () => option.click());
    expect(onChange).toHaveBeenCalledWith("opencode", "ox-alpha-free");
  });

  it("derives Cursor from descriptors and returns its exact ACP model id", async () => {
    const onChange = vi.fn();
    await act(async () => root.render(
      <ChatModelControl adapters={adapters} harness="codex" model="gpt-balanced" compact onChange={onChange} />,
    ));
    await act(async () => trigger().click());
    const option = [...container.querySelectorAll("button")].find(button => button.textContent?.includes("Claude Opus 4.1"))!;
    await act(async () => option.click());
    expect(onChange).toHaveBeenCalledWith("cursor", "cursor/claude-opus-4.1");
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
    const active = control!.querySelectorAll('[aria-pressed="true"]');
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

// The user's chosen thinking-control style (Settings › Appearance) is read from
// storage; each style renders the same levels in a different form.
describe("ChatModelControl effort styles", () => {
  const codexAdapters: AdapterDescriptor[] = [
    {
      id: "codex", label: "Codex", available: true, authState: "signed_in", version: "test", capabilities: ["messages", "reasoning"], unavailableReason: null,
      models: [{ id: "gpt-luna", label: "GPT Luna", tier: "strong", defaultForTier: true, supportedEffortLevels: ["low", "medium", "high", "xhigh", "max", "ultra"] }],
      defaultModel: "gpt-luna",
    },
  ];
  const control = () => panel().querySelector<HTMLElement>('[data-testid="effort-control"]')!;
  const key = (target: Element, key: string) => target.dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true }));

  // jsdom here has an opaque origin and no storage; the style is read from
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

  describe("slider (default)", () => {
    it("announces the current level on the thumb and steps with the keyboard", async () => {
      const onEffortChange = vi.fn();
      await act(async () => root.render(<ChatModelControl adapters={effortAwareAdapters} harness="claude" model="opus" onChange={vi.fn()} effort="high" onEffortChange={onEffortChange} />));
      await act(async () => trigger().click());
      const thumb = control().querySelector('[role="slider"]')!;
      expect(thumb.getAttribute("aria-valuetext")).toBe("High");
      expect(thumb.getAttribute("aria-valuenow")).toBe("2");
      await act(async () => { key(thumb, "ArrowRight"); });
      expect(onEffortChange).toHaveBeenLastCalledWith("xhigh");
      await act(async () => { key(thumb, "Home"); });
      expect(onEffortChange).toHaveBeenLastCalledWith("low");
      await act(async () => { key(thumb, "End"); });
      expect(onEffortChange).toHaveBeenLastCalledWith("max");
    });

    it("draws Codex's six levels into the same fixed footer box as Claude's five", async () => {
      await act(async () => root.render(<ChatModelControl adapters={codexAdapters} harness="codex" model="gpt-luna" onChange={vi.fn()} effort="ultra" onEffortChange={vi.fn()} />));
      await act(async () => trigger().click());
      expect(control().querySelectorAll("[data-effort]")).toHaveLength(6);
      expect(control().className).toContain("h-[72px]");
      expect(control().textContent).toContain("Ultra");
    });

    it("says Default, with nothing lit, while the session has no effort yet", async () => {
      const onEffortChange = vi.fn();
      await act(async () => root.render(<ChatModelControl adapters={effortAwareAdapters} harness="claude" model="opus" onChange={vi.fn()} effort={null} onEffortChange={onEffortChange} />));
      await act(async () => trigger().click());
      expect(control().querySelector('[role="slider"]')!.getAttribute("aria-valuetext")).toBe("Default");
      expect(control().querySelectorAll('[aria-pressed="true"]')).toHaveLength(0);
      await act(async () => { key(control().querySelector('[role="slider"]')!, "ArrowRight"); });
      expect(onEffortChange).toHaveBeenLastCalledWith("medium");
    });

    it("stays inert without a setter but still shows the level", async () => {
      await act(async () => root.render(<ChatModelControl adapters={effortAwareAdapters} harness="claude" model="opus" onChange={vi.fn()} effort="max" />));
      await act(async () => trigger().click());
      expect(control().querySelector('[role="slider"]')!.getAttribute("aria-valuetext")).toBe("Max");
      expect([...control().querySelectorAll("button")].every(button => button.disabled)).toBe(true);
    });
  });

  describe("sentence", () => {
    beforeEach(() => localStorage.setItem("bridge.effortSelector", "sentence"));

    it("reads as prose naming the model and steps on click, wrapping at the top", async () => {
      const onEffortChange = vi.fn();
      await act(async () => root.render(<ChatModelControl adapters={effortAwareAdapters} harness="claude" model="sonnet" onChange={vi.fn()} effort="high" onEffortChange={onEffortChange} />));
      await act(async () => trigger().click());
      expect(control().textContent).toContain("Think");
      expect(control().textContent).toContain("with Claude Sonnet.");
      const word = control().querySelector<HTMLButtonElement>('button[data-effort="high"]')!;
      expect(word.textContent).toBe("properly");
      // A click is a pointer down and up without movement. jsdom has no
      // PointerEvent; a MouseEvent under the pointer type name reaches the
      // same React handler.
      const pointer = (target: Element, type: string) => target.dispatchEvent(new MouseEvent(type, { clientX: 100, bubbles: true }));
      await act(async () => { pointer(word, "pointerdown"); });
      await act(async () => { pointer(word, "pointerup"); });
      expect(onEffortChange).toHaveBeenLastCalledWith("xhigh");

      await act(async () => root.render(<ChatModelControl adapters={effortAwareAdapters} harness="claude" model="sonnet" onChange={vi.fn()} effort="xhigh" onEffortChange={onEffortChange} />));
      const top = control().querySelector<HTMLButtonElement>('button[data-effort="xhigh"]')!;
      await act(async () => { pointer(top, "pointerdown"); });
      await act(async () => { pointer(top, "pointerup"); });
      expect(onEffortChange).toHaveBeenLastCalledWith("low");
    });

    it("reads 'normally' while unset and steps onto the first level", async () => {
      const onEffortChange = vi.fn();
      await act(async () => root.render(<ChatModelControl adapters={effortAwareAdapters} harness="claude" model="sonnet" onChange={vi.fn()} effort={null} onEffortChange={onEffortChange} />));
      await act(async () => trigger().click());
      const word = control().querySelector<HTMLButtonElement>("p button")!;
      expect(word.textContent).toBe("normally");
      await act(async () => { key(word, "Enter"); });
      expect(onEffortChange).toHaveBeenLastCalledWith("low");
    });

    it("scrubs with the arrow keys", async () => {
      const onEffortChange = vi.fn();
      await act(async () => root.render(<ChatModelControl adapters={effortAwareAdapters} harness="claude" model="sonnet" onChange={vi.fn()} effort="high" onEffortChange={onEffortChange} />));
      await act(async () => trigger().click());
      await act(async () => { key(control().querySelector('button[data-effort]')!, "ArrowLeft"); });
      expect(onEffortChange).toHaveBeenLastCalledWith("low");
    });
  });

  describe("list", () => {
    beforeEach(() => localStorage.setItem("bridge.effortSelector", "list"));

    it("renders effort as radios in a second pane and keeps the popover open across a model switch", async () => {
      const onChange = vi.fn();
      const onEffortChange = vi.fn();
      await act(async () => root.render(<ChatModelControl adapters={effortAwareAdapters} harness="claude" model="opus" onChange={onChange} effort="high" onEffortChange={onEffortChange} />));
      await act(async () => trigger().click());
      expect(panel().className).toContain("w-[460px]");
      const radios = control().querySelectorAll('[role="radio"]');
      expect(radios).toHaveLength(5);
      expect(control().querySelector('[role="radio"][aria-checked="true"]')!.getAttribute("data-effort")).toBe("high");
      await act(async () => (radios[4] as HTMLButtonElement).click());
      expect(onEffortChange).toHaveBeenLastCalledWith("max");
      await act(async () => { key(control().querySelector('[role="radiogroup"]')!, "1"); });
      expect(onEffortChange).toHaveBeenLastCalledWith("low");

      const sonnet = [...panel().querySelectorAll<HTMLButtonElement>('button[role="option"]')].find(button => button.textContent?.includes("Sonnet"))!;
      await act(async () => sonnet.click());
      expect(onChange).toHaveBeenCalledWith("claude", "sonnet");
      expect(panel()).not.toBeNull();
    });

    it("keeps the pane, and its width, for a model with no effort knob", async () => {
      await act(async () => root.render(<ChatModelControl adapters={effortAwareAdapters} harness="claude" model="haiku" onChange={vi.fn()} effort="high" onEffortChange={vi.fn()} />));
      await act(async () => trigger().click());
      expect(panel().className).toContain("w-[460px]");
      expect(panel().querySelector('[data-testid="effort-control"]')).toBeNull();
      expect(panel().textContent).toContain("No thinking control for this model.");
    });

    it("stays single-pane on a surface that does not do effort", async () => {
      await act(async () => root.render(<ChatModelControl adapters={effortAwareAdapters} harness="claude" model="opus" onChange={vi.fn()} />));
      await act(async () => trigger().click());
      expect(panel().className).toContain("w-[340px]");
      expect(panel().textContent).not.toContain("Thinking effort");
    });
  });
});

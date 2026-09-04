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
    id: "claude", label: "Claude", available: true, authState: "signed_in", version: "test", capabilities: ["messages"], unavailableReason: null,
    models: [
      { id: "haiku", label: "Claude Haiku", tier: "fast", defaultForTier: true },
      { id: "sonnet", label: "Claude Sonnet", tier: "standard", defaultForTier: true },
      { id: "fable", label: "Claude Fable", tier: "strong", defaultForTier: true },
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

  it("renders the tier badge for each model row in the open picker", async () => {
    await act(async () => root.render(
      <ChatModelControl adapters={claudeAdapters} harness="claude" model="sonnet" onChange={vi.fn()} />,
    ));
    await act(async () => trigger().click());
    const tierBadges = panel().querySelectorAll('[data-tier]');
    const tiers = [...tierBadges].map(el => el.getAttribute("data-tier"));
    expect(tiers).toEqual(["fast", "standard", "strong"]);
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
      <ChatModelControl adapters={claudeAdapters} harness="claude" model="sonnet" onChange={vi.fn()} effort="high" />,
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

  it("moves effort selection off the rows into the footer control", async () => {
    const onEffortChange = vi.fn();
    await act(async () => root.render(
      <ChatModelControl adapters={claudeAdapters} harness="claude" model="sonnet" onChange={vi.fn()} effort="high" onEffortChange={onEffortChange} />,
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
});

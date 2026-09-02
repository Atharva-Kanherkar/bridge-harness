// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ChatModelControl, type ChatModelControlProps } from "./ChatModelControl";
import type { AdapterDescriptor } from "../types";

let container: HTMLDivElement;
let root: Root;

const adapters: AdapterDescriptor[] = [
  {
    id: "claude",
    label: "Claude",
    available: true,
    authState: "signed_in",
    capabilities: ["messages"],
    models: [
      { id: "haiku", label: "Claude Haiku", tier: "fast", defaultForTier: true },
      { id: "sonnet", label: "Claude Sonnet", tier: "standard", defaultForTier: true },
      { id: "fable", label: "Claude Fable", tier: "strong", defaultForTier: true },
    ],
    defaultModel: "sonnet",
    sandboxModes: [],
  },
];

const noop = () => {};

function props(overrides: Partial<ChatModelControlProps> = {}): ChatModelControlProps {
  return {
    adapters,
    harness: "claude",
    model: "sonnet",
    onChange: noop,
    ...overrides,
  };
}

function mount(overrides: Partial<ChatModelControlProps> = {}) {
  act(() => { root.render(<ChatModelControl {...props(overrides)} />); });
}

function click(element: Element) {
  act(() => { element.dispatchEvent(new MouseEvent("click", { bubbles: true })); });
}

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

describe("ChatModelControl", () => {
  it("renders the tier badge for each model row in the open picker", () => {
    mount();
    click(container.querySelector("button")!);
    const rows = document.querySelectorAll('[class*="w-full flex items-center"]');
    expect(rows.length).toBeGreaterThanOrEqual(3);
    const tiers = [...rows].map(row => row.querySelector('[class*="uppercase"]')?.textContent);
    expect(tiers).toContain("fast");
    expect(tiers).toContain("standard");
    expect(tiers).toContain("strong");
  });

  it("shows effort badge on the selected row when effort is provided", () => {
    mount({ effort: "high" });
    click(container.querySelector("button")!);
    const badge = document.querySelector('[data-testid="effort-badge"]');
    expect(badge).not.toBeNull();
    expect(badge!.textContent).toBe("high");
  });

  it("omits effort badge when effort is null", () => {
    mount({ effort: null });
    click(container.querySelector("button")!);
    expect(document.querySelector('[data-testid="effort-badge"]')).toBeNull();
  });

  it("shows effort badge only on the selected row, not all rows", () => {
    mount({ effort: "high" });
    click(container.querySelector("button")!);
    const badges = document.querySelectorAll('[data-testid="effort-badge"]');
    expect(badges).toHaveLength(1);
  });
});

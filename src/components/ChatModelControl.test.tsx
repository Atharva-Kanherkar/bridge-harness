// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ChatModelControl, modelDisplayName } from "./ChatModelControl";
import type { AdapterDescriptor } from "../types";

let container: HTMLDivElement;
let root: Root;

const adapters: AdapterDescriptor[] = [
  {
    id: "codex", label: "Codex", available: true, authState: "signed_in", version: "test", capabilities: [], unavailableReason: null,
    models: [{ id: "gpt-balanced", label: "GPT Balanced", tier: "standard", defaultForTier: true }], defaultModel: "gpt-balanced",
  },
  {
    id: "opencode", label: "OpenCode", available: true, authState: "signed_in", version: "test", capabilities: [], unavailableReason: null,
    models: [{ id: "ox-alpha-free", label: "Ox Alpha Free (Unlimited)", tier: "fast", defaultForTier: true }], defaultModel: "ox-alpha-free",
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
// The session toolbar is the top row of an overflow-hidden container: a panel
// anchored to the trigger's top edge there lands above the viewport and is
// clipped away entirely, which makes the pill look inert.
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

  // Some catalogs bake the provider into the model label; prefixing the
  // harness again stuttered: "OpenCode · OpenCode Go · MiMo V2.5".
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
});

describe("modelDisplayName", () => {
  it("strips a trailing (Unlimited) suffix case-insensitively", () => {
    expect(modelDisplayName(adapters, "opencode", "ox-alpha-free")).toBe("Ox Alpha Free");
  });

  it("falls back to Automatic when no model is configured", () => {
    expect(modelDisplayName(adapters, "codex", null)).toBe("Automatic");
  });
});

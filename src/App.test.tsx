// @vitest-environment jsdom
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { describe, expect, it, vi } from "vitest";
import { ChatModelControl } from "./App";
import type { AdapterDescriptor } from "./types";

const adapters: AdapterDescriptor[] = [
  {
    id: "codex", label: "Codex", available: true, version: "test", capabilities: [], unavailableReason: null,
    models: [{ id: "gpt-balanced", label: "GPT Balanced", tier: "standard", defaultForTier: true }], defaultModel: "gpt-balanced",
  },
  {
    id: "claude", label: "Claude", available: true, version: "test", capabilities: [], unavailableReason: null,
    models: [{ id: "opus", label: "Claude Opus", tier: "strong", defaultForTier: true }], defaultModel: "opus",
  },
];

describe("ChatModelControl", () => {
  it("shows the exact orchestrator runtime and explains a model switch", async () => {
    (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
    const onChange = vi.fn();
    const container = document.createElement("div");
    const root = createRoot(container);
    await act(async () => root.render(<ChatModelControl adapters={adapters} harness="codex" model="gpt-balanced" roleLabel="Orchestrator" compact onChange={onChange} />));

    const trigger = container.querySelector<HTMLButtonElement>('button[aria-label="Orchestrator model: Codex GPT Balanced"]')!;
    expect(trigger.textContent).toContain("Codex · GPT Balanced");
    await act(async () => trigger.click());
    expect(container.textContent).toContain("Switching starts a fresh provider session");

    const opus = [...container.querySelectorAll("button")].find(button => button.textContent?.includes("Claude Opus"))!;
    await act(async () => opus.click());
    expect(onChange).toHaveBeenCalledWith("claude", "opus");
    await act(async () => root.unmount());
  });
});

// The Cursor sidebar mock renders invented repos and no session list, so with
// its flag on the shipped app loses the real chats and every route to the Work
// board. Nothing else mounts App, so the flag is asserted from source here.
describe("shell flags", () => {
  it("ships the real rail, not the Cursor sidebar mock", () => {
    const source = readFileSync(join(__dirname, "App.tsx"), "utf8");
    expect(source).toContain("const SHOW_CURSOR_SIDEBAR_MOCK = false;");
  });
});

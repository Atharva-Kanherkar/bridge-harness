// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { MotionGlobalConfig } from "framer-motion";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { bridgeApi } from "../api";
import type { AdapterDescriptor } from "../types";
import type { CacheDiagnostic, UsageHistoryEntry } from "../usage";
import { UsageWidget } from "./UsageWidget";

const adapters: AdapterDescriptor[] = [
  { id: "codex", label: "Codex", available: true, authState: "signed_out", version: "mock", capabilities: [], unavailableReason: null, models: [] },
  { id: "claude", label: "Claude", available: true, authState: "signed_in", version: "mock", capabilities: [], unavailableReason: null, models: [] },
  { id: "cursor", label: "Cursor", available: true, authState: "signed_in", version: "mock", capabilities: [], unavailableReason: null, models: [] },
  { id: "opencode", label: "OpenCode", available: true, authState: "signed_in", version: "mock", capabilities: [], unavailableReason: null, models: [] },
];

function signInButtons(container: HTMLElement): HTMLButtonElement[] {
  return [...container.querySelectorAll("button")].filter(candidate => candidate.textContent?.trim() === "Sign in");
}

describe("UsageWidget sign-in control", () => {
  let container: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
    container = document.createElement("div");
    document.body.append(container);
    root = createRoot(container);
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
    vi.restoreAllMocks();
  });

  it("renders the control only on the signed-out provider's row and starts login with its provider id", async () => {
    const startLogin = vi.spyOn(bridgeApi, "startProviderLogin").mockResolvedValue({ workspaceId: "provider-login", terminalId: "codex" });
    await act(async () => {
      root.render(<UsageWidget usage={{}} adapters={adapters} />);
    });

    const controls = signInButtons(container);
    expect(controls).toHaveLength(1);

    await act(async () => { controls[0].click(); });
    await act(async () => {});
    expect(startLogin).toHaveBeenCalledWith("codex");
  });

  it("renders no sign-in control when every provider is signed in", async () => {
    const signedIn = adapters.map(adapter => ({ ...adapter, authState: "signed_in" as const }));
    await act(async () => {
      root.render(<UsageWidget usage={{}} adapters={signedIn} />);
    });
    expect(signInButtons(container)).toHaveLength(0);
  });

  it("opens an inline pane that forwards entered lines to the provider login PTY", async () => {
    vi.spyOn(bridgeApi, "startProviderLogin").mockResolvedValue({ workspaceId: "provider-login", terminalId: "codex" });
    const writeTerminal = vi.spyOn(bridgeApi, "writeTerminal").mockResolvedValue(undefined);
    await act(async () => {
      root.render(<UsageWidget usage={{}} adapters={adapters} />);
    });

    await act(async () => { signInButtons(container)[0].click(); });

    const output = container.querySelector('[aria-label="Codex sign-in output"]');
    expect(output).not.toBeNull();

    const input = container.querySelector<HTMLInputElement>('input[aria-label="Reply to the Codex sign-in prompt"]');
    expect(input).not.toBeNull();
    await act(async () => {
      const setValue = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!;
      setValue.call(input!, "123456");
      input!.dispatchEvent(new Event("input", { bubbles: true }));
    });
    // Enter forwards the line. The pane is intentionally not a <form> (it
    // renders inside the composer's form via the `trailing` slot), so there is
    // no submit event to dispatch — the key handler is the contract.
    await act(async () => {
      input!.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
    });
    expect(writeTerminal).toHaveBeenCalledWith("provider-login", "codex", "123456\r");
  });

  it("starts the vendor process only after both PTY subscriptions are registered", async () => {
    let resolveOutput: (fn: () => void) => void;
    let resolveExit: (fn: () => void) => void;
    vi.spyOn(bridgeApi, "onTerminal").mockImplementation(() => new Promise(resolve => { resolveOutput = resolve; }));
    vi.spyOn(bridgeApi, "onTerminalExited").mockImplementation(() => new Promise(resolve => { resolveExit = resolve; }));
    const startLogin = vi.spyOn(bridgeApi, "startProviderLogin").mockResolvedValue({ workspaceId: "provider-login", terminalId: "codex" });
    await act(async () => {
      root.render(<UsageWidget usage={{}} adapters={adapters} />);
    });
    await act(async () => { signInButtons(container)[0].click(); });
    await act(async () => {});
    expect(startLogin).not.toHaveBeenCalled();

    await act(async () => { resolveOutput!(() => undefined); });
    await act(async () => {});
    expect(startLogin).not.toHaveBeenCalled();

    await act(async () => { resolveExit!(() => undefined); });
    await act(async () => {});
    expect(startLogin).toHaveBeenCalledWith("codex");
  });

  it("shows an inline error instead of closing when the flow cannot start", async () => {
    vi.spyOn(bridgeApi, "onTerminal").mockResolvedValue(() => undefined);
    vi.spyOn(bridgeApi, "onTerminalExited").mockResolvedValue(() => undefined);
    vi.spyOn(bridgeApi, "startProviderLogin").mockRejectedValue(new Error("no cli"));
    await act(async () => {
      root.render(<UsageWidget usage={{}} adapters={adapters} />);
    });
    await act(async () => { signInButtons(container)[0].click(); });
    await act(async () => {});

    expect(container.querySelector('[aria-label="Codex sign-in output"]')).not.toBeNull();
    expect(container.querySelector('[role="alert"]')?.textContent).toContain("could not be started");
  });

  it("closes the pane and keeps the control reachable when the vendor process exits", async () => {
    vi.spyOn(bridgeApi, "startProviderLogin").mockResolvedValue({ workspaceId: "provider-login", terminalId: "codex" });
    let emitExit: ((exit: { sessionId: string; terminalId: string }) => void) | undefined;
    vi.spyOn(bridgeApi, "onTerminalExited").mockImplementation(async handler => {
      emitExit = handler;
      return () => undefined;
    });
    vi.spyOn(bridgeApi, "onTerminal").mockResolvedValue(() => undefined);
    await act(async () => {
      root.render(<UsageWidget usage={{}} adapters={adapters} />);
    });
    await act(async () => { signInButtons(container)[0].click(); });
    expect(container.querySelector('[aria-label="Codex sign-in output"]')).not.toBeNull();

    await act(async () => { emitExit?.({ sessionId: "provider-login", terminalId: "codex" }); });
    expect(container.querySelector('[aria-label="Codex sign-in output"]')).toBeNull();
  });
});

describe("UsageWidget panel shell", () => {
  let container: HTMLDivElement;
  let root: Root;

  const cacheFixture = (index: number, overrides: Partial<CacheDiagnostic> = {}): CacheDiagnostic => ({
    key: `cache-${index}`, harness: "codex", model: `gpt-${index}`, role: "worker:implementation",
    taskFamily: "implementation", restorationMode: "checkpoint_restored",
    cacheReadTokens: 1, cacheWriteTokens: 0, uncachedInputTokens: 1,
    observations: 1, crossHarnessReuse: [], costSources: [], costCoverage: "unknown", ...overrides,
  });

  const details = () => container.querySelector<HTMLElement>("#usage-health-details");
  const toggle = () => [...container.querySelectorAll("button")].find(button => /Show (more|less)/.test(button.textContent ?? ""))!;
  /** Let Framer's frame loop run so a finished exit actually unmounts. */
  const settle = async () => { await act(async () => { await new Promise(resolve => setTimeout(resolve, 40)); }); };

  beforeEach(() => {
    (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
    // The disclosure body animates its height through Framer; `skipAnimations`
    // collapses the frames so the assertions are about structure, not timing.
    MotionGlobalConfig.skipAnimations = true;
    container = document.createElement("div");
    document.body.append(container);
    root = createRoot(container);
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
    MotionGlobalConfig.skipAnimations = false;
  });

  it("is a flat surface that scrolls its own content rather than clipping it", async () => {
    await act(async () => {
      root.render(<UsageWidget compact usage={{}} />);
    });
    const card = container.querySelector<HTMLElement>("#usage-health-panel > div")!;
    expect(card.className).toContain("bg-popover");
    expect(card.className).toContain("border-border");
    // No elevation: nothing here carries a drop shadow.
    expect(card.className).not.toContain("u-overlay");
    expect(card.className).not.toContain("u-glass");
    expect(card.className).not.toContain("shadow");
    // Content-sized, capped at the viewport, scrolling inside the cap.
    expect(card.className).toContain("max-h-[80dvh]");
    const scroller = card.firstElementChild as HTMLElement;
    expect(scroller.className).toContain("overflow-y-auto");
    expect(scroller.className).toContain("min-h-0");
    // The drag-to-resize grip is gone in both modes.
    expect(container.querySelector('[aria-label="Resize usage panel"]')).toBeNull();
    expect(container.querySelector('[role="separator"]')).toBeNull();
  });

  it("caps and scrolls the non-compact panel the same way", async () => {
    await act(async () => {
      root.render(<UsageWidget usage={{}} />);
    });
    const card = container.querySelector<HTMLElement>("#usage-health-panel > div")!;
    expect(card.className).toContain("max-h-[80dvh]");
    expect(card.className).toContain("w-[390px]");
    expect(card.className).not.toContain("shadow");
    expect((card.firstElementChild as HTMLElement).className).toContain("overflow-y-auto");
  });

  it("mounts the cache and history sections only while Show more is on", async () => {
    await act(async () => {
      root.render(<UsageWidget compact usage={{}} />);
    });
    expect(details()).toBeNull();
    expect(toggle().getAttribute("aria-expanded")).toBe("false");
    expect(toggle().getAttribute("aria-controls")).toBe("usage-health-details");

    await act(async () => { toggle().click(); });
    expect(toggle().textContent).toContain("Show less");
    expect(toggle().getAttribute("aria-expanded")).toBe("true");
    const body = details();
    expect(body).not.toBeNull();
    // The wrapper Framer animates has to hide the overflow while it grows.
    expect(body?.className).toContain("overflow-hidden");
    expect(body?.textContent).toContain("Prompt cache");
    expect(body?.textContent).toContain("Recent work units");

    await act(async () => { toggle().click(); });
    await settle();
    expect(details()).toBeNull();
    expect(toggle().textContent).toContain("Show more");
    expect(toggle().getAttribute("aria-expanded")).toBe("false");
  });

  it("renders cache ratios, prefix provenance, and unknown provider cost without fake savings", async () => {
    const cache = cacheFixture(0, {
      key: "codex-cache", model: "gpt-5", restorationMode: "fresh",
      stablePrefixId: "bridge-prompt-v1-deadbeef", stablePrefixHash: "deadbeef",
      promptSchemaVersion: 1, prefixTokenEstimate: 100,
      cacheReadTokens: 120, cacheWriteTokens: 20, uncachedInputTokens: 160,
      cacheHitRatio: 0.4, writeAmortization: 6, observations: 2,
      crossHarnessReuse: ["same_harness"],
    });
    await act(async () => {
      root.render(<UsageWidget usage={{}} cacheDiagnostics={[cache]} />);
    });
    await act(async () => { toggle().click(); });
    const body = details()!;
    expect(body.textContent).toContain("Prompt cache");
    expect(body.textContent).toContain("Hit 40%");
    expect(body.textContent).toContain("write amortization 6.0×");
    expect(body.textContent).toContain("bridge-prompt-v1-deadbeef");
    expect(body.textContent).toContain("schema v1");
    expect(body.textContent).toContain("Role: Worker · implementation");
    expect(body.textContent).toContain("Restore: Fresh");
    expect(body.textContent).toContain("Reuse: Same harness");
    expect(body.textContent).toContain("Cost unknown — provider did not report it");
    expect(container.innerHTML.toLowerCase()).not.toContain("savings");
  });

  it("discloses when additional prompt groups are hidden, and renders work-unit history", async () => {
    const history: UsageHistoryEntry[] = [{ id: 1, workUnit: "turn-51", harness: "codex", model: "gpt-5", outcome: "completed", source: "reported", totalTokens: 150, contextPercent: 45, createdAt: "2026-07-16T10:00:00Z" }];
    await act(async () => {
      root.render(<UsageWidget usage={{}} history={history} cacheDiagnostics={Array.from({ length: 7 }, (_, index) => cacheFixture(index))} />);
    });
    await act(async () => { toggle().click(); });
    const body = details()!;
    expect(body.textContent).toContain("Showing 6 of 7 recent prompt groups.");
    expect(body.textContent).toContain("Restore: Checkpoint restored");
    expect(body.textContent).toContain("turn-51");
    expect(body.textContent).toContain("completed");
  });

  it("portals the compact panel onto the composer frame, flush with its top edge", async () => {
    await act(async () => {
      root.render(
        <div data-composer-frame className="relative">
          <UsageWidget compact usage={{}} />
        </div>,
      );
    });
    const panel = container.querySelector<HTMLElement>("#usage-health-panel")!;
    expect(panel.parentElement?.hasAttribute("data-composer-frame")).toBe(true);
    expect(panel.className).toContain("inset-x-0");
    expect(panel.className).toContain("bottom-full");
    // Flush: nothing lifts the popup off the composer's top edge.
    expect(panel.className).not.toContain("mb-2");
    expect(panel.className).not.toContain("pb-2");
    // It still sits above the transcript.
    expect(panel.className).toContain("z-50");
    expect(container.querySelector('[aria-label="Resize usage panel"]')).toBeNull();
  });
});

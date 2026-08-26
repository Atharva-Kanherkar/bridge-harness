// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { bridgeApi } from "../api";
import type { AdapterDescriptor } from "../types";
import { UsageWidget } from "./UsageWidget";

const adapters: AdapterDescriptor[] = [
  { id: "codex", label: "Codex", available: true, authState: "signed_out", version: "mock", capabilities: [], unavailableReason: null, models: [] },
  { id: "claude", label: "Claude", available: true, authState: "signed_in", version: "mock", capabilities: [], unavailableReason: null, models: [] },
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
    await act(async () => {
      input!.form!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
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

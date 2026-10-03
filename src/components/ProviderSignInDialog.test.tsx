// @vitest-environment jsdom
// A sign-in raised by a failed turn must say how it ended, not just vanish.
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { bridgeApi } from "../api";
import type { Health, TerminalChunk, TerminalExit } from "../types";
import { ProviderSignInDialog } from "./ProviderSignInDialog";

let container: HTMLDivElement;
let root: Root;
let emitChunk: (chunk: TerminalChunk) => void;
let emitExit: (exit: TerminalExit) => void;
const flush = async () => { await act(async () => { await new Promise(resolve => setTimeout(resolve, 0)); }); };
const buttonByText = (needle: string) =>
  [...document.querySelectorAll<HTMLButtonElement>("button")].find(button => button.textContent?.trim() === needle);
const healthWith = (authState: "signed_in" | "signed_out" | "unknown") =>
  ({ adapters: [{ id: "claude", authState }] }) as unknown as Health;

beforeEach(() => {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
  vi.spyOn(bridgeApi, "onTerminal").mockImplementation(async handler => { emitChunk = handler; return () => undefined; });
  vi.spyOn(bridgeApi, "onTerminalExited").mockImplementation(async handler => { emitExit = handler; return () => undefined; });
  vi.spyOn(bridgeApi, "startProviderLogin").mockResolvedValue({ workspaceId: "provider-login", terminalId: "claude" });
  vi.spyOn(bridgeApi, "cancelProviderLogin").mockResolvedValue();
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.restoreAllMocks();
});

async function open(props: { onRetry?: () => void; onClose?: () => void; onAuthChanged?: () => void } = {}) {
  act(() => {
    root.render(<ProviderSignInDialog provider="claude" label="Claude Code" onAuthChanged={props.onAuthChanged ?? (() => undefined)} onClose={props.onClose ?? (() => undefined)} onRetry={props.onRetry} />);
  });
  await flush();
}

async function finish(output = "Login successful.") {
  act(() => emitChunk({ sessionId: "provider-login", terminalId: "claude", data: output } as TerminalChunk));
  act(() => emitExit({ sessionId: "provider-login", terminalId: "claude" }));
  await flush();
}

describe("provider sign-in dialog", () => {
  it("shows one header and the sign-in link while the flow runs", async () => {
    await open();
    act(() => emitChunk({ sessionId: "provider-login", terminalId: "claude", data: "Visit https://claude.ai/oauth/authorize?code=1 to sign in" } as TerminalChunk));
    expect(document.body.textContent).toContain("Sign in to Claude Code");
    expect(document.body.textContent).not.toContain("Connect Claude Code");
    expect(document.querySelector('a[href^="https://claude.ai/oauth"]')?.textContent).toContain("Open sign-in page");
    expect(document.querySelector('input[aria-label="Reply to the Claude Code sign-in prompt"]')).not.toBeNull();
  });

  it("confirms the sign-in and offers the retry once the provider reports signed in", async () => {
    vi.spyOn(bridgeApi, "health").mockResolvedValue(healthWith("signed_in"));
    const onRetry = vi.fn();
    const onClose = vi.fn();
    const onAuthChanged = vi.fn();
    await open({ onRetry, onClose, onAuthChanged });
    await finish();
    expect(document.body.textContent).toContain("Signed in to Claude Code");
    expect(onAuthChanged).toHaveBeenCalled();
    act(() => buttonByText("Retry message")!.click());
    expect(onClose).toHaveBeenCalled();
    expect(onRetry).toHaveBeenCalledTimes(1);
  });

  it("says Done instead of offering a retry when there is nothing to resend", async () => {
    vi.spyOn(bridgeApi, "health").mockResolvedValue(healthWith("unknown"));
    await open();
    await finish();
    expect(document.body.textContent).toContain("Signed in to Claude Code");
    expect(buttonByText("Retry message")).toBeUndefined();
    expect(buttonByText("Done")).toBeDefined();
  });

  it("reports a failed sign-in when the provider is still signed out", async () => {
    vi.spyOn(bridgeApi, "health").mockResolvedValue(healthWith("signed_out"));
    await open({ onRetry: vi.fn() });
    await finish();
    expect(document.body.textContent).toContain("Sign-in didn't finish");
    expect(buttonByText("Retry message")).toBeUndefined();
    act(() => buttonByText("Try again")!.click());
    await flush();
    expect(document.body.textContent).toContain("Sign in to Claude Code");
    expect(bridgeApi.startProviderLogin).toHaveBeenCalledTimes(2);
  });

  it("trusts the vendor's own failure text over a stale signed-in probe", async () => {
    const health = vi.spyOn(bridgeApi, "health").mockResolvedValue(healthWith("signed_in"));
    await open();
    await finish("OAuth error: authorization denied");
    expect(document.body.textContent).toContain("Sign-in didn't finish");
    expect(health).not.toHaveBeenCalled();
  });

  it("cancels the vendor process when the user cancels", async () => {
    const onClose = vi.fn(() => {
      root.render(<ProviderSignInDialog provider={null} label="" onAuthChanged={() => undefined} onClose={() => undefined} />);
    });
    await open({ onClose });
    act(() => buttonByText("Cancel")!.click());
    await flush();
    expect(onClose).toHaveBeenCalled();
    expect(bridgeApi.cancelProviderLogin).toHaveBeenCalledWith("claude");
  });
});

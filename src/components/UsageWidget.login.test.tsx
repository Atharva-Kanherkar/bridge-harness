// @vitest-environment jsdom
// Regression: the provider sign-in reply must stay local to the provider
// terminal. The widget renders through the composer's `trailing` slot, inside
// the composer's <form>, so the login pane must NOT itself be a nested form —
// a nested form's submit bubbles and would also fire the composer's onSubmit,
// sending the chat/steer draft. See UsageWidget.tsx ProviderLoginPane.
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { bridgeApi } from "../api";
import { UsageWidget } from "./UsageWidget";
import type { AdapterDescriptor, AuthState } from "../types";

const cursorSignedOut: AdapterDescriptor = {
  id: "cursor", label: "Cursor", available: true, authState: "signed_out" as AuthState,
  version: "mock", capabilities: [], unavailableReason: null, models: [],
};

let container: HTMLDivElement;
let root: Root;
const flush = async () => { await act(async () => {}); };
const click = (element: Element) => { act(() => { element.dispatchEvent(new MouseEvent("click", { bubbles: true })); }); };
const buttonByText = (needle: string) =>
  [...document.querySelectorAll<HTMLButtonElement>("button")].find(button => button.textContent?.trim() === needle)!;
const typeInto = (input: HTMLInputElement, value: string) => {
  const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!;
  act(() => { setter.call(input, value); input.dispatchEvent(new Event("input", { bubbles: true })); });
};

beforeEach(() => {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
  vi.spyOn(bridgeApi, "onTerminal").mockResolvedValue(() => undefined);
  vi.spyOn(bridgeApi, "onTerminalExited").mockResolvedValue(() => undefined);
  vi.spyOn(bridgeApi, "startProviderLogin").mockResolvedValue({ workspaceId: "provider-login", terminalId: "cursor" });
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.restoreAllMocks();
});

describe("provider sign-in inside a composer form", () => {
  const openLogin = async (onSubmit: () => void) => {
    act(() => {
      // The widget lives inside the composer's own <form>, exactly as the
      // `trailing` slot mounts it.
      root.render(<form onSubmit={onSubmit}><UsageWidget usage={{}} adapters={[cursorSignedOut]} /></form>);
    });
    await flush();
    click(buttonByText("Sign in"));
    await flush();
    return document.querySelector<HTMLInputElement>('input[aria-label="Reply to the Cursor sign-in prompt"]')!;
  };

  it("sends the reply to the terminal without submitting the composer form (Send button)", async () => {
    const write = vi.spyOn(bridgeApi, "writeTerminal").mockResolvedValue();
    const onSubmit = vi.fn();
    const input = await openLogin(onSubmit);
    expect(input).toBeTruthy();
    typeInto(input, "auth-code-123");
    click(buttonByText("Send"));
    await flush();
    expect(write).toHaveBeenCalledWith("provider-login", "cursor", "auth-code-123\r");
    expect(onSubmit).not.toHaveBeenCalled();
  });

  it("sends on Enter without submitting the composer form", async () => {
    const write = vi.spyOn(bridgeApi, "writeTerminal").mockResolvedValue();
    const onSubmit = vi.fn();
    const input = await openLogin(onSubmit);
    typeInto(input, "paste-url");
    act(() => { input.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true })); });
    await flush();
    expect(write).toHaveBeenCalledWith("provider-login", "cursor", "paste-url\r");
    expect(onSubmit).not.toHaveBeenCalled();
  });

  // Regression: Cancel used to only unmount the pane, leaving the vendor login
  // PTY alive. `start_provider_login` then reattached to that runtime without
  // replaying the URL and prompts the closed pane missed, so the retry showed
  // an empty, unusable terminal.
  it("stops the provider login process when the pane is cancelled", async () => {
    const cancel = vi.spyOn(bridgeApi, "cancelProviderLogin").mockResolvedValue();
    await openLogin(vi.fn());
    click(buttonByText("Cancel"));
    await flush();
    expect(cancel).toHaveBeenCalledWith("cursor");
    expect(document.querySelector('input[aria-label="Reply to the Cursor sign-in prompt"]')).toBeNull();
  });

  it("renders no nested form element in the login pane", async () => {
    const input = await openLogin(vi.fn());
    // The pane must not reintroduce a <form>; the outer test form is the only one.
    expect(document.querySelectorAll("form")).toHaveLength(1);
    expect(input.closest("form")?.parentElement).toBeTruthy();
  });
});

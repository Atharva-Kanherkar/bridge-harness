// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { AsideChat } from "./AsideChat";
import type { Session } from "../types";

let container: HTMLDivElement;
let root: Root;

const aside: Session = { id: "aside-1", workspaceId: null, harness: "claude", label: "is this right?", status: "working", startedAt: "now", endedAt: null, contextPercent: null, usagePercent: null, metricSource: "estimated", providerSessionId: null, activeTurnId: "t1", model: "sonnet", requestedTier: "fast", restorationMode: "fresh", continuationFidelity: "native", title: "is this right?", kind: "direct" };

const noop = () => undefined;
const asyncNoop = async () => undefined;

async function mount(overrides: Partial<Parameters<typeof AsideChat>[0]> = {}) {
  await act(async () => root.render(
    <AsideChat
      session={aside}
      events={[]}
      pendingMessages={["is this right?"]}
      working
      onSend={asyncNoop}
      onResolve={noop}
      onPromote={noop}
      onClose={noop}
      {...overrides}
    />,
  ));
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

const dialog = () => container.querySelector<HTMLElement>('div[role="dialog"]')!;

describe("AsideChat", () => {
  it("wears the harness's tinted mark and names the delegation", async () => {
    await mount();
    expect(dialog().getAttribute("aria-label")).toBe("Aside with Claude");
    expect(dialog().innerHTML).toContain("text-harness-claude");
    expect(dialog().textContent).toContain("is this right?");
    expect(dialog().textContent).toContain("Claude · Sonnet · aside");
  });

  it("closes on Escape and on the scrim, but never from inside the panel", async () => {
    const onClose = vi.fn();
    await mount({ onClose });
    await act(async () => { window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true })); });
    expect(onClose).toHaveBeenCalledTimes(1);
    const scrim = container.querySelector<HTMLElement>('div[role="presentation"]')!;
    await act(async () => { scrim.dispatchEvent(new MouseEvent("mousedown", { bubbles: true })); });
    expect(onClose).toHaveBeenCalledTimes(2);
    // A click that starts inside the panel must not close it.
    await act(async () => { dialog().dispatchEvent(new MouseEvent("mousedown", { bubbles: true })); });
    expect(onClose).toHaveBeenCalledTimes(2);
  });

  it("promotes through its header button", async () => {
    const onPromote = vi.fn();
    await mount({ onPromote });
    const promote = [...container.querySelectorAll("button")].find(button => button.textContent?.includes("Open as chat"))!;
    await act(async () => { promote.click(); });
    expect(onPromote).toHaveBeenCalledTimes(1);
  });

  it("sends a follow-up on Enter and clears the box", async () => {
    const onSend = vi.fn(async () => undefined);
    await mount({ onSend });
    const box = container.querySelector<HTMLTextAreaElement>("textarea")!;
    await act(async () => {
      const setter = Object.getOwnPropertyDescriptor(window.HTMLTextAreaElement.prototype, "value")!.set!;
      setter.call(box, "and the failure mode?");
      box.dispatchEvent(new Event("input", { bubbles: true }));
    });
    await act(async () => {
      box.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true }));
    });
    expect(onSend).toHaveBeenCalledWith("and the failure mode?");
    expect(box.value).toBe("");
  });

  it("routes an approval inside the panel through the aside's resolver", async () => {
    const onResolve = vi.fn();
    await mount({
      onResolve,
      events: [{ id: 7, sessionId: "aside-1", sequence: 7, protocolVersion: 1, kind: "approval.requested", itemId: null, role: null, status: "pending", title: "Run bun test", text: "bun test", data: {}, providerMeta: {}, createdAt: "now" }],
    });
    const approve = [...container.querySelectorAll("button")].find(button => /approve|allow|accept/i.test(button.textContent ?? ""))!;
    expect(approve).toBeTruthy();
    await act(async () => { approve.click(); });
    expect(onResolve).toHaveBeenCalled();
    expect(onResolve.mock.calls[0][0]).toBe(7);
  });
});

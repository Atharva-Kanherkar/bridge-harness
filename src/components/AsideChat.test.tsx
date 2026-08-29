// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { AsideChat } from "./AsideChat";
import { bridgeApi } from "../api";
import type { ComposerAttachment } from "../pasteAttachments";
import type { AdapterDescriptor, AgentEvent, Session } from "../types";

let container: HTMLDivElement;
let root: Root;

const aside: Session = { id: "aside-1", workspaceId: null, harness: "claude", label: "is this right?", status: "working", startedAt: "now", endedAt: null, contextPercent: null, usagePercent: null, metricSource: "estimated", providerSessionId: null, activeTurnId: "t1", model: "sonnet", requestedTier: "fast", restorationMode: "fresh", continuationFidelity: "native", title: "is this right?", kind: "direct" };

const adapters: AdapterDescriptor[] = [
  { id: "claude", label: "Claude", available: true, authState: "signed_in", version: "test", capabilities: [], sandboxModes: [], unavailableReason: null, defaultModel: "sonnet", models: [
    { id: "sonnet", label: "Sonnet", tier: "standard", defaultForTier: true },
    { id: "opus", label: "Opus", tier: "strong", defaultForTier: true },
  ] },
];

const noop = () => undefined;
const asyncNoop = async () => undefined;

async function mount(overrides: Partial<Parameters<typeof AsideChat>[0]> = {}) {
  await act(async () => root.render(
    <AsideChat
      session={aside}
      adapters={adapters}
      events={[]}
      pendingMessages={["is this right?"]}
      working
      onSend={asyncNoop}
      onChangeModel={noop}
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

const makeEvent = (sequence: number): AgentEvent => ({
  id: sequence, sessionId: "aside-1", sequence, protocolVersion: 1, kind: "assistant.delta", itemId: null, role: "assistant", status: null, title: null, text: "…", data: {}, providerMeta: {}, createdAt: "now",
});

describe("AsideChat", () => {
  it("wears the harness's tinted mark and names the delegation", async () => {
    await mount();
    expect(dialog().getAttribute("aria-label")).toBe("Aside with Claude");
    expect(dialog().innerHTML).toContain("text-harness-claude");
    expect(dialog().textContent).toContain("is this right?");
    // The model now lives in an interactive control, followed by the aside tag.
    expect(dialog().querySelector('[aria-label="Aside model: Claude Sonnet"]')).toBeTruthy();
    expect(dialog().textContent).toContain("aside");
  });

  it("switches the side chat's model through its header control", async () => {
    const onChangeModel = vi.fn();
    await mount({ working: false, onChangeModel });
    const pill = dialog().querySelector<HTMLButtonElement>('[aria-label="Aside model: Claude Sonnet"]')!;
    await act(async () => { pill.click(); });
    const opus = [...dialog().querySelectorAll("button")].find(button => button.textContent?.includes("Opus"))!;
    await act(async () => { opus.click(); });
    expect(onChangeModel).toHaveBeenCalledWith("claude", "opus");
  });

  it("wears a failed model switch inside the panel, not the banner behind it", async () => {
    const onChangeModel = vi.fn(async () => { throw new Error("Wait for the current response before switching models"); });
    await mount({ working: false, onChangeModel });
    const pill = dialog().querySelector<HTMLButtonElement>('[aria-label="Aside model: Claude Sonnet"]')!;
    await act(async () => { pill.click(); });
    const opus = [...dialog().querySelectorAll("button")].find(button => button.textContent?.includes("Opus"))!;
    await act(async () => { opus.click(); });
    expect(dialog().textContent).toContain("Wait for the current response before switching models");
  });

  it("narrates a model switch in flight and locks the picker for its duration", async () => {
    await mount({ working: false, modelSwitch: { harness: "claude", label: "Opus" } });
    expect(dialog().textContent).toContain("Switching to Opus…");
    const pill = dialog().querySelector<HTMLButtonElement>('[aria-label="Aside model: Claude Sonnet"]')!;
    expect(pill.disabled).toBe(true);
  });

  it("offers Steer mid-turn when the aside's harness advertises steering", async () => {
    const steering = [{ ...adapters[0], capabilities: ["steering"] }];
    await mount({ adapters: steering, working: true });
    expect([...container.querySelectorAll("button")].some(button => button.textContent?.trim() === "Steer")).toBe(true);
  });

  it("keeps Queue mid-turn when the harness cannot steer", async () => {
    await mount({ working: true });
    expect([...container.querySelectorAll("button")].some(button => button.textContent?.trim() === "Queue")).toBe(true);
  });

  it("disables the model control while the aside is working", async () => {
    await mount({ working: true });
    const pill = dialog().querySelector<HTMLButtonElement>('[aria-label="Aside model: Claude Sonnet"]')!;
    expect(pill.disabled).toBe(true);
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
    expect(onSend).toHaveBeenCalledWith("and the failure mode?", []);
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

  it("attaches a pasted image as a removable chip and sends it with the message", async () => {
    const onSend = vi.fn(async (_text: string, _attachments?: ComposerAttachment[]) => undefined);
    await mount({ onSend });
    const box = container.querySelector<HTMLTextAreaElement>("textarea")!;

    const file = new File(["fake-image-bytes"], "shot.png", { type: "image/png" });
    const items = [{ kind: "file", type: "image/png", getAsFile: () => file }];
    const pasteEvent = new Event("paste", { bubbles: true, cancelable: true });
    Object.defineProperty(pasteEvent, "clipboardData", { value: { items } });
    await act(async () => { box.dispatchEvent(pasteEvent); });
    // Let the FileReader promise resolve into attachment state.
    await act(async () => { await new Promise(resolve => setTimeout(resolve, 50)); });

    expect(container.querySelectorAll("img")).toHaveLength(1);
    expect(container.querySelector<HTMLButtonElement>('button[aria-label="Remove attached image"]')).toBeTruthy();

    await act(async () => {
      box.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true }));
    });
    expect(onSend).toHaveBeenCalledTimes(1);
    const [text, attachments] = onSend.mock.calls[0];
    expect(text).toBe("");
    expect(attachments).toHaveLength(1);
    expect(attachments![0].mediaType).toBe("image/png");
    // The chip clears once the send that carried it has gone out.
    expect(container.querySelectorAll("img")).toHaveLength(0);
  });

  it("polls the forest by digest instead of refetching the full snapshot on every streamed event", async () => {
    const forestSpy = vi.spyOn(bridgeApi, "sessionForest");
    const digestSpy = vi.spyOn(bridgeApi, "sessionForestDigest");
    await mount({ events: [] });
    // Let the initial digest -> full-snapshot chain settle.
    await act(async () => { await new Promise(resolve => setTimeout(resolve, 20)); });
    expect(forestSpy).toHaveBeenCalledTimes(1);
    expect(digestSpy).toHaveBeenCalledTimes(1);

    // A burst of streamed frames for the same session, same as a turn in
    // flight growing the live event list on every chunk. The old effect kept
    // `ownEvents.length` in its deps and refetched the whole snapshot on
    // each one; the digest-gated poll must not.
    for (let sequence = 1; sequence <= 5; sequence += 1) {
      const events = Array.from({ length: sequence }, (_, index) => makeEvent(index));
      await mount({ events });
    }
    await act(async () => { await new Promise(resolve => setTimeout(resolve, 20)); });

    expect(forestSpy).toHaveBeenCalledTimes(1);
    expect(digestSpy).toHaveBeenCalledTimes(1);
  });
});

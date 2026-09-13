// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import { bridgeApi } from "../api";
import { ConnectorPane } from "./ConnectorPane";
import type {
  ConnectorInboxItem,
  ConnectorInboxResult,
  ConnectorListResult,
} from "../protocol/generated/protocol";

(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let root: Root | undefined;
let host: HTMLDivElement | undefined;
const flush = async () => { for (let i = 0; i < 4; i += 1) await Promise.resolve(); };

const connected: ConnectorListResult = {
  connectors: [{ family: "slack", displayName: "Slack", server: "claude.ai Slack", available: true, reason: null, explanation: null }],
};

function item(overrides: Partial<ConnectorInboxItem> = {}): ConnectorInboxItem {
  return {
    itemKey: "slack:D09:1757756400.000100",
    family: "slack",
    channelId: "D09",
    channelLabel: "Nina Alvarez",
    author: "Nina Alvarez",
    kind: "directMessage",
    text: "can you look at the release checklist before standup?",
    permalink: null,
    receivedAt: new Date().toISOString(),
    state: "rendered",
    renderRejection: null,
    resolution: null,
    card: {
      itemKey: "slack:D09:1757756400.000100",
      headline: "Nina wants the release checklist reviewed",
      blocks: [
        { kind: "message", author: "Nina Alvarez", text: "can you look at the release checklist before standup?", timestamp: null },
        { kind: "summary", text: "She is blocked on the updater step." },
        { kind: "fact", label: "Channel", value: "Nina Alvarez" },
      ],
      suggestedReplies: ["On it — looking now."],
      harnessRendered: true,
    },
    ...overrides,
  };
}

function inbox(items: ConnectorInboxItem[], degraded: string | null = null): ConnectorInboxResult {
  return {
    items,
    unreadCount: items.filter(entry => entry.state !== "resolved").length,
    poll: [{ family: "slack", lastAttemptAt: "2026-09-13T09:00:00Z", lastSuccessAt: degraded ? null : "2026-09-13T09:00:00Z", degraded }],
  };
}

function stub(result: ConnectorInboxResult, connectors: ConnectorListResult = connected) {
  vi.spyOn(bridgeApi, "connectorInbox").mockResolvedValue(result);
  vi.spyOn(bridgeApi, "connectorList").mockResolvedValue(connectors);
  vi.spyOn(bridgeApi, "onConnectorInboxChanged").mockResolvedValue(() => undefined);
}

async function mount(node: React.ReactElement) {
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
  await act(async () => { root!.render(node); await flush(); });
  return host;
}

afterEach(() => {
  act(() => root?.unmount());
  host?.remove();
  root = undefined;
  host = undefined;
  vi.restoreAllMocks();
});

describe("ConnectorPane", () => {
  it("renders a card's blocks as text, never as markup", async () => {
    const hostile = item();
    hostile.card!.blocks = [
      { kind: "message", author: "Nina Alvarez", text: "<img src=x onerror=alert(1)>", timestamp: null },
    ];
    stub(inbox([hostile]));
    const node = await mount(<ConnectorPane />);
    // The closed block union means connector content reaches the DOM as a text
    // node. If this ever regresses to markup, this is where it shows up.
    expect(node.querySelector("img")).toBeNull();
    expect(node.textContent).toContain("<img src=x onerror=alert(1)>");
  });

  it("shows the headline, the summary, and the harness-rendered attribution", async () => {
    stub(inbox([item()]));
    const node = await mount(<ConnectorPane />);
    expect(node.textContent).toContain("Nina wants the release checklist reviewed");
    expect(node.textContent).toContain("She is blocked on the updater step.");
    expect(node.textContent).toContain("Rendered by your harness");
  });

  it("still shows the message while its card is in flight", async () => {
    stub(inbox([item({ state: "pending", card: null })]));
    const node = await mount(<ConnectorPane />);
    // The message already arrived; hiding it behind a spinner would trade a
    // real notification for a prettier one.
    expect(node.textContent).toContain("can you look at the release checklist before standup?");
    expect(node.querySelector(".connector-shimmer")).not.toBeNull();
  });

  it("disables the composer until a card is ready", async () => {
    stub(inbox([item({ state: "pending", card: null })]));
    const node = await mount(<ConnectorPane />);
    const box = node.querySelector("textarea") as HTMLTextAreaElement;
    expect(box.disabled).toBe(true);
  });

  it("opens Bridge's own confirmation instead of sending immediately", async () => {
    stub(inbox([item()]));
    const act_ = vi.spyOn(bridgeApi, "connectorAct").mockResolvedValue({
      status: "approvalRequired",
      effect: "Send to Nina Alvarez:\nOn it.",
    });
    const node = await mount(<ConnectorPane />);
    const box = node.querySelector("textarea") as HTMLTextAreaElement;
    await act(async () => {
      const setter = Object.getOwnPropertyDescriptor(window.HTMLTextAreaElement.prototype, "value")!.set!;
      setter.call(box, "On it.");
      box.dispatchEvent(new Event("input", { bubbles: true }));
      await flush();
    });
    const send = node.querySelector('button[aria-label="Send reply"]') as HTMLButtonElement;
    await act(async () => { send.click(); await flush(); });

    // The first call carried no decision, which is what makes it a request for
    // approval rather than a send.
    expect(act_).toHaveBeenCalledWith("slack:D09:1757756400.000100", { kind: "reply", text: "On it." }, undefined);
    const dialog = node.querySelector('[role="dialog"]');
    expect(dialog).not.toBeNull();
    // The sheet shows the host's sentence verbatim, not one assembled here.
    expect(dialog!.textContent).toContain("Send to Nina Alvarez");
    expect(dialog!.textContent).toContain("On it.");
  });

  it("sends nothing when the confirmation is cancelled", async () => {
    stub(inbox([item()]));
    const act_ = vi.spyOn(bridgeApi, "connectorAct").mockResolvedValue({
      status: "approvalRequired",
      effect: "Send to Nina Alvarez:\nOn it.",
    });
    const node = await mount(<ConnectorPane />);
    const suggestion = Array.from(node.querySelectorAll("button")).find(button => button.textContent === "On it — looking now.")!;
    await act(async () => { suggestion.click(); await flush(); });
    const send = node.querySelector('button[aria-label="Send reply"]') as HTMLButtonElement;
    await act(async () => { send.click(); await flush(); });

    act_.mockResolvedValue({ status: "refused", reason: "the action was denied" });
    const cancel = Array.from(node.querySelectorAll('[role="dialog"] button')).find(button => button.textContent?.includes("Cancel"))!;
    await act(async () => { (cancel as HTMLButtonElement).click(); await flush(); });

    // Denial goes back through the host so the refusal is recorded there too;
    // what must never happen is an approved send.
    expect(act_).toHaveBeenLastCalledWith("slack:D09:1757756400.000100", { kind: "reply", text: "On it — looking now." }, false);
    expect(act_.mock.calls.some(call => call[2] === true)).toBe(false);
  });

  it("fills the composer from a suggested reply without sending it", async () => {
    stub(inbox([item()]));
    const act_ = vi.spyOn(bridgeApi, "connectorAct").mockResolvedValue({ status: "sent", itemKey: "x" });
    const node = await mount(<ConnectorPane />);
    const suggestion = Array.from(node.querySelectorAll("button")).find(button => button.textContent === "On it — looking now.")!;
    await act(async () => { suggestion.click(); await flush(); });
    expect((node.querySelector("textarea") as HTMLTextAreaElement).value).toBe("On it — looking now.");
    expect(act_).not.toHaveBeenCalled();
  });

  it("says it could not read rather than showing an empty inbox", async () => {
    stub(inbox([], "provider timed out"));
    const node = await mount(<ConnectorPane />);
    expect(node.textContent).toContain("Couldn’t read your inbox");
    expect(node.textContent).toContain("provider timed out");
    expect(node.textContent).not.toContain("all caught up");
  });

  it("says caught up only after a check succeeded and found nothing", async () => {
    stub(inbox([]));
    const node = await mount(<ConnectorPane />);
    expect(node.textContent).toContain("You’re all caught up");
  });

  it("explains an unavailable connector instead of erroring", async () => {
    stub(inbox([]), {
      connectors: [{
        family: "slack", displayName: "Slack", server: "claude.ai Slack", available: false,
        reason: "authRequired", explanation: "Slack is configured but signed out. Sign in from your harness.",
      }],
    });
    const node = await mount(<ConnectorPane />);
    expect(node.textContent).toContain("Sign in from your harness");
    expect(node.textContent).not.toContain("Error");
  });

  it("notes when Bridge authored the card itself", async () => {
    stub(inbox([item({ renderRejection: "the render run did not return a JSON card" })]));
    const node = await mount(<ConnectorPane />);
    expect(node.textContent).toContain("Bridge wrote this card itself");
  });

  it("reports its unread count to the dock", async () => {
    const onUnreadChange = vi.fn();
    stub(inbox([item(), item({ itemKey: "b", state: "resolved", resolution: "replied" })]));
    await mount(<ConnectorPane onUnreadChange={onUnreadChange} />);
    expect(onUnreadChange).toHaveBeenLastCalledWith(1);
  });

  it("opens the item a toast deep-linked to", async () => {
    const other = item({ itemKey: "slack:C07:1757756100.000300", channelLabel: "#eng-alerts", author: "Devesh" });
    other.card = { ...other.card!, itemKey: other.itemKey, headline: "Devesh asked about notarisation" };
    stub(inbox([item(), other]));
    const node = await mount(<ConnectorPane focusItemKey={other.itemKey} />);
    // Without the deep link the pane would show the first unresolved item.
    expect(node.querySelector("article")!.textContent).toContain("Devesh asked about notarisation");
  });
});

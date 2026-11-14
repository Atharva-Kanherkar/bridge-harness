// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ConversationItem } from "../conversation";
import { MOCK_OUTGOING_ID, resetMockOutgoing } from "../outgoingMock";
import type { OutgoingMessage } from "../protocol/generated/protocol";
import { bridgeApi } from "../api";
import { OutgoingCard } from "./OutgoingCard";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const call = (overrides: Partial<ConversationItem> = {}, data: Record<string, unknown> = {}): ConversationItem => ({
  key: "k", identity: "i", type: "activity", eventId: 1, sequence: 1, turn: 1, text: "", status: "completed",
  data: {
    name: "mcp__bridge__outgoing_draft",
    input: { to: { slack: { channel: "#dev" } }, body: "x", reason: "r" },
    aggregatedOutput: JSON.stringify({ id: MOCK_OUTGOING_ID, state: "draft" }),
    ...data,
  },
  ...overrides,
});

const draft = (overrides: Partial<OutgoingMessage> = {}): OutgoingMessage => ({
  id: MOCK_OUTGOING_ID,
  sessionId: "session-1",
  destination: { kind: "slackChannel", channel: "#dev" },
  label: "Slack · #dev",
  format: "slack",
  body: "Deploy is done, all green",
  rendered: "Deploy is done, all green",
  lint: [{ rule: "no_em_dash", message: "Em dash replaced with a comma", line: 1, fixable: true, fixed: true }],
  state: "draft",
  revision: 1,
  reason: "tell the team",
  createdAt: "2026-11-08T09:00:00Z",
  updatedAt: "2026-11-08T09:00:00Z",
  ...overrides,
});

let container: HTMLDivElement;
let root: Root;
let clipboard: string[];

beforeEach(() => {
  clipboard = [];
  Object.defineProperty(navigator, "clipboard", {
    configurable: true,
    value: { writeText: (text: string) => { clipboard.push(text); return Promise.resolve(); } },
  });
  resetMockOutgoing([draft()]);
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.restoreAllMocks();
});

async function mount(item: ConversationItem = call(), readOnly = false) {
  await act(async () => {
    root.render(<OutgoingCard item={item} sessionId="session-1" readOnly={readOnly} />);
  });
  await act(async () => { await Promise.resolve(); });
}

const button = (name: string) => [...container.querySelectorAll("button")].find(node => node.textContent?.trim() === name);

async function click(name: string) {
  const target = button(name);
  expect(target, `button ${name}`).toBeTruthy();
  await act(async () => { target!.click(); });
  await act(async () => { await Promise.resolve(); });
}

describe("OutgoingCard", () => {
  it("shows a quiet placeholder while the call is still being written", async () => {
    await mount(call({ status: "inProgress" }, { aggregatedOutput: undefined }));
    expect(container.querySelector("[data-outgoing-state=drafting]")?.textContent).toContain("Drafting a message");
  });

  it("shows a refused call as one muted line that opens to the reason", async () => {
    await mount(call({}, { aggregatedOutput: "to must be exactly one of ..." }));
    const row = container.querySelector("[data-outgoing-state=refused]")!;
    expect(row.textContent).toContain("Message not drafted");
    await act(async () => { row.querySelector("button")!.click(); });
    expect(row.textContent).toContain("to must be exactly one of");
  });

  it("draws the destination, the rendered preview, style checks and Send", async () => {
    await mount();
    expect(container.querySelector("[data-outgoing-destination]")?.textContent).toBe("Slack · #dev");
    expect(container.querySelector("[data-outgoing-preview]")?.textContent).toBe("Deploy is done, all green");
    expect(container.querySelector("[data-finding=no_em_dash]")?.textContent).toContain("Em dash replaced with a comma");
    expect(button("Copy")).toBeTruthy();
    expect(button("Send")).toBeTruthy();
    expect(button("Fix"), "nothing left to fix").toBeUndefined();
  });

  it("offers Fix only for an unfixed fixable finding, and applies it", async () => {
    resetMockOutgoing([draft({
      body: "Done — green", rendered: "Done — green",
      lint: [{ rule: "no_em_dash", message: "Em dash", line: 1, fixable: true, fixed: false }],
    })]);
    await mount();
    await click("Fix");
    expect(container.querySelector("[data-outgoing-preview]")?.textContent).toBe("Done, green");
    expect(button("Fix")).toBeUndefined();
  });

  it("saves an edit against the revision the card showed", async () => {
    await mount();
    await click("Edit");
    const textarea = container.querySelector("textarea")!;
    await act(async () => {
      const setter = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")!.set!;
      setter.call(textarea, "Deploy is done.");
      textarea.dispatchEvent(new Event("input", { bubbles: true }));
    });
    await click("Save");
    expect(container.querySelector("textarea")).toBeNull();
    expect(container.querySelector("[data-outgoing-preview]")?.textContent).toBe("Deploy is done.");
  });

  it("shows the refusal when the draft changed underneath the editor", async () => {
    await mount();
    // The stored draft moves to revision 2 and the change event never
    // reaches this card, which still shows revision 1.
    resetMockOutgoing([draft({ revision: 2, body: "changed elsewhere", rendered: "changed elsewhere" })]);
    await click("Edit");
    await click("Save");
    expect(container.querySelector("[role=alert]")?.textContent).toContain("changed since you opened it");
  });

  it("copies the rendered text to the clipboard and marks it copied", async () => {
    await mount();
    await click("Copy");
    expect(clipboard).toEqual(["Deploy is done, all green"]);
    expect(container.querySelector("[data-outgoing-state=copied]")).toBeTruthy();
    expect(button("Copied")).toBeTruthy();
  });

  it("discards to a single quiet line", async () => {
    await mount();
    await click("Discard");
    const row = container.querySelector("[data-outgoing-state=discarded]")!;
    expect(row.textContent).toContain("Draft discarded");
    expect(button("Copy")).toBeUndefined();
  });

  it("is read-only in a historical transcript", async () => {
    await mount(call(), true);
    expect(container.querySelector("[data-outgoing-preview]")).toBeTruthy();
    expect(button("Copy")).toBeUndefined();
    expect(button("Edit")).toBeUndefined();
  });

  it("previews Slack connector drafts as formatted Markdown, tables and code included", async () => {
    resetMockOutgoing([draft({
      format: "slackMarkdown",
      body: "Done.\n\n| service | status |\n|---|---|\n| api | ok |\n\n```bash\nbun run deploy\n```",
      rendered: "Done.\n\n| service | status |\n|---|---|\n| api | ok |\n\n```bash\nbun run deploy\n```",
      lint: [],
    })]);
    await mount();
    const preview = container.querySelector("[data-outgoing-preview]")!;
    expect(preview.querySelector("table")?.textContent).toContain("api");
    expect(preview.textContent).toContain("bun run deploy");
    expect(preview.textContent).not.toContain("|---|");
  });

  it("shows attached images and removes one as an edit", async () => {
    resetMockOutgoing([draft({ attachments: [
      { path: "/tmp/a.png", name: "a.png", mime: "image/png", bytes: 68 },
      { path: "/tmp/b.png", name: "b.png", mime: "image/png", bytes: 68 },
    ] })]);
    await mount();
    await act(async () => { await Promise.resolve(); });
    const images = container.querySelectorAll("[data-outgoing-attachment] img");
    expect(images).toHaveLength(2);
    expect(images[0].getAttribute("src")).toMatch(/^data:image\/png;base64,/);
    await act(async () => { (container.querySelector('[aria-label="Remove image a.png"]') as HTMLButtonElement).click(); });
    await act(async () => { await Promise.resolve(); });
    expect([...container.querySelectorAll("[data-outgoing-attachment]")].map(node => node.getAttribute("data-outgoing-attachment"))).toEqual(["b.png"]);
  });

  it("copies an attached image to the clipboard as a PNG", async () => {
    const written: unknown[] = [];
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: { writeText: () => Promise.resolve(), write: (items: unknown[]) => { written.push(...items); return Promise.resolve(); } },
    });
    vi.stubGlobal("ClipboardItem", class { constructor(public items: Record<string, Blob>) {} });
    vi.stubGlobal("fetch", async () => ({ blob: async () => new Blob(["png"], { type: "image/png" }) }));
    resetMockOutgoing([draft({ attachments: [{ path: "/tmp/a.png", name: "a.png", mime: "image/png", bytes: 68 }] })]);
    await mount();
    await act(async () => { await Promise.resolve(); });
    await act(async () => { (container.querySelector('[aria-label="Copy image a.png"]') as HTMLButtonElement).click(); });
    await act(async () => { await Promise.resolve(); await Promise.resolve(); });
    expect(written).toHaveLength(1);
    expect(Object.keys((written[0] as { items: Record<string, Blob> }).items)).toEqual(["image/png"]);
    vi.unstubAllGlobals();
  });

  it("refuses a save when the draft changed after Edit was pressed, even if the card refreshed", async () => {
    await mount();
    await click("Edit");
    const textarea = container.querySelector("textarea")!;
    await act(async () => {
      const setter = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")!.set!;
      setter.call(textarea, "my local text");
      textarea.dispatchEvent(new Event("input", { bubbles: true }));
    });
    // Another view edits the draft; the change event refreshes this card to revision 2.
    const { mockUpdateOutgoing } = await import("../outgoingMock");
    await act(async () => { await mockUpdateOutgoing(MOCK_OUTGOING_ID, 1, "edited elsewhere"); });
    await act(async () => { await Promise.resolve(); });
    await click("Save");
    expect(container.querySelector("[role=alert]")?.textContent).toContain("changed since you opened it");
    expect((await import("../outgoingMock")).mockListOutgoing("session-1").items[0].body).toBe("edited elsewhere");
  });

  it("finds a forked chat's copy of a draft by the id its transcript still names", async () => {
    resetMockOutgoing([draft({ id: "fork-copy-1", sourceId: MOCK_OUTGOING_ID, body: "the fork's copy", rendered: "the fork's copy" })]);
    await mount();
    expect(container.querySelector("[data-outgoing-preview]")?.textContent).toBe("the fork's copy");
    expect(container.querySelector("[data-outgoing-state=missing]")).toBeNull();
  });

  it("previews fenced html, diagram and mermaid blocks as code, the way the destination shows them", async () => {
    const body = "```html\n<button>Ship it</button>\n```\n\n```mermaid\ngraph TD; a-->b\n```";
    resetMockOutgoing([draft({ format: "github", body, rendered: body, lint: [] })]);
    await mount();
    const preview = container.querySelector("[data-outgoing-preview]")!;
    expect(preview.querySelector("iframe")).toBeNull();
    expect(preview.querySelector("button")?.textContent).not.toBe("Ship it");
    expect(preview.textContent).toContain("<button>Ship it</button>");
    expect(preview.textContent).toContain("graph TD; a-->b");
  });
});


describe("Outgoing send approval", () => {
  it("opens the exact effect, cancels without sending, and confirms once to a permalink", async () => {
    await mount(); await click("Send");
    expect(container.querySelector('[role="dialog"]')?.textContent).toContain("Deploy is done, all green");
    expect(container.querySelector('[role="dialog"]')?.textContent).toContain("Slack · #dev");
    await click("Cancel");
    expect(container.querySelector('[role="dialog"]')).toBeNull();
    expect(container.querySelector('[data-outgoing-state="draft"]')).toBeTruthy();
    expect(container.querySelector('[role="alert"]')).toBeNull();
    await click("Send");
    const confirm = container.querySelector('[role="dialog"] button:last-child') as HTMLButtonElement;
    await act(async () => { confirm.click(); confirm.click(); await new Promise(resolve => setTimeout(resolve, 130)); });
    expect(container.querySelector('[data-outgoing-state="sent"]')).toBeTruthy();
    expect(container.querySelector('a')?.textContent).toBe("View sent message");
    expect(button("Edit")).toBeUndefined(); expect(button("Send")).toBeUndefined();
  });
  it("keeps the approved revision when the underlying draft changes before confirmation", async () => {
    await mount(); await click("Send");
    // No outgoing event: model an older UI still showing its approval.
    resetMockOutgoing([draft({ revision: 2, body: "changed", rendered: "changed" })]);
    const confirm = container.querySelector('[role="dialog"] button:last-child') as HTMLButtonElement;
    await act(async () => { confirm.click(); });
    expect(container.querySelector('[role="alert"]')?.textContent).toContain("changed since");
    expect(container.querySelector('[data-outgoing-state="sent"]')).toBeNull();
  });
  it("shows failed sends and requires another review on Retry", async () => {
    resetMockOutgoing([draft({ state: "failed", lastError: "Slack unavailable" })]);
    await mount(); expect(container.querySelector('[role="alert"]')?.textContent).toBe("Slack unavailable");
    await click("Retry"); expect(container.querySelector('[role="dialog"]')).toBeTruthy();
  });
  it("clears the old failure alert when the draft is copied", async () => {
    resetMockOutgoing([draft({ state: "failed", lastError: "Slack unavailable" })]);
    await mount();
    await click("Copy");
    expect(container.querySelector('[data-outgoing-state="copied"]')).toBeTruthy();
    expect(container.querySelector('[role="alert"]')).toBeNull();
    expect(button("Send")).toBeTruthy();
  });
  it("explains unavailable image uploads and preserves Copy image", async () => {
    resetMockOutgoing([draft({ attachments: [{ path: "/tmp/a.png", name: "a.png", mime: "image/png", bytes: 68 }] })]);
    vi.spyOn(bridgeApi, "sendOutgoing").mockResolvedValue({ kind: "refused", detail: "This Slack connection cannot upload images. Use Copy image." });
    await mount(); await click("Send");
    expect(container.querySelector('[role="alert"]')?.textContent).toContain("cannot upload images");
    expect(container.querySelector('[aria-label="Copy image a.png"]')).toBeTruthy();
  });
  it("clipboard and archived cards cannot send", async () => {
    resetMockOutgoing([draft({ destination: { kind: "clipboard", format: "plain" } })]);
    await mount(); expect(button("Send")).toBeUndefined(); expect(button("Copy")).toBeTruthy();
    await mount(call(), true); expect(container.querySelectorAll("button")).toHaveLength(0);
  });
});

it("keeps Copy image available when text was sent but image sharing failed", async () => {
  resetMockOutgoing([draft({ state: "sent", permalink: "https://acme.slack.com/archives/C123/p1794571200000001", lastError: "Text sent, but an image could not be shared. Use Copy image.", attachments: [{ path: "/tmp/a.png", name: "a.png", mime: "image/png", bytes: 68 }] })]);
  await mount();
  expect(container.querySelector('[aria-label="Copy image a.png"]')).toBeTruthy();
  expect(container.querySelector('[aria-label="Remove image a.png"]')).toBeNull();
  expect(button("Send")).toBeUndefined();
});

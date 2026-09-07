// @vitest-environment jsdom
// Remember-this rules (testing/feat-memory-ui.md): assistant prose only, the
// callback receives the full untruncated text, and no affordance exists while
// streaming or when the surface has no handler.
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { AgentEvent, Session } from "../types";
import { AgentConversation } from "./AgentConversation";
import { asWireKind } from "../transcript/wire";

const session: Session = { id: "s", workspaceId: "w", harness: "codex", label: "Chat", status: "idle", startedAt: "now", endedAt: null, contextPercent: null, usagePercent: null, metricSource: "reported", model: "gpt-5.6-luna", restorationMode: "fresh", continuationFidelity: "native", kind: "chat" } as Session;
const event = (id: number, kind: string, overrides: Partial<AgentEvent> = {}): AgentEvent => ({ id, sessionId: "s", sequence: id, protocolVersion: 1, kind: asWireKind(kind), itemId: null, role: null, status: null, title: null, text: null, data: {}, providerMeta: {}, createdAt: "now", ...overrides });

let container: HTMLDivElement;
let root: Root;

function mount(events: AgentEvent[], onRemember?: (text: string) => void) {
  act(() => {
    root.render(<AgentConversation session={session} onResolve={() => undefined} events={events} onRemember={onRemember} />);
  });
}

const rememberButtons = () => [...document.querySelectorAll<HTMLButtonElement>('[aria-label="Remember this"]')];

beforeEach(() => {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

describe("Remember this", () => {
  it("appears on completed assistant prose only, and hands over the full text", () => {
    const long = "Detailed decision. ".repeat(40).trim();
    const onRemember = vi.fn();
    mount([
      event(1, "message.completed", { itemId: "u", role: "user", text: "Question", status: "completed" }),
      event(2, "message.completed", { itemId: "m", role: "assistant", text: long, status: "completed" }),
    ], onRemember);
    expect(rememberButtons()).toHaveLength(1);
    act(() => { rememberButtons()[0].dispatchEvent(new MouseEvent("click", { bubbles: true })); });
    expect(onRemember).toHaveBeenCalledWith(long);
  });

  it("offers nothing while streaming, and nothing without a handler", () => {
    const streaming = [event(1, "message.delta", { itemId: "m", role: "assistant", text: "Half a thought", status: "streaming" })];
    mount(streaming, vi.fn());
    expect(rememberButtons()).toHaveLength(0);
    mount([event(1, "message.completed", { itemId: "m", role: "assistant", text: "Done", status: "completed" })]);
    expect(rememberButtons()).toHaveLength(0);
  });

  it("worker and tool rows carry no remember affordance", () => {
    mount([
      event(1, "worker.result", { itemId: "w", role: "assistant", title: "Implementation", status: "completed", data: { status: "completed", summary: "Did the thing" } }),
      event(2, "tool.completed", { itemId: "t", title: "Read file", status: "completed", data: {} }),
    ], vi.fn());
    expect(rememberButtons()).toHaveLength(0);
  });
});

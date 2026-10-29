// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import { AgentConversation } from "./AgentConversation";
import { INITIAL_TRANSCRIPT_ROWS, TRANSCRIPT_HYDRATE_CHUNK } from "./transcriptWindow";
import { asWireKind } from "../transcript/wire";
import type { AgentEvent, Session, SessionEntry } from "../types";
import type { SessionEntryWindowSummary } from "../protocol/generated/protocol";

const session: Session = { id: "s", workspaceId: "w", harness: "codex", label: "Chat", status: "idle", startedAt: "now", endedAt: null, contextPercent: null, usagePercent: null, metricSource: "reported", model: "gpt-5.6-luna", restorationMode: "fresh", continuationFidelity: "native", kind: "direct" };

const entry: SessionEntry = { id: "e1", sessionId: "s", parentEntryId: null, sequence: 1, semanticSchemaVersion: 2, kind: "assistant.message", payload: { text: "history" }, providerEventId: null, contextVisibility: "eligible", tokenEstimate: null, createdAt: "now" };

const window_ = (returned: number, total: number): SessionEntryWindowSummary => ({ returned, total, trimmedPayloads: 0 });

async function render(node: React.ReactElement) {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  const container = document.createElement("div");
  const root = createRoot(container);
  await act(async () => root.render(node));
  return { container, root, cleanup: () => act(async () => root.unmount()) };
}

const history = (count: number): SessionEntry[] => Array.from({ length: count }, (_, index) => ({
  id: `h${index}`, sessionId: "s", parentEntryId: index === 0 ? null : `h${index - 1}`, sequence: index + 1,
  semanticSchemaVersion: 2, kind: index % 2 === 0 ? "user.message" : "assistant.message",
  payload: { text: `Message ${index}` }, providerEventId: null, contextVisibility: "eligible",
  tokenEstimate: null, createdAt: "now",
}));

const rowCount = (container: HTMLElement) => container.querySelectorAll("[data-transcript-row]").length;

/// The idle fallback is a frame-spaced timeout, so a real 40ms wait lets one
/// hydration slice commit.
const settle = async () => { await act(async () => { await new Promise(resolve => setTimeout(resolve, 40)); }); };

/// A manual idle queue. The opening window is asserted on the first commit, so
/// the slice must not fire from a timer racing the assertion: capture the
/// callbacks and run exactly one per `slice()`.
const pendingIdle = new Map<number, () => void>();
let nextIdleId = 1;

function manualIdle() {
  vi.stubGlobal("requestIdleCallback", (callback: IdleRequestCallback) => {
    const id = nextIdleId++;
    pendingIdle.set(id, () => callback({ didTimeout: false, timeRemaining: () => 50 }));
    return id;
  });
  vi.stubGlobal("cancelIdleCallback", (id: number) => { pendingIdle.delete(id); });
}

async function slice() {
  const next = pendingIdle.entries().next();
  if (next.done) throw new Error("no idle slice was scheduled");
  const [id, task] = next.value;
  pendingIdle.delete(id);
  await act(async () => { task(); });
}

afterEach(() => {
  vi.unstubAllGlobals();
  pendingIdle.clear();
});

describe("AgentConversation history loading", () => {
  it("shows a skeleton while an empty transcript's history is in flight", async () => {
    const { container, cleanup } = await render(
      <AgentConversation session={session} onResolve={() => undefined} />,
    );
    try {
      expect(container.querySelector("[aria-label=\"Loading conversation\"]")).not.toBeNull();
    } finally {
      await cleanup();
    }
  });

  it("drops the skeleton once history arrives", async () => {
    const { container, cleanup } = await render(
      <AgentConversation session={session} onResolve={() => undefined}
        forestEntries={[entry]} activeLeafId="e1" entryWindow={window_(1, 1)} />,
    );
    try {
      // A poll that refreshes an already-rendered transcript must not flicker
      // it back to a placeholder.
      expect(container.querySelector("[aria-label=\"Loading conversation\"]")).toBeNull();
      expect(container.textContent).toContain("history");
    } finally {
      await cleanup();
    }
  });

  it("says how much of a long chat is rendered rather than passing a tail off as the whole thing", async () => {
    const { container, cleanup } = await render(
      <AgentConversation session={session} onResolve={() => undefined}
        forestEntries={[entry]} activeLeafId="e1" entryWindow={window_(1500, 7344)} />,
    );
    try {
      expect(container.textContent).toContain("most recent 1,500 of 7,344 events");
      expect(container.textContent).toContain("5,844 earlier events are");
    } finally {
      await cleanup();
    }
  });

  it("stays silent when the whole conversation fits in the window", async () => {
    const { container, cleanup } = await render(
      <AgentConversation session={session} onResolve={() => undefined}
        forestEntries={[entry]} activeLeafId="e1" entryWindow={window_(1, 1)} />,
    );
    try {
      expect(container.textContent).not.toContain("most recent");
    } finally {
      await cleanup();
    }
  });

  it("mounts only the newest rows on the first commit", async () => {
    manualIdle();
    const entries = history(300);
    const { container, cleanup } = await render(
      <AgentConversation session={session} onResolve={() => undefined}
        forestEntries={entries} activeLeafId="h299" />,
    );
    try {
      expect(rowCount(container)).toBe(INITIAL_TRANSCRIPT_ROWS);
      // The newest row is painted; the oldest is not mounted yet.
      expect(container.querySelector('[data-entry-id="h299"]')).not.toBeNull();
      expect(container.querySelector('[data-entry-id="h0"]')).toBeNull();
    } finally {
      await cleanup();
    }
  });

  it("hydrates older rows one idle slice at a time", async () => {
    manualIdle();
    const entries = history(300);
    const { container, cleanup } = await render(
      <AgentConversation session={session} onResolve={() => undefined}
        forestEntries={entries} activeLeafId="h299" />,
    );
    try {
      expect(rowCount(container)).toBe(INITIAL_TRANSCRIPT_ROWS);
      await slice();
      expect(rowCount(container)).toBe(INITIAL_TRANSCRIPT_ROWS + TRANSCRIPT_HYDRATE_CHUNK);
      await slice();
      expect(rowCount(container)).toBe(INITIAL_TRANSCRIPT_ROWS + 2 * TRANSCRIPT_HYDRATE_CHUNK);
      expect(container.querySelector('[data-entry-id="h219"]')).not.toBeNull();
    } finally {
      await cleanup();
    }
  });

  it("mounts every row at once for a highlighted entry", async () => {
    const entries = history(300);
    const { container, cleanup } = await render(
      <AgentConversation session={session} onResolve={() => undefined}
        forestEntries={entries} activeLeafId="h299" highlightEntryId="h0" />,
    );
    try {
      expect(rowCount(container)).toBe(300);
      expect(container.querySelector('#forest-entry-h0')).not.toBeNull();
    } finally {
      await cleanup();
    }
  });

  it("keeps an already-hydrated row mounted when a live row arrives", async () => {
    const entries = history(300);
    const message: AgentEvent = {
      id: 900, sessionId: "s", sequence: 901, protocolVersion: 1, kind: asWireKind("message.completed"),
      itemId: "live-1", role: "assistant", status: "completed", title: null, text: "Streamed reply",
      data: {}, providerMeta: {}, createdAt: "now",
    };
    const props = { session, onResolve: () => undefined, forestEntries: entries, activeLeafId: "h299" };
    const { container, root, cleanup } = await render(<AgentConversation {...props} events={[]} />);
    try {
      await settle();
      const hydrated = container.querySelector('[data-entry-id="h250"]');
      expect(hydrated).not.toBeNull();
      await act(async () => root.render(<AgentConversation {...props} events={[message]} />));
      expect(container.textContent).toContain("Streamed reply");
      // The row the slice mounted is the same node: a tail arrival must not
      // remount history that is already on screen.
      expect(container.querySelector('[data-entry-id="h250"]')).toBe(hydrated);
    } finally {
      await cleanup();
    }
  });

  it("keeps the history mounted after a search highlight clears", async () => {
    const entries = history(300);
    const props = { session, onResolve: () => undefined, forestEntries: entries, activeLeafId: "h299" };
    const { container, root, cleanup } = await render(<AgentConversation {...props} highlightEntryId="h0" />);
    try {
      expect(rowCount(container)).toBe(300);
      await act(async () => root.render(<AgentConversation {...props} highlightEntryId={null} />));
      // Clearing the highlight must not collapse the window and yank the row
      // the reader jumped to.
      expect(rowCount(container)).toBe(300);
    } finally {
      await cleanup();
    }
  });
});

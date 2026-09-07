// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterAll, afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { AgentConversation } from "./AgentConversation";
import type { Session, SessionEntry } from "../types";

// jsdom has no layout: `scrollHeight` and `clientHeight` are 0 for everything,
// and `Element.prototype.scrollTo` does not exist. Both are stubbed on the
// prototype so the transcript container measures like a real viewport with a
// long transcript behind it, and every programmatic scroll is recorded with the
// behavior it asked for.
const VIEWPORT = 600;
let contentHeight = VIEWPORT;
let scrolls: { top: number; behavior?: string }[] = [];
const bottom = () => contentHeight - VIEWPORT;

const originals = {
  scrollHeight: Object.getOwnPropertyDescriptor(HTMLElement.prototype, "scrollHeight"),
  clientHeight: Object.getOwnPropertyDescriptor(HTMLElement.prototype, "clientHeight"),
  scrollTo: Object.getOwnPropertyDescriptor(HTMLElement.prototype, "scrollTo"),
};
Object.defineProperty(HTMLElement.prototype, "scrollHeight", { configurable: true, get: () => contentHeight });
Object.defineProperty(HTMLElement.prototype, "clientHeight", { configurable: true, get: () => VIEWPORT });
Object.defineProperty(HTMLElement.prototype, "scrollTo", {
  configurable: true,
  writable: true,
  value(this: HTMLElement, options?: ScrollToOptions | number, y?: number) {
    const top = typeof options === "object" && options !== null ? options.top ?? 0 : y ?? 0;
    scrolls.push({ top, behavior: typeof options === "object" && options !== null ? options.behavior : undefined });
    this.scrollTop = top;
  },
});
afterAll(() => {
  for (const [name, descriptor] of Object.entries(originals)) {
    if (descriptor) Object.defineProperty(HTMLElement.prototype, name, descriptor);
    else delete (HTMLElement.prototype as unknown as Record<string, unknown>)[name];
  }
});

const chat = (id: string): Session => ({ id, workspaceId: "w", harness: "codex", label: "Orchestrator", status: "idle", startedAt: "now", endedAt: null, contextPercent: null, usagePercent: null, metricSource: "reported", model: "gpt-5.6-luna", restorationMode: "fresh", continuationFidelity: "native", kind: "orchestrator" });

/// A durable transcript: the history a chat opened cold receives in one commit
/// when its forest snapshot lands.
const history = (sessionId: string, count: number): SessionEntry[] => Array.from({ length: count }, (_, index) => ({
  id: `${sessionId}-e${index}`,
  sessionId,
  parentEntryId: index === 0 ? null : `${sessionId}-e${index - 1}`,
  sequence: index + 1,
  semanticSchemaVersion: 2,
  kind: index % 2 === 0 ? "user.message" : "assistant.message",
  payload: { text: `Message ${index}` },
  providerEventId: null,
  contextVisibility: "eligible",
  tokenEstimate: null,
  createdAt: "now",
}));

describe("AgentConversation scroll placement", () => {
  let container: HTMLDivElement;
  let root: ReturnType<typeof createRoot>;

  beforeEach(() => {
    (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
    contentHeight = VIEWPORT;
    scrolls = [];
    container = document.createElement("div");
    document.body.append(container);
    root = createRoot(container);
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
  });

  /// `rows` is what the transcript would measure once React commits it, so the
  /// height is in place before the placement reads it, exactly as it is in a
  /// browser where the commit lays out before anything is painted.
  const show = async (props: Record<string, unknown>, rows: number) => {
    contentHeight = rows === 0 ? VIEWPORT : VIEWPORT + rows * 120;
    await act(async () => root.render(<AgentConversation onResolve={() => undefined} events={[]} {...props} />));
    return container.firstElementChild as HTMLDivElement;
  };
  const scrollTo = async (el: HTMLDivElement, top: number) => {
    el.scrollTop = top;
    await act(async () => { el.dispatchEvent(new Event("scroll")); });
  };

  // The bug: history arrives from the forest snapshot after the first commit,
  // so the transcript used to mount at `scrollTop = 0` and stay there.
  it("lands on the latest message when history arrives after mount", async () => {
    const session = chat("late");
    const el = await show({ session, continuationFidelity: "projected_at_boundary" }, 0);
    expect(el.scrollTop).toBe(0);

    const entries = history("late", 40);
    await show({ session, forestEntries: entries, activeLeafId: entries[entries.length - 1].id }, 40);
    expect(el.scrollTop).toBe(bottom());
  });

  it("lands on the latest message when the whole transcript mounts at once", async () => {
    const entries = history("cold", 40);
    const el = await show({ session: chat("cold"), forestEntries: entries, activeLeafId: entries[entries.length - 1].id }, 40);
    expect(el.scrollTop).toBe(bottom());
  });

  // A landing is a placement, not a journey: `scroll-smooth` on the container
  // would otherwise animate it down from the top, showing the first message and
  // every message between it and the last.
  it("lands instantly, never gliding down from the top", async () => {
    const entries = history("instant", 40);
    await show({ session: chat("instant"), forestEntries: entries, activeLeafId: entries[entries.length - 1].id }, 40);
    expect(scrolls.length).toBeGreaterThan(0);
    expect(scrolls.every(scroll => scroll.behavior === "instant")).toBe(true);
    expect(scrolls.every(scroll => scroll.top === bottom())).toBe(true);
  });

  // The scroll event a programmatic scroll produces arrives a frame late, by
  // which time streaming has already made the transcript taller. Read as a
  // reader's scroll it looks like they left the bottom, and live follow used to
  // disarm itself on exactly that.
  it("does not treat its own scroll as the reader leaving the bottom", async () => {
    const session = chat("follow");
    const entries = history("follow", 20);
    const el = await show({ session, forestEntries: entries, activeLeafId: entries[entries.length - 1].id }, 20);
    const landed = el.scrollTop;
    expect(landed).toBe(bottom());

    contentHeight = VIEWPORT + 60 * 120;
    await act(async () => { el.dispatchEvent(new Event("scroll")); });
    expect(el.scrollTop).toBe(landed);

    const longer = history("follow", 60);
    await show({ session, forestEntries: longer, activeLeafId: longer[longer.length - 1].id }, 60);
    expect(el.scrollTop).toBe(bottom());
  });

  it("treats a real scroll away from the bottom as intent and stops following", async () => {
    const session = chat("intent");
    const entries = history("intent", 20);
    const el = await show({ session, forestEntries: entries, activeLeafId: entries[entries.length - 1].id }, 20);

    await scrollTo(el, 0);
    const longer = history("intent", 60);
    await show({ session, forestEntries: longer, activeLeafId: longer[longer.length - 1].id }, 60);
    expect(el.scrollTop).toBe(0);
  });

  it("restores the last-read position when a chat is reopened", async () => {
    const first = chat("first");
    const firstEntries = history("first", 40);
    const el = await show({ session: first, forestEntries: firstEntries, activeLeafId: firstEntries[firstEntries.length - 1].id }, 40);
    await scrollTo(el, 900);

    const second = chat("second");
    const secondEntries = history("second", 30);
    await show({ session: second, forestEntries: secondEntries, activeLeafId: secondEntries[secondEntries.length - 1].id }, 30);
    expect(el.scrollTop).toBe(bottom());

    await show({ session: first, forestEntries: firstEntries, activeLeafId: firstEntries[firstEntries.length - 1].id }, 40);
    expect(el.scrollTop).toBe(900);
  });

  // A remembered mid-position with nothing left to scroll through (the window
  // grew, or the entries were compacted) used to return from `land` without
  // landing, so `landed.current` stayed false and every later signature change
  // just retried the same no-op restore instead of arming live follow.
  it("arms live follow when a reopened chat has nothing to scroll through", async () => {
    const session = chat("shrunk");
    const entries = history("shrunk", 40);
    const el = await show({ session, forestEntries: entries, activeLeafId: entries[entries.length - 1].id }, 40);
    await scrollTo(el, 900);

    const other = chat("elsewhere2");
    const otherEntries = history("elsewhere2", 10);
    await show({ session: other, forestEntries: otherEntries, activeLeafId: otherEntries[otherEntries.length - 1].id }, 10);

    await show({ session, forestEntries: entries, activeLeafId: entries[entries.length - 1].id }, 0);
    expect(el.scrollTop).toBe(0);

    const grown = history("shrunk", 60);
    await show({ session, forestEntries: grown, activeLeafId: grown[grown.length - 1].id }, 60);
    expect(el.scrollTop).toBe(bottom());
  });

  // Selecting a chat swaps `session` one commit before its forest snapshot
  // follows. Placing against that in-between render measures the chat you just
  // left, and counts the new one as already opened.
  it("waits for the chat's own history before placing", async () => {
    const leaving = chat("leaving");
    const leavingEntries = history("leaving", 40);
    const el = await show({ session: leaving, forestEntries: leavingEntries, activeLeafId: leavingEntries[leavingEntries.length - 1].id }, 40);
    await scrollTo(el, 700);

    // The lagging commit: the arriving chat's id, the departing chat's rows.
    const arriving = chat("arriving");
    await show({ session: arriving, forestEntries: leavingEntries, activeLeafId: leavingEntries[leavingEntries.length - 1].id }, 40);
    expect(el.scrollTop).toBe(700);

    const arrivingEntries = history("arriving", 60);
    await show({ session: arriving, forestEntries: arrivingEntries, activeLeafId: arrivingEntries[arrivingEntries.length - 1].id }, 60);
    expect(el.scrollTop).toBe(bottom());
  });

  // Read to the end, and the end is where you belong on the way back: whatever
  // arrived since is what you have not read.
  it("reopens a chat that was left at the bottom at its new bottom", async () => {
    const session = chat("bottomed");
    const entries = history("bottomed", 20);
    const el = await show({ session, forestEntries: entries, activeLeafId: entries[entries.length - 1].id }, 20);
    await scrollTo(el, bottom());

    const other = chat("elsewhere");
    const otherEntries = history("elsewhere", 10);
    await show({ session: other, forestEntries: otherEntries, activeLeafId: otherEntries[otherEntries.length - 1].id }, 10);

    const grown = history("bottomed", 50);
    await show({ session, forestEntries: grown, activeLeafId: grown[grown.length - 1].id }, 50);
    expect(el.scrollTop).toBe(bottom());
  });

  // The Ask-aside chip's always-mounted wrapper is a child of this container.
  // ScrollFollow's growth observer watches `firstElementChild`, and an empty
  // chip wrapper in that slot would never resize — late-growing rows (an image
  // decoding, a code block highlighting) would stop re-pinning the reader at
  // the bottom. The transcript content must stay the observed child.
  it("keeps observing the transcript content, not the Ask-aside chip wrapper", async () => {
    const observed: Element[] = [];
    let lastCallback: ResizeObserverCallback | undefined;
    class RecordingResizeObserver {
      constructor(callback: ResizeObserverCallback) { lastCallback = callback; }
      observe(el: Element) { observed.push(el); }
      unobserve() {}
      disconnect() {}
    }
    vi.stubGlobal("ResizeObserver", RecordingResizeObserver);
    try {
      const session = chat("chip-observer");
      const entries = history("chip-observer", 20);
      const el = await show({
        session,
        forestEntries: entries,
        activeLeafId: entries[entries.length - 1].id,
        onAskAside: () => {},
      }, 20);
      expect(el.querySelector("[data-ask-aside-chip], .contents")).not.toBeNull();
      const content = el.querySelector(".max-w-3xl")!;
      expect(content).not.toBeNull();
      expect(observed).toContain(content);
      expect(observed).not.toContain(el.querySelector("span.contents"));

      // Late growth inside the settle window re-pins at the new bottom, so
      // the landing holds exactly as it does without the chip.
      contentHeight = VIEWPORT + 40 * 120;
      await act(async () => {
        lastCallback?.([], undefined as unknown as ResizeObserver);
      });
      expect(el.scrollTop).toBe(bottom());
    } finally {
      vi.unstubAllGlobals();
    }
  });
});

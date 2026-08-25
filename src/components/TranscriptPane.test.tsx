// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { AgentEvent, SessionEntry } from "../types";
import type { SessionHead } from "../protocol/generated/protocol";
import { TRANSCRIPT_PAGE_SIZE, TranscriptPane, type TranscriptLoader } from "./TranscriptPane";

// Contract: testing/feat-dock-transcript.md §1–§3.

const event = (id: number, sequence: number, kind: string, overrides: Partial<AgentEvent> = {}): AgentEvent => ({
  id, sessionId: "s", sequence, protocolVersion: 1, kind, itemId: null, role: null, status: null,
  title: null, text: null, data: {}, providerMeta: {}, createdAt: "2026-08-25T10:00:00Z", ...overrides,
});

const entry = (id: string, kind: string, parentEntryId: string | null): SessionEntry => ({
  id, sessionId: "s", parentEntryId, sequence: 1, semanticSchemaVersion: 2, kind, payload: {},
  providerEventId: null, contextVisibility: "eligible", tokenEstimate: null, createdAt: "2026-08-25T10:00:00Z",
});

const head: SessionHead = { sessionId: "s", activeEntryId: "e3", nativeProviderSessionId: null, restorationMode: "hot", resumeEligibility: "native", latestCheckpointEntryId: null, updatedAt: "now" };

let container: HTMLDivElement;
let root: Root;
let clipboard: ReturnType<typeof vi.fn>;

async function mount(node: React.ReactElement) {
  await act(async () => root.render(node));
  await act(async () => {
    await new Promise(resolve => setTimeout(resolve, 0));
  });
}

const click = async (element: Element) => {
  await act(async () => {
    element.dispatchEvent(new MouseEvent("click", { bubbles: true }));
  });
};

beforeEach(() => {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  clipboard = vi.fn().mockResolvedValue(undefined);
  Object.defineProperty(navigator, "clipboard", { configurable: true, value: { writeText: clipboard } });
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});

afterEach(async () => {
  await act(async () => root.unmount());
  container.remove();
  vi.restoreAllMocks();
});

describe("TranscriptPane stream", () => {
  it("loads the first page through the tail loader and renders its rows", async () => {
    const loader = vi.fn<TranscriptLoader>().mockResolvedValue([
      event(1, 1, "turn.started"),
      event(2, 2, "tool.started", { title: "Edit src/App.tsx" }),
    ]);
    await mount(<TranscriptPane sessionId="s" events={[]} loadOlder={loader} />);
    expect(loader).toHaveBeenCalledTimes(1);
    expect(loader).toHaveBeenCalledWith({ tail: true });
    expect(container.textContent).toContain("turn.started");
    expect(container.textContent).toContain("tool.started");
    expect(container.textContent).toContain("2 of 2 events");
  });

  it("merges live events by id in sequence order", async () => {
    const loader = vi.fn<TranscriptLoader>().mockResolvedValue([event(1, 1, "turn.started")]);
    await mount(<TranscriptPane sessionId="s" events={[event(1, 1, "turn.started"), event(3, 3, "message.completed")]} loadOlder={loader} />);
    const kinds = [...container.querySelectorAll("button[aria-expanded]")].map(row => row.textContent);
    expect(kinds.filter(text => text?.includes("turn.started"))).toHaveLength(1);
    expect(container.textContent).toContain("message.completed");
  });

  it("pages backward until sequence one is loaded", async () => {
    const first = Array.from({ length: TRANSCRIPT_PAGE_SIZE }, (_, index) => event(500 + index, 500 + index, "message.delta"));
    const loader = vi.fn<TranscriptLoader>().mockResolvedValue(first);
    await mount(<TranscriptPane sessionId="s" events={[]} loadOlder={loader} />);
    const earlier = [...container.querySelectorAll("button")].find(button => button.textContent?.includes("Load earlier"))!;
    expect(earlier).toBeTruthy();

    loader.mockResolvedValue([event(1, 1, "turn.started")]);
    await click(earlier);
    expect(loader).toHaveBeenLastCalledWith({ beforeSequence: 500, tail: false });
    expect([...container.querySelectorAll("button")].some(button => button.textContent?.includes("Load earlier"))).toBe(false);
  });

  it("filters by kind or text and restores on clear", async () => {
    const loader = vi.fn<TranscriptLoader>().mockResolvedValue([
      event(1, 1, "turn.started"),
      event(2, 2, "approval.requested", { text: "bun install" }),
    ]);
    await mount(<TranscriptPane sessionId="s" events={[]} loadOlder={loader} />);
    const input = container.querySelector<HTMLInputElement>('input[aria-label="Filter events"]')!;
    const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!;
    await act(async () => {
      setter.call(input, "approval");
      input.dispatchEvent(new Event("input", { bubbles: true }));
    });
    expect(container.textContent).not.toContain("turn.started");
    expect(container.textContent).toContain("approval.requested");
    expect(container.textContent).toContain("1 of 2 events");
    await act(async () => {
      setter.call(input, "");
      input.dispatchEvent(new Event("input", { bubbles: true }));
    });
    expect(container.textContent).toContain("turn.started");
  });

  it("copies the filtered stream as a JSON array", async () => {
    const loader = vi.fn<TranscriptLoader>().mockResolvedValue([event(1, 1, "turn.started"), event(2, 2, "tool.started")]);
    await mount(<TranscriptPane sessionId="s" events={[]} loadOlder={loader} />);
    await click(container.querySelector('button[aria-label="Copy the filtered stream as JSON"]')!);
    const written = clipboard.mock.calls[0][0] as string;
    const parsed = JSON.parse(written) as AgentEvent[];
    expect(parsed).toHaveLength(2);
    expect(parsed[0].kind).toBe("turn.started");
  });
});

describe("TranscriptPane inspector", () => {
  it("opens one raw payload at a time and copies it", async () => {
    const loader = vi.fn<TranscriptLoader>().mockResolvedValue([
      event(1, 1, "tool.started", { data: { command: "bun test" } }),
      event(2, 2, "turn.completed"),
    ]);
    await mount(<TranscriptPane sessionId="s" events={[]} loadOlder={loader} />);
    const rows = [...container.querySelectorAll<HTMLButtonElement>("button[aria-expanded]")];
    await click(rows[0]);
    expect(container.querySelector("pre")!.textContent).toContain('"command": "bun test"');

    await click(rows[1]);
    expect(container.querySelectorAll("pre")).toHaveLength(1);
    expect(container.querySelector("pre")!.textContent).toContain("turn.completed");

    await click(container.querySelector('button[aria-label="Copy event 2 as JSON"]')!);
    expect(JSON.parse(clipboard.mock.calls[0][0] as string).kind).toBe("turn.completed");
  });
});

describe("TranscriptPane entries", () => {
  const entries = [
    entry("e1", "branch.summary", null),
    entry("e2", "message.user", "e1"),
    entry("e3", "message.assistant", "e2"),
    entry("e4", "message.assistant", "e2"),
  ];

  async function mountEntries(onRevealEntry?: (id: string) => void) {
    await mount(<TranscriptPane sessionId="s" events={[]} entries={entries} head={head} leaves={[entries[2], entries[3]]} onRevealEntry={onRevealEntry} />);
    const tab = [...container.querySelectorAll("button")].find(button => button.textContent === "entries")!;
    await click(tab);
  }

  it("swaps views through the segmented control", async () => {
    await mountEntries();
    expect(container.textContent).toContain("branch.summary");
    expect(container.textContent).toContain("4 entries");
  });

  it("shows forest identity and marks only the active chain", async () => {
    await mountEntries();
    expect(container.textContent).toContain("eligible");
    const rowFor = (id: string) => [...container.querySelectorAll("div")].find(row =>
      row.className.includes("items-baseline") && row.querySelector("span")?.nextElementSibling?.textContent === id);
    const marked = (id: string) => [...(rowFor(id)?.querySelectorAll("span") ?? [])].some(span => span.textContent === "active");
    expect(rowFor("e3")).toBeTruthy();
    expect(marked("e3")).toBe(true);
    expect(marked("e2")).toBe(true);
    expect(marked("e4")).toBe(false);
  });

  it("states the head and leaf count", async () => {
    await mountEntries();
    expect(container.textContent).toContain("head e3");
    expect(container.textContent).toContain("2 leaves");
  });

  it("reveals an entry through the callback", async () => {
    const onRevealEntry = vi.fn();
    await mountEntries(onRevealEntry);
    await click(container.querySelector('button[aria-label="Reveal entry e2 in the conversation"]')!);
    expect(onRevealEntry).toHaveBeenCalledWith("e2");
  });
});

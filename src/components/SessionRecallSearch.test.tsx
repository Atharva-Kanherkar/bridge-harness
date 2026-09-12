// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { RECALL_PAGE_SIZE, SessionRecallSearch } from "./SessionRecallSearch";
import type { SearchSessionEntriesResult, SessionRecallHit } from "../types";

let container: HTMLDivElement;
let root: Root;

type Search = (sessionId: string, query: string, limit?: number | null, offset?: number | null) => Promise<SearchSessionEntriesResult>;

function mount(search: Search, onJump = vi.fn()) {
  act(() => {
    root.render(
      <SessionRecallSearch
        sessionId="chat-a"
        search={search}
        onClose={() => undefined}
        onJump={onJump}
      />,
    );
  });
}

beforeEach(() => {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
  vi.useFakeTimers();
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.useRealTimers();
});

function typeQuery(value: string) {
  const input = container.querySelector<HTMLInputElement>("input")!;
  const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!;
  act(() => {
    setter.call(input, value);
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
}

describe("SessionRecallSearch", () => {
  it("searches only the session it was opened on", async () => {
    const search = vi.fn(async (sessionId: string) => ({
      sessionId,
      query: "cookie",
      hits: [{ entryId: "e1", kind: "user.message", sequence: 1, snippet: "cookie in a", createdAt: "now" }],
      offset: 0,
      hasMore: false,
    }));
    mount(search);
    typeQuery("cookie");
    await act(async () => {
      await vi.advanceTimersByTimeAsync(200);
    });
    expect(search).toHaveBeenCalledWith("chat-a", "cookie", RECALL_PAGE_SIZE, 0);
    expect(container.textContent).toContain("cookie in a");
    expect(container.textContent).toContain("This search stays in this chat");
  });

  it("jumps to a hit in this chat", async () => {
    const onJump = vi.fn();
    mount(async () => ({
      sessionId: "chat-a",
      query: "cookie",
      hits: [{ entryId: "e1", kind: "user.message", sequence: 1, snippet: "cookie in a", createdAt: "now" }],
      offset: 0,
      hasMore: false,
    }), onJump);
    typeQuery("cookie");
    await act(async () => {
      await vi.advanceTimersByTimeAsync(200);
    });
    const hit = [...container.querySelectorAll("button")].find(button => button.textContent?.includes("cookie in a"));
    expect(hit).toBeTruthy();
    act(() => {
      hit!.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });
    expect(onJump).toHaveBeenCalledWith("e1");
  });

  // Contract: testing/feat-session-observability.md §D1–D4 (issue #446).

  const page = (offset: number, count: number): SessionRecallHit[] =>
    Array.from({ length: count }, (_, index) => ({
      entryId: `e${offset + index}`,
      kind: "user.message",
      sequence: offset + index,
      snippet: `hit ${offset + index}`,
      createdAt: "now",
    }));

  it("offers another page only when the server says one exists", async () => {
    const search = vi.fn(async (sessionId: string, query: string, _limit?: number | null, offset?: number | null) => ({
      sessionId, query, offset: offset ?? 0, hits: page(offset ?? 0, RECALL_PAGE_SIZE), hasMore: (offset ?? 0) === 0,
    }));
    mount(search);
    typeQuery("hit");
    await act(async () => { await vi.advanceTimersByTimeAsync(200); });

    const showMore = () => [...container.querySelectorAll("button")].find(button => button.textContent === "Show more");
    expect(showMore()).toBeTruthy();

    await act(async () => {
      showMore()!.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });
    // Appended, not replaced: paging is reading further, not searching again.
    expect(search).toHaveBeenLastCalledWith("chat-a", "hit", RECALL_PAGE_SIZE, RECALL_PAGE_SIZE);
    expect(container.querySelectorAll("li")).toHaveLength(RECALL_PAGE_SIZE * 2);
    expect(container.textContent).toContain("hit 0");
    expect(container.textContent).toContain(`hit ${RECALL_PAGE_SIZE}`);
    expect(showMore()).toBeUndefined();
  });

  it("does not offer another page for a short result", async () => {
    mount(async (sessionId, query) => ({ sessionId, query, hits: page(0, 3), offset: 0, hasMore: false }));
    typeQuery("hit");
    await act(async () => { await vi.advanceTimersByTimeAsync(200); });
    expect([...container.querySelectorAll("button")].some(button => button.textContent === "Show more")).toBe(false);
  });

  it("treats a daemon that predates paging as having no further page", async () => {
    // An app talking to an older bridged gets no flag. Offering Show more
    // there would lead to nothing; the honest reading is that this is all.
    mount(async (sessionId, query) => ({ sessionId, query, hits: page(0, RECALL_PAGE_SIZE) } as SearchSessionEntriesResult));
    typeQuery("hit");
    await act(async () => { await vi.advanceTimersByTimeAsync(200); });
    expect([...container.querySelectorAll("button")].some(button => button.textContent === "Show more")).toBe(false);
  });

  it("says plainly when nothing matched", async () => {
    mount(async (sessionId, query) => ({ sessionId, query, hits: [], offset: 0, hasMore: false }));
    typeQuery("nothing");
    await act(async () => { await vi.advanceTimersByTimeAsync(200); });
    expect(container.textContent).toContain("No matches in this chat.");
  });
});

// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { SessionRecallSearch } from "./SessionRecallSearch";
import type { SearchSessionEntriesResult } from "../types";

let container: HTMLDivElement;
let root: Root;

function mount(search: (sessionId: string, query: string) => Promise<SearchSessionEntriesResult>, onJump = vi.fn()) {
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
    }));
    mount(search);
    typeQuery("cookie");
    await act(async () => {
      await vi.advanceTimersByTimeAsync(200);
    });
    expect(search).toHaveBeenCalledWith("chat-a", "cookie");
    expect(container.textContent).toContain("cookie in a");
    expect(container.textContent).toContain("This search stays in this chat");
  });

  it("jumps to a hit in this chat", async () => {
    const onJump = vi.fn();
    mount(async () => ({
      sessionId: "chat-a",
      query: "cookie",
      hits: [{ entryId: "e1", kind: "user.message", sequence: 1, snippet: "cookie in a", createdAt: "now" }],
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
});

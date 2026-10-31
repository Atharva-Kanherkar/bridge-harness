// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { splitNotesContext } from "../../notesBrief";
import type { NotePage } from "../../types";
import { NotesCopilot } from "./NotesCopilot";

let host: HTMLDivElement;
let root: Root;
beforeEach(() => {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  host = document.createElement("div");
  document.body.append(host);
  root = createRoot(host);
});
afterEach(() => { act(() => root.unmount()); host.remove(); });

const page = (seq: number, markdown = "# Launch\nship it\n"): NotePage => ({
  note: { id: "n1", spaceId: "personal", kind: "note", state: "open", tags: [], sessionIds: [], title: "Launch", createdAt: "", updatedAt: "", createdBy: "person", lastActor: "person", preview: markdown, seq },
  markdown, seq, structured: null, tasks: [],
});
const click = (label: string) => act(() => {
  [...host.querySelectorAll("button")].find(node => node.textContent?.trim() === label)!.dispatchEvent(new MouseEvent("click", { bubbles: true }));
});

it("offers only page-free asks with no page open, and sends them bare", () => {
  const onAsk = vi.fn();
  act(() => root.render(<NotesCopilot page={null} onAsk={onAsk} />));
  expect(host.textContent).toContain("Open a page and it reads that page too.");
  expect(host.textContent).not.toContain("Summarize this page");
  click("Find a note");
  expect(splitNotesContext(onAsk.mock.calls[0][0]).context).toBeNull();
});

const send = async (label: string) => {
  await act(async () => {
    [...host.querySelectorAll("button")].find(node => node.textContent?.trim() === label)!.dispatchEvent(new MouseEvent("click", { bubbles: true }));
  });
  await act(async () => { await Promise.resolve(); });
};
const contextOf = (message: string) => splitNotesContext(message).context;

it("folds the open page onto a message only when it changed", async () => {
  const onAsk = vi.fn(async (_message: string) => true);
  act(() => root.render(<NotesCopilot page={page(3)} projectName="Bridge" onAsk={onAsk} />));
  await send("Summarize this page");
  const first = splitNotesContext(onAsk.mock.calls[0][0]);
  expect(first.text).toBe("Summarize the open page in five bullets. Don't change it.");
  expect(first.context).toContain("Open page: Launch (id n1, seq 3)");
  expect(first.context).toContain("Project: Bridge");
  await send("What's missing?");
  expect(contextOf(onAsk.mock.calls[1][0])).toBeNull();
  act(() => root.render(<NotesCopilot page={page(4, "# Launch\nship it friday\n")} projectName="Bridge" onAsk={onAsk} />));
  await send("What's missing?");
  expect(contextOf(onAsk.mock.calls[2][0])).toContain("ship it friday");
});

it("carries the page again after a send that did not go through", async () => {
  // The first ask is refused (the chat was still being created elsewhere).
  const onAsk = vi.fn<(message: string) => Promise<boolean>>().mockResolvedValueOnce(false).mockResolvedValue(true);
  act(() => root.render(<NotesCopilot page={page(3)} onAsk={onAsk} />));
  await send("Summarize this page");
  await send("What's missing?");
  expect(contextOf(onAsk.mock.calls[1][0])).toContain("seq 3");
  await send("What's missing?");
  expect(contextOf(onAsk.mock.calls[2][0])).toBeNull();
});

it("holds the docked chat to the same rule once the chat exists", async () => {
  const sent: string[] = [];
  let refuse = true;
  const deliver = async (message: string) => {
    sent.push(message);
    if (refuse) { refuse = false; throw new Error("busy"); }
  };
  act(() => root.render(<NotesCopilot page={page(5)} onAsk={vi.fn(async () => true)} renderChat={chatSend => <button type="button" onClick={() => void chatSend("hello", deliver).catch(() => undefined)}>send</button>} />));
  expect(host.querySelector("aside[aria-label='Notes chat']")).toBeNull();
  await send("send");
  await send("send");
  await send("send");
  expect(contextOf(sent[0])).toContain("seq 5");
  expect(contextOf(sent[1])).toContain("seq 5");
  expect(sent[2]).toBe("hello");
});

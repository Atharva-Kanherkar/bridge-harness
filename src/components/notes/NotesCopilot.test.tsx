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

it("folds the open page onto a message only when it changed", () => {
  const onAsk = vi.fn();
  act(() => root.render(<NotesCopilot page={page(3)} projectName="Bridge" onAsk={onAsk} />));
  click("Summarize this page");
  const first = splitNotesContext(onAsk.mock.calls[0][0]);
  expect(first.text).toBe("Summarize the open page in five bullets. Don't change it.");
  expect(first.context).toContain("Open page: Launch (id n1, seq 3)");
  expect(first.context).toContain("Project: Bridge");
  click("What's missing?");
  expect(splitNotesContext(onAsk.mock.calls[1][0]).context).toBeNull();
  act(() => root.render(<NotesCopilot page={page(4, "# Launch\nship it friday\n")} projectName="Bridge" onAsk={onAsk} />));
  click("What's missing?");
  expect(splitNotesContext(onAsk.mock.calls[2][0]).context).toContain("ship it friday");
});

it("hands the docked chat the same brief once the chat exists", () => {
  const sent: string[] = [];
  act(() => root.render(<NotesCopilot page={page(5)} onAsk={vi.fn()} renderChat={brief => <button type="button" onClick={() => sent.push(brief("hello"))}>send</button>} />));
  expect(host.querySelector("aside[aria-label='Notes chat']")).toBeNull();
  click("send");
  click("send");
  expect(splitNotesContext(sent[0]).context).toContain("seq 5");
  expect(sent[1]).toBe("hello");
});

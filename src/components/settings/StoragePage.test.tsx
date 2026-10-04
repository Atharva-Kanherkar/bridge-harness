// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { bridgeApi } from "../../api";
import { StoragePage } from "./StoragePage";
import { StorageCopilot, type StorageCopilotHost } from "./StorageCopilot";
import { Markdown } from "../Markdown";

let host: HTMLDivElement;
let root: Root;
beforeEach(() => {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  host = document.createElement("div");
  document.body.append(host);
  root = createRoot(host);
});
afterEach(() => {
  act(() => root.unmount());
  host.remove();
  vi.restoreAllMocks();
});

// The host app's half: the rail before a storage chat exists, sending through
// `brief` exactly as App does.
function copilotFor(ask: (prompt: string) => void): StorageCopilotHost {
  return { ask, render: (brief, selectedCount) => <StorageCopilot selectedCount={selectedCount} onAsk={question => ask(brief(question))} /> };
}

async function render(ask?: (prompt: string) => void) {
  await act(async () => { root.render(<StoragePage copilot={ask && copilotFor(ask)} />); });
  await act(async () => { await Promise.resolve(); });
}

const button = (label: string) =>
  [...host.querySelectorAll("button")].find(node => node.textContent?.trim() === label || node.getAttribute("aria-label") === label);
const row = (text: string) =>
  [...host.querySelectorAll("section[aria-label='Everything on this Mac'] li")].find(node => node.textContent?.includes(text));

it("shows free space against the whole disk and the home folder's biggest children", async () => {
  await render();
  const disk = host.querySelector("section[aria-label='Disk']")?.textContent ?? "";
  expect(disk).toContain("19.4 GB");
  expect(disk).toContain("free of 494 GB");
  expect(disk).toContain("Nearly full");
  expect(disk).toContain("Library");
  expect(disk).toContain("Apps, system, and other");
});

it("lists cleanup suggestions largest first with whether they rebuild themselves", async () => {
  await render();
  const suggestions = [...host.querySelectorAll("section[aria-label='Cleanup suggestions'] li")].map(node => node.textContent ?? "");
  expect(suggestions[0]).toContain("Docker disk image");
  expect(suggestions.find(text => text.includes("npm cache"))).toContain("Rebuilds itself");
});

it("drills into a folder and protects the standard ones", async () => {
  await render();
  expect((row("Documents")?.querySelector("[role=checkbox]") as HTMLButtonElement | null)?.disabled).toBe(true);
  expect(row("Documents")?.textContent).toContain("Protected");
  await act(async () => { button("Downloads")!.click(); });
  await act(async () => { await Promise.resolve(); });
  expect(row("Xcode_16.4.xip")?.textContent).toContain("11.9 GB");
});

it("moves selected items to the Trash only after confirmation", async () => {
  const remove = vi.spyOn(bridgeApi, "deletePaths");
  await render();
  await act(async () => { button("Downloads")!.click(); });
  await act(async () => { await Promise.resolve(); });
  await act(async () => { (row("Xcode_16.4.xip")!.querySelector("[role=checkbox]") as HTMLButtonElement).click(); });
  await act(async () => { (row("Docker.dmg")!.querySelector("[role=checkbox]") as HTMLButtonElement).click(); });
  expect(host.querySelector("[aria-label='Selection']")?.textContent).toContain("2 selected · 14.0 GB");
  const bar = host.querySelector("[aria-label='Selection']")!;
  await act(async () => { [...bar.querySelectorAll("button")].find(node => node.textContent === "Move to Trash")!.click(); });
  expect(remove).not.toHaveBeenCalled();
  expect(host.textContent).toContain("Move 2 items to the Trash?");
  const confirm = host.querySelector("[aria-label='Confirm deletion']")!;
  await act(async () => { [...confirm.querySelectorAll("button")].find(node => node.textContent === "Move to Trash")!.click(); });
  expect(remove).toHaveBeenCalledWith(["/Users/demo/Downloads/Xcode_16.4.xip", "/Users/demo/Downloads/Docker.dmg"], false);
  expect(host.textContent).toContain("Moved 2 items, 14.0 GB to the Trash");
  expect(row("Xcode_16.4.xip")).toBeUndefined();
});

it("sends the question as typed with what the page measured folded on as a snapshot", async () => {
  const ask = vi.fn();
  await render(ask);
  await act(async () => { button("What can I safely delete?")!.click(); });
  expect(ask).toHaveBeenCalledTimes(1);
  const prompt = ask.mock.calls[0][0] as string;
  expect(prompt.startsWith("What can I safely delete?")).toBe(true);
  expect(prompt).toContain("```storage-snapshot\n");
  expect(prompt).toContain("19.4 GB free of 494 GB");
  expect(prompt).toContain("Docker disk image: 31.0 GB");
  // The rules live in the agent's system prompt, never in the person's message.
  expect(prompt).not.toContain("wait for me to say yes");
});

it("re-sends the snapshot only when the page measured something new", async () => {
  const ask = vi.fn();
  await render(ask);
  await act(async () => { button("What can I safely delete?")!.click(); });
  await act(async () => { button("Why is System Data so large?")!.click(); });
  expect(ask.mock.calls[1][0]).not.toContain("storage-snapshot");
  await act(async () => { button("Downloads")!.click(); });
  await act(async () => { await Promise.resolve(); });
  // An earlier case trashed the installers from the shared mock disk.
  await act(async () => { (row("screen-recording.mov")!.querySelector("[role=checkbox]") as HTMLButtonElement).click(); });
  await act(async () => { button("Developer caches")!.click(); });
  const third = ask.mock.calls[2][0] as string;
  expect(third).toContain("Viewing ~/Downloads");
  expect(third).toContain("Selected on the page:\n- ~/Downloads/screen-recording.mov (1.6 GB)");
});

it("asks about one suggestion from its card", async () => {
  const ask = vi.fn();
  await render(ask);
  await act(async () => { button("Ask about Docker disk image")!.click(); });
  expect(ask.mock.calls[0][0]).toMatch(/^What is Docker disk image \(~\/Library\/Containers\/com\.docker\.docker\/Data\/vms, 31\.0 GB\)\?/);
});

it("lets an agent's plan card move ticked items to the Trash and tells the agent next time", async () => {
  const remove = vi.spyOn(bridgeApi, "deletePaths");
  const ask = vi.fn();
  const plan = JSON.stringify({ title: "Clear downloads", items: [
    { path: "~/Downloads/screen-recording.mov", sizeBytes: 1_600_000_000, why: "Old recording", safety: "safe" },
    { path: "~/Downloads/invoice.pdf", sizeBytes: 220_000, why: "A document you may want", safety: "review" },
  ] });
  await act(async () => {
    root.render(<StoragePage copilot={{ ask, render: brief => <div><Markdown text={"Here is a plan.\n\n```storage-plan\n" + plan + "\n```"} /><button type="button" onClick={() => ask(brief("thanks"))}>reply</button></div> }} />);
  });
  await act(async () => { await Promise.resolve(); });
  const card = host.querySelector("section[aria-label='Plan: Clear downloads']")!;
  expect(card.textContent).toContain("Rebuilds");
  expect(card.textContent).toContain("Review");
  // Only the safe item starts ticked.
  const go = [...card.querySelectorAll("button")].find(node => node.textContent?.startsWith("Move 1 to Trash"))!;
  expect(go.textContent).toContain("1.6 GB");
  await act(async () => { go.click(); });
  await act(async () => { await Promise.resolve(); });
  expect(remove).toHaveBeenCalledWith(["/Users/demo/Downloads/screen-recording.mov"], false);
  expect(card.textContent).toContain("In Trash");
  expect(card.textContent).toContain("Moved 1 item");
  await act(async () => { button("reply")!.click(); });
  expect(ask.mock.calls.at(-1)![0]).toContain("Moved to the Trash from your plans (Trash not yet emptied):\n- ~/Downloads/screen-recording.mov");
});

it("approves a plan's command by replying to the agent", async () => {
  const ask = vi.fn();
  const plan = JSON.stringify({ title: "Tools", commands: [{ run: "brew cleanup --prune=all", why: "Old downloads", frees: 1_000_000_000 }] });
  await act(async () => {
    root.render(<StoragePage copilot={{ ask, render: () => <Markdown text={"```storage-plan\n" + plan + "\n```"} /> }} />);
  });
  await act(async () => { button("Run it")!.click(); });
  expect(ask.mock.calls[0][0]).toMatch(/^Approved: run `brew cleanup --prune=all`/);
  expect(host.textContent).toContain("Approved");
});

it("docks the storage chat in place of the intro once it exists", async () => {
  await act(async () => {
    root.render(<StoragePage copilot={{ ask: () => {}, render: () => <StorageCopilot selectedCount={0} onAsk={() => {}} chat={<section aria-label="Storage with Claude" />} /> }} />);
  });
  expect(host.querySelector("section[aria-label='Storage with Claude']")).not.toBeNull();
  expect(host.querySelector("aside[aria-label='Ask Bridge']")).toBeNull();
});

it("has no copilot when nothing can open a chat", async () => {
  await render();
  expect(host.querySelector("aside[aria-label='Ask Bridge']")).toBeNull();
});

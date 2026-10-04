// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { bridgeApi } from "../../api";
import { StoragePage } from "./StoragePage";
import { StorageCopilot, type StorageCopilotHost } from "./StorageCopilot";

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
  expect(suggestions.find(text => text.includes("npm cache"))).toContain("Rebuilt automatically");
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

it("hands a question to a Bridge chat with what the page measured", async () => {
  const ask = vi.fn();
  await render(ask);
  await act(async () => { button("What can I safely delete?")!.click(); });
  expect(ask).toHaveBeenCalledTimes(1);
  const prompt = ask.mock.calls[0][0] as string;
  expect(prompt).toContain("19.4 GB free of 494 GB");
  expect(prompt).toContain("Docker disk image: 31.0 GB");
  expect(prompt).toContain("wait for me to say yes");
  expect(prompt).toContain("My question: What can I safely delete?");
});

it("briefs only the first question of a visit", async () => {
  const ask = vi.fn();
  await render(ask);
  await act(async () => { button("What can I safely delete?")!.click(); });
  await act(async () => { button("Why is System Data so large?")!.click(); });
  expect(ask).toHaveBeenCalledTimes(2);
  expect(ask.mock.calls[0][0]).toContain("My question: What can I safely delete?");
  expect(ask.mock.calls[1][0]).toBe("Why is System Data so large?");
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

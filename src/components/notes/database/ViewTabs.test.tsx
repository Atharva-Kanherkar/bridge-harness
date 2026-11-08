// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { bridgeApi } from "../../../api";
import { mockCreateDatabase, mockReadPage, resetMockNotes } from "../../../notesMock";
import { DatabaseView } from "./DatabaseView";

let host: HTMLDivElement;
let root: Root;
beforeEach(() => {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  Object.defineProperty(HTMLElement.prototype, "clientHeight", { configurable: true, get() { return 600; } });
  resetMockNotes();
  host = document.createElement("div");
  document.body.append(host);
  root = createRoot(host);
});
afterEach(() => {
  act(() => root.unmount());
  host.remove();
  document.body.innerHTML = "";
  delete (HTMLElement.prototype as { clientHeight?: number }).clientHeight;
  vi.restoreAllMocks();
});

const settle = () => act(async () => { for (let i = 0; i < 8; i += 1) await new Promise(resolve => setTimeout(resolve, 5)); });
const button = (name: string) => [...document.querySelectorAll<HTMLButtonElement>("button")].find(candidate => candidate.textContent?.trim() === name || candidate.getAttribute("aria-label") === name);
const click = async (name: string) => { await act(async () => { button(name)!.click(); }); await settle(); };
const tabs = () => [...document.querySelectorAll("[role=tab]")].map(tab => tab.textContent);
const stored = (id: string) => [...mockReadPage(id).database!.views].sort((a, b) => a.order - b.order);
function type(label: string, text: string) {
  const input = document.querySelector(`input[aria-label="${label}"]`) as HTMLInputElement;
  act(() => { Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input, text); input.dispatchEvent(new Event("input", { bubbles: true })); });
}
const submit = async (label: string) => { await act(async () => { (document.querySelector(`input[aria-label="${label}"]`) as HTMLInputElement).form!.requestSubmit(); }); await settle(); };

async function mount() {
  const made = mockCreateDatabase({ title: "Books", properties: [{ name: "Status", type: "status", options: [{ name: "To read", group: "todo" }] }, { name: "Due", type: "date" }] });
  await act(async () => { root.render(<DatabaseView databaseId={made.note.id} workspaces={[]} onOpenPage={vi.fn()} onError={vi.fn()} />); });
  await settle();
  return made;
}

describe("ViewTabs", () => {
  it("new view of each type saves its defaults", async () => {
    const made = await mount();
    const [status, due] = made.database!.properties;
    for (const [type, name] of [["Board", ""], ["Calendar", "Due dates"], ["List", "Plain"]]) {
      await click("New view");
      await act(async () => { [...document.querySelectorAll<HTMLButtonElement>("[role=radio]")].find(radio => radio.textContent === type)!.click(); });
      if (name) nameNewView(name);
      await submit("New view name");
    }
    expect(tabs()).toEqual(["Table", "Board", "Due dates", "Plain"]);
    const views = stored(made.note.id);
    expect(views.map(view => [view.name, view.type, view.order])).toEqual([["Table", "table", 0], ["Board", "board", 1], ["Due dates", "calendar", 2], ["Plain", "list", 3]]);
    expect(views[1].groupBy).toEqual({ property: status.id });
    expect(views[2].dateProperty).toBe(due.id);
    // The new view opens.
    expect(document.querySelector("[role=tab][aria-selected=true]")!.textContent).toBe("Plain");
  });

  it("rename, duplicate, change type and delete save, and the last view cannot be deleted", async () => {
    const made = await mount();
    await click("Table view menu");
    await click("Rename");
    type("View name", "Everything");
    await submit("View name");
    expect(stored(made.note.id).map(view => view.name)).toEqual(["Everything"]);
    await click("Everything view menu");
    expect(button("Delete (the last view stays)")!.disabled).toBe(true);
    await click("Duplicate");
    expect(tabs()).toEqual(["Everything", "Everything copy"]);
    expect(stored(made.note.id).map(view => [view.name, view.order])).toEqual([["Everything", 0], ["Everything copy", 1]]);
    await click("Everything copy view menu");
    await click("Change type");
    await act(async () => { [...document.querySelectorAll<HTMLButtonElement>("[role=menuitemradio]")].find(item => item.textContent === "Board")!.click(); });
    await settle();
    expect(stored(made.note.id)[1]).toMatchObject({ type: "board", groupBy: { property: made.database!.properties[0].id } });
    await click("Everything view menu");
    await click("Delete");
    expect(stored(made.note.id).map(view => view.name)).toEqual(["Everything copy"]);
    expect(tabs()).toEqual(["Everything copy"]);
  });

  it("moving a tab saves the order", async () => {
    const made = await mount();
    for (const name of ["Second", "Third"]) {
      await click("New view");
      nameNewView(name);
      await submit("New view name");
    }
    await click("Third view menu");
    await click("Move left");
    expect(stored(made.note.id).map(view => [view.name, view.order])).toEqual([["Table", 0], ["Third", 1], ["Second", 2]]);
    expect(tabs()).toEqual(["Table", "Third", "Second"]);
    // A drag of one tab onto another puts it there.
    const tab = (name: string) => [...document.querySelectorAll("[role=tab]")].find(candidate => candidate.textContent === name)!.parentElement!;
    act(() => { tab("Second").dispatchEvent(new Event("dragstart", { bubbles: true })); });
    await act(async () => { tab("Table").dispatchEvent(new Event("drop", { bubbles: true, cancelable: true })); });
    await settle();
    expect(stored(made.note.id).map(view => view.name)).toEqual(["Second", "Table", "Third"]);
  });

  it("a slow reorder never writes an older name back", async () => {
    const made = await mount();
    await click("New view");
    nameNewView("Second");
    await submit("New view name");
    const real = bridgeApi.saveDatabaseView.bind(bridgeApi);
    let release!: () => void;
    const gate = new Promise<void>(resolve => { release = resolve; });
    let first = true;
    vi.spyOn(bridgeApi, "saveDatabaseView").mockImplementation(async params => { if (first) { first = false; await gate; } return real(params); });
    await click("Table view menu");
    await click("Move right");
    await click("Table view menu");
    await click("Rename");
    type("View name", "Renamed");
    await submit("View name");
    await act(async () => { release(); });
    await settle();
    expect(stored(made.note.id).map(view => view.name)).toEqual(["Second", "Renamed"]);
  });
});

function nameNewView(text: string) { type("New view name", text); }

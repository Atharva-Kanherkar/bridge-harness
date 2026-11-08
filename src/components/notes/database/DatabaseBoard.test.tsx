// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { bridgeApi } from "../../../api";
import { mockCreateDatabase, mockCreatePage, mockQueryDatabase, mockReadPage, mockSaveView, mockUpdateNote, resetMockNotes } from "../../../notesMock";
import { DatabaseView } from "./DatabaseView";
import { groupConditions, movedValue } from "./DatabaseBoard";

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

function reading() {
  const made = mockCreateDatabase({ title: "Books", properties: [
    { name: "Status", type: "status", options: [{ name: "To read", group: "todo" }, { name: "Reading", group: "inProgress" }, { name: "Read", group: "done" }] },
    { name: "Labels", type: "multiSelect", options: [{ name: "Short" }, { name: "Gift" }] },
  ] });
  const [status, labels] = made.database!.properties;
  const option = (name: string) => [...status.options!, ...labels.options!].find(item => item.name === name)!.id;
  const row = (title: string, values: Record<string, unknown>) => { const page = mockCreatePage({ title, parentId: made.note.id }); mockUpdateNote({ id: page.id, values }); return page.id; };
  return { made, status, labels, option, row };
}

async function mountBoard(databaseId: string, groupBy: string, onError = vi.fn()) {
  const view = mockSaveView({ databaseId, view: { name: "Board", type: "board", groupBy: { property: groupBy } } });
  await act(async () => { root.render(<DatabaseView databaseId={databaseId} viewId={view.id} workspaces={[]} onOpenPage={vi.fn()} onError={onError} />); });
  await settle();
  return onError;
}

const columnLabels = () => [...document.querySelectorAll("[aria-label=Board] > section")].map(section => section.getAttribute("aria-label"));
const cardsIn = (label: string) => [...document.querySelectorAll(`[aria-label=Board] > section[aria-label^='${label},'] li`)].map(card => card.querySelector("button span")?.textContent);
const card = (title: string) => [...document.querySelectorAll<HTMLElement>("[aria-label=Board] li > div")].find(element => element.querySelector("button span")?.textContent === title)!;
const column = (label: string) => document.querySelector(`[aria-label=Board] > section[aria-label^='${label},']`)!;
const fire = (target: Element, type: string) => act(() => { target.dispatchEvent(new Event(type, { bubbles: true, cancelable: true })); });

describe("DatabaseBoard", () => {
  it("shows one column per group with its count", async () => {
    const { made, status, option, row } = reading();
    row("DDIA", { [status.id]: option("Reading") });
    row("SICP", { [status.id]: option("To read") });
    row("Piranesi", { [status.id]: option("Reading") });
    row("Loose", {});
    await mountBoard(made.note.id, status.id);
    expect(columnLabels()).toEqual(["To read, 1", "Reading, 2", "Read, 0", "No value, 1"]);
    expect(cardsIn("Reading").sort()).toEqual(["DDIA", "Piranesi"]);
  });

  it("dragging a card writes the grouped value and rolls back on error", async () => {
    const { made, status, option, row } = reading();
    const ddia = row("DDIA", { [status.id]: option("To read") });
    const onError = await mountBoard(made.note.id, status.id);
    fire(card("DDIA"), "dragstart");
    fire(column("Read"), "dragover");
    fire(column("Read"), "drop");
    expect(cardsIn("Read")).toEqual(["DDIA"]);
    await settle();
    expect(mockReadPage(ddia).values).toEqual({ [status.id]: option("Read") });
    expect(columnLabels()).toEqual(["To read, 0", "Reading, 0", "Read, 1", "No value, 0"]);
    // A refused write puts the card back.
    vi.spyOn(bridgeApi, "updateNote").mockRejectedValueOnce(new Error("refused"));
    fire(card("DDIA"), "dragstart");
    fire(column("Reading"), "drop");
    await settle();
    expect(cardsIn("Read")).toEqual(["DDIA"]);
    expect(cardsIn("Reading")).toEqual([]);
    expect(onError).toHaveBeenCalledWith("refused");
  });

  it("Move to… moves a card from the keyboard", async () => {
    const { made, status, option, row } = reading();
    const ddia = row("DDIA", { [status.id]: option("To read") });
    await mountBoard(made.note.id, status.id);
    act(() => (document.querySelector("button[aria-label='Move DDIA to…']") as HTMLButtonElement).click());
    expect([...document.querySelectorAll("[role=menu][aria-label='Move to'] [role=menuitem]")].map(item => item.textContent)).toEqual(["Reading", "Read", "No value"]);
    await act(async () => { [...document.querySelectorAll<HTMLButtonElement>("[role=menuitem]")].find(item => item.textContent === "No value")!.click(); });
    await settle();
    expect(mockReadPage(ddia).values).toEqual({});
  });

  it("a multi-select move swaps the option", async () => {
    const { made, labels, option, row } = reading();
    const ddia = row("DDIA", { [labels.id]: [option("Short"), option("Gift")] });
    await mountBoard(made.note.id, labels.id);
    expect(columnLabels()).toEqual(["Short, 1", "Gift, 1", "No value, 0"]);
    const short = [...column("Short").querySelectorAll<HTMLElement>("li > div")][0];
    fire(short, "dragstart");
    fire(column("No value"), "drop");
    await settle();
    expect(mockReadPage(ddia).values).toEqual({ [labels.id]: [option("Gift")] });
    expect(movedValue({ id: "l", name: "L", type: "multiSelect" }, ["a", "b"], "a", "c")).toEqual(["b", "c"]);
    expect(movedValue({ id: "l", name: "L", type: "multiSelect" }, ["a"], "a", "")).toBeNull();
    expect(movedValue({ id: "c", name: "C", type: "checkbox" }, true, "true", "false")).toBe(false);
  });

  it("Load more pages one column", async () => {
    const { made, status, option, row } = reading();
    for (let index = 0; index < 120; index += 1) row(`Book ${String(index).padStart(3, "0")}`, { [status.id]: option("To read") });
    await mountBoard(made.note.id, status.id);
    expect(cardsIn("To read")).toHaveLength(50);
    const query = vi.spyOn(bridgeApi, "queryDatabase");
    await act(async () => { [...column("To read").querySelectorAll<HTMLButtonElement>("button")].find(button => button.textContent === "Load more (70)")!.click(); });
    await settle();
    expect(cardsIn("To read")).toHaveLength(100);
    expect(query.mock.calls[0][0].filter).toEqual({ and: groupConditions(status, option("To read")) });
    await act(async () => { [...column("To read").querySelectorAll<HTMLButtonElement>("button")].find(button => button.textContent === "Load more (20)")!.click(); });
    await settle();
    expect(cardsIn("To read")).toHaveLength(120);
    expect(query.mock.calls.at(-1)![0].cursor).toBeTruthy();
    expect(new Set(cardsIn("To read")).size).toBe(120);
  });

  it("New in a column makes a row with that value", async () => {
    const { made, status, option } = reading();
    await mountBoard(made.note.id, status.id);
    await act(async () => { (document.querySelector("button[aria-label='New in Reading']") as HTMLButtonElement).click(); });
    await settle();
    expect(columnLabels()).toEqual(["To read, 0", "Reading, 1", "Read, 0", "No value, 0"]);
    expect(cardsIn("Reading")).toEqual(["Untitled"]);
    const [fresh] = mockQueryDatabase({ databaseId: made.note.id }).rows;
    expect(fresh.values).toEqual({ [status.id]: option("Reading") });
  });
});

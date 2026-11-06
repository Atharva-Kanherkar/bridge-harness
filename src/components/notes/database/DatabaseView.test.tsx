// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { bridgeApi } from "../../../api";
import { mockCreateDatabase, mockCreatePage, mockReadPage, mockSaveView, mockUpdateNote, mockUpdateSchema, resetMockNotes } from "../../../notesMock";
import type { DatabaseQueryResult } from "../../../types";
import { DatabaseView, FIRST_PAGE, LATER_PAGE, type DatabaseViewProps } from "./DatabaseView";

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
  vi.useRealTimers();
});

const settle = () => act(async () => { for (let i = 0; i < 6; i += 1) await new Promise(resolve => setTimeout(resolve, 5)); });

function books() {
  const page = mockCreateDatabase({ title: "Books", properties: [{ name: "Rating", type: "number" }] });
  const [rating] = page.database!.properties;
  return { id: page.note.id, rating: rating.id };
}

async function mount(overrides: Partial<DatabaseViewProps> & Pick<DatabaseViewProps, "databaseId">) {
  const props: DatabaseViewProps = { workspaces: [], onOpenPage: vi.fn(), onError: vi.fn(), ...overrides };
  await act(async () => { root.render(<DatabaseView {...props} />); });
  await settle();
  return props;
}

const titles = () => [...document.querySelectorAll("[role=row]")].slice(1).map(row => row.querySelector("[role=gridcell]")?.textContent?.replace(/Open$/, ""));
const ratingCell = (index: number) => [...document.querySelectorAll("[role=row]")][index + 1].querySelectorAll("[role=gridcell]")[1] as HTMLElement;

describe("DatabaseView", () => {
  it("loads the first 100 rows, then the rest by cursor", async () => {
    const { id } = books();
    for (let index = 0; index < 250; index += 1) mockCreatePage({ title: `Book ${String(index).padStart(3, "0")}`, parentId: id });
    const query = vi.spyOn(bridgeApi, "queryDatabase");
    await mount({ databaseId: id });
    const calls = query.mock.calls.map(([params]) => [params.limit, !!params.cursor]);
    expect(calls).toEqual([[FIRST_PAGE, false], [LATER_PAGE, true]]);
    expect(document.querySelector("[role=grid]")!.getAttribute("aria-rowcount")).toBe("251");
  });

  it("shows a committed value at once and puts it back when the write is refused", async () => {
    const { id, rating } = books();
    const row = mockCreatePage({ title: "DDIA", parentId: id });
    mockUpdateNote({ id: row.id, values: { [rating]: 3 } });
    const onError = vi.fn();
    await mount({ databaseId: id, onError });
    expect(ratingCell(0).textContent).toBe("3");

    let refuse!: (error: Error) => void;
    vi.spyOn(bridgeApi, "updateNote").mockImplementationOnce(() => new Promise((_, reject) => { refuse = reject; }));
    const grid = document.querySelector("[role=grid]") as HTMLElement;
    act(() => { grid.focus(); ratingCell(0).click(); });
    act(() => { grid.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true })); });
    const box = document.querySelector("input[aria-label=Rating]") as HTMLInputElement;
    act(() => { Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(box, "5"); box.dispatchEvent(new Event("input", { bubbles: true })); });
    act(() => { box.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true })); });
    expect(ratingCell(0).textContent).toBe("5");
    await act(async () => { refuse(new Error("Rating is a finite number")); });
    await settle();
    expect(ratingCell(0).textContent).toBe("3");
    expect(onError).toHaveBeenCalledWith("Rating is a finite number");

    // An accepted write stays through the re-query.
    // The cell is still selected, so one click opens its editor.
    act(() => { ratingCell(0).click(); });
    const again = document.querySelector("input[aria-label=Rating]") as HTMLInputElement;
    act(() => { Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(again, "4"); again.dispatchEvent(new Event("input", { bubbles: true })); });
    act(() => { again.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true })); });
    await settle();
    expect(ratingCell(0).textContent).toBe("4");
  });

  it("re-queries on page_changed, and an older answer never replaces a newer one", async () => {
    const { id } = books();
    mockCreatePage({ title: "First", parentId: id });
    await mount({ databaseId: id });
    expect(titles()).toEqual(["First"]);
    // An agent adds a row: the table re-reads.
    mockCreatePage({ title: "Second", parentId: id });
    await settle();
    expect(titles()).toHaveLength(2);
    expect(titles()).toContain("Second");

    // Two reads in flight: the first answers last and is dropped.
    const real = bridgeApi.queryDatabase.bind(bridgeApi);
    let releaseOld!: () => void;
    const stale: DatabaseQueryResult = { rows: [], total: 0 };
    const spy = vi.spyOn(bridgeApi, "queryDatabase");
    spy.mockImplementationOnce(() => new Promise(resolve => { releaseOld = () => resolve(stale); }));
    spy.mockImplementation(params => real(params));
    mockCreatePage({ title: "Third", parentId: id });
    await act(async () => { await new Promise(resolve => setTimeout(resolve, 40)); });
    mockCreatePage({ title: "Fourth", parentId: id });
    await settle();
    expect(titles()).toHaveLength(4);
    await act(async () => { releaseOld(); });
    await settle();
    expect(titles()).toHaveLength(4);
  });

  it("shows a later-update notice for a non-table view", async () => {
    const { id } = books();
    const board = mockSaveView({ databaseId: id, view: { name: "Board", type: "board" } });
    await mount({ databaseId: id, viewId: board.id });
    expect(document.querySelector("[role=status]")!.textContent).toBe("Board, calendar and list views arrive in a later update.");
    const tabs = [...document.querySelectorAll("[role=tab]")];
    expect(tabs.map(tab => [tab.textContent, tab.getAttribute("aria-selected")])).toEqual([["Table", "false"], ["Board", "true"]]);
    act(() => (tabs[0] as HTMLElement).click());
    await settle();
    expect(document.querySelector("[role=grid]")).not.toBeNull();
  });

  it("an embed ignores its host page's own saves", async () => {
    const { id } = books();
    const holder = mockCreatePage({ title: "Holder" });
    await mount({ databaseId: id, hostPageId: holder.id });
    const query = vi.spyOn(bridgeApi, "queryDatabase");
    mockUpdateNote({ id: holder.id, title: "Holder, renamed" });
    await settle();
    expect(query).not.toHaveBeenCalled();
    mockCreatePage({ title: "A row", parentId: id });
    await settle();
    expect(query).toHaveBeenCalled();
  });

  it("keeps an option someone else added while the options editor was open", async () => {
    const made = mockCreateDatabase({ title: "Books", properties: [{ name: "Genre", type: "select", options: [{ name: "Fiction" }] }] });
    const genre = made.database!.properties[0];
    const other = mockCreatePage({ title: "Other", parentId: made.note.id });
    await mount({ databaseId: made.note.id });
    act(() => (document.querySelector("[role=columnheader][aria-label='Genre'] button") as HTMLElement).click());
    act(() => [...document.querySelectorAll<HTMLButtonElement>("[role=menuitem]")].find(item => item.textContent === "Edit options")!.click());
    // Another writer adds Tech and uses it while the editor is open.
    const stored = mockUpdateSchema({ id: made.note.id, changes: [{ op: "setOptions", propertyId: genre.id, options: [{ id: genre.options![0].id, name: "Fiction" }, { name: "Tech" }] }] });
    const tech = stored.properties[0].options!.find(option => option.name === "Tech")!.id;
    mockUpdateNote({ id: other.id, values: { [genre.id]: tech } });
    await settle();
    await act(async () => { [...document.querySelectorAll<HTMLButtonElement>("button")].find(button => button.textContent === "Save options")!.click(); });
    await settle();
    expect(mockReadPage(made.note.id).database!.properties[0].options!.map(option => option.name)).toEqual(["Fiction", "Tech"]);
    expect(mockReadPage(other.id).values).toEqual({ [genre.id]: tech });
  });
});

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

const later = () => act(async () => { await new Promise(resolve => setTimeout(resolve, 100)); });
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

  it("routes each view type", async () => {
    const made = mockCreateDatabase({ title: "Books", properties: [{ name: "Status", type: "status", options: [{ name: "To read", group: "todo" }] }, { name: "Due", type: "date" }] });
    const [status, due] = made.database!.properties;
    const board = mockSaveView({ databaseId: made.note.id, view: { name: "Board", type: "board", groupBy: { property: status.id } } });
    const calendar = mockSaveView({ databaseId: made.note.id, view: { name: "Calendar", type: "calendar", dateProperty: due.id } });
    const list = mockSaveView({ databaseId: made.note.id, view: { name: "List", type: "list" } });
    mockCreatePage({ title: "DDIA", parentId: made.note.id });
    await mount({ databaseId: made.note.id });
    expect(document.querySelector("[role=grid][aria-label$=table]")).not.toBeNull();
    const tab = (name: string) => [...document.querySelectorAll<HTMLButtonElement>("[role=tab]")].find(candidate => candidate.textContent === name)!;
    act(() => tab("Board").click());
    await settle();
    expect(document.querySelector("[aria-label=Board] section[aria-label='No value, 1']")).not.toBeNull();
    act(() => tab("Calendar").click());
    await settle();
    expect(document.querySelector("[aria-label=Calendar]")).not.toBeNull();
    expect(document.querySelector("aside[aria-label='No date']")!.textContent).toContain("DDIA");
    act(() => tab("List").click());
    await settle();
    expect(document.querySelector("[aria-label=List] [role=listitem]")!.textContent).toContain("DDIA");
    expect([board.id, calendar.id, list.id].every(Boolean)).toBe(true);
  });

  it("a nested or filter, two sorts and a grouping apply at once and save for everyone", async () => {
    const made = mockCreateDatabase({ title: "Books", properties: [{ name: "Rating", type: "number" }, { name: "Genre", type: "select", options: [{ name: "Fiction" }, { name: "Tech" }] }] });
    const [rating, genre] = made.database!.properties;
    const [fiction, tech] = genre.options!;
    const book = (title: string, score: number, kind: string) => { const row = mockCreatePage({ title, parentId: made.note.id }); mockUpdateNote({ id: row.id, values: { [rating.id]: score, [genre.id]: kind } }); };
    book("A", 5, fiction.id);
    book("B", 2, tech.id);
    book("C", 4, tech.id);
    book("D", 1, fiction.id);
    await mount({ databaseId: made.note.id });
    const query = vi.spyOn(bridgeApi, "queryDatabase");
    const filter = { and: [{ property: rating.id, op: "gte", value: 2 }, { or: [{ property: genre.id, op: "is", value: tech.id }, { property: rating.id, op: "gte", value: 5 }] }] };
    const view = mockReadPage(made.note.id).database!.views[0];
    // The override, built in the toolbar's own controls.
    const click = (element: Element) => act(() => (element as HTMLElement).click());
    const button = (name: string) => [...document.querySelectorAll<HTMLButtonElement>("button")].find(candidate => candidate.textContent?.trim() === name || candidate.getAttribute("aria-label") === name)!;
    const choose = (label: string, value: string) => act(() => { const select = document.querySelector(`select[aria-label="${label}"]`) as HTMLSelectElement; select.value = value; select.dispatchEvent(new Event("change", { bubbles: true })); });
    const type = (label: string, text: string) => act(() => { const input = document.querySelector(`input[aria-label="${label}"]`) as HTMLInputElement; Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input, text); input.dispatchEvent(new Event("input", { bubbles: true })); });
    click(button("Filter"));
    click(button("Add a filter"));
    choose("Filter 1 op", "gte");
    type("Filter 1 value", "2");
    click(button("Add a filter group"));
    choose("Filter 2.1 property", genre.id);
    choose("Filter 2.1 value", tech.id);
    click(button("Add to group"));
    choose("Filter 2.2 op", "gte");
    type("Filter 2.2 value", "5");
    click(button("Filter, 3"));
    click(button("Sort"));
    click(button("Add a sort"));
    choose("Sort 1 property", genre.id);
    click(button("Add a sort"));
    choose("Sort 2 property", rating.id);
    choose("Sort 2 direction", "desc");
    click(button("Sort, 2"));
    click(button("Group"));
    choose("Group by", genre.id);
    await settle();
    const last = query.mock.calls.at(-1)![0];
    expect(last).toMatchObject({ filter, sorts: [{ property: genre.id, direction: "asc" }, { property: rating.id, direction: "desc" }], groupBy: { property: genre.id } });
    expect(last.viewId).toBeUndefined();
    expect(titles()).toEqual(["A", "C", "B"]);
    // Saved for everyone, it is the view itself.
    await act(async () => { button("Save for everyone").click(); });
    await settle();
    const saved = mockReadPage(made.note.id).database!.views.find(candidate => candidate.id === view.id)!;
    expect(saved).toMatchObject({ filter, sorts: [{ property: genre.id, direction: "asc" }, { property: rating.id, direction: "desc" }], groupBy: { property: genre.id } });
    expect(button("Save for everyone")).toBeUndefined();
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

  it("keeps every loaded row and an open editor through a background refresh", async () => {
    const { id } = books();
    for (let index = 0; index < 250; index += 1) mockCreatePage({ title: `Book ${String(index).padStart(3, "0")}`, parentId: id });
    await mount({ databaseId: id });
    const scroller = document.querySelector("[role=grid]")!.parentElement!;
    act(() => { scroller.scrollTop = 150 * 36; scroller.dispatchEvent(new Event("scroll")); });
    const row = [...document.querySelectorAll("[role=row]")].find(candidate => candidate.getAttribute("aria-rowindex") === String(150 + 2))!;
    const rating = row.querySelectorAll("[role=gridcell]")[1] as HTMLElement;
    const grid = document.querySelector("[role=grid]") as HTMLElement;
    act(() => { grid.focus(); rating.click(); });
    act(() => { grid.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true })); });
    const box = () => document.querySelector("input[aria-label=Rating]") as HTMLInputElement | null;
    act(() => { Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(box()!, "9"); box()!.dispatchEvent(new Event("input", { bubbles: true })); });
    // A refresh whose later pages are slow.
    const real = bridgeApi.queryDatabase.bind(bridgeApi);
    let release!: () => void;
    const gate = new Promise<void>(resolve => { release = resolve; });
    vi.spyOn(bridgeApi, "queryDatabase").mockImplementation(async params => { if (params.cursor) await gate; return real(params); });
    mockCreatePage({ title: "Elsewhere" });
    await settle();
    expect(grid.getAttribute("aria-rowcount")).toBe("251");
    expect(box()?.value).toBe("9");
    await act(async () => { release(); });
    await settle();
    expect(grid.getAttribute("aria-rowcount")).toBe("251");
    expect(box()?.value).toBe("9");
    expect(box()!.closest("[role=row]")!.getAttribute("aria-rowindex")).toBe(String(150 + 2));
  });

  it("re-reads the schema when a burst names the database before its rows", async () => {
    const { id, rating } = books();
    const row = mockCreatePage({ title: "DDIA", parentId: id });
    mockUpdateNote({ id: row.id, values: { [rating]: 4 } });
    await mount({ databaseId: id });
    expect(ratingCell(0).textContent).toBe("4");
    // The retype writes the database, then each converted row: one burst
    // whose last event names a row.
    mockUpdateSchema({ id, changes: [{ op: "retype", propertyId: rating, type: "text" }] });
    await settle();
    expect(mockReadPage(row.id).values).toEqual({ [rating]: "4" });
    expect(ratingCell(0).textContent).toBe("4");
    expect(document.querySelector("[role=columnheader][aria-label='Rating'] svg.lucide-text")).not.toBeNull();
  });

  it("composes overlapping view edits and keeps them in order", async () => {
    const made = mockCreateDatabase({ title: "Books", properties: [{ name: "Rating", type: "number" }, { name: "Genre", type: "select" }] });
    await mount({ databaseId: made.note.id });
    const real = bridgeApi.saveDatabaseView.bind(bridgeApi);
    let release!: () => void;
    const gate = new Promise<void>(resolve => { release = resolve; });
    let first = true;
    vi.spyOn(bridgeApi, "saveDatabaseView").mockImplementation(async params => {
      if (first) { first = false; await gate; }
      return real(params);
    });
    const hide = (name: string) => {
      act(() => (document.querySelector(`[role=columnheader][aria-label='${name}'] button`) as HTMLElement).click());
      act(() => [...document.querySelectorAll<HTMLButtonElement>("[role=menuitem]")].find(item => item.textContent === "Hide")!.click());
    };
    hide("Rating");
    // Hidden at once, before the save answers; the next edit builds on it.
    expect(document.querySelector("[role=columnheader][aria-label='Rating']")).toBeNull();
    hide("Genre");
    expect(document.querySelector("[role=columnheader][aria-label='Genre']")).toBeNull();
    await act(async () => { release(); });
    await settle();
    const stored = mockReadPage(made.note.id).database!.views[0].properties!;
    const visible = Object.fromEntries(stored.map(entry => [entry.id, entry.visible]));
    expect(visible).toEqual({ title: true, [made.database!.properties[0].id]: false, [made.database!.properties[1].id]: false });
    expect([...document.querySelectorAll("[role=columnheader]")].map(header => header.getAttribute("aria-label"))).toEqual(["Title"]);
  });

  it("never lets an older schema read replace a newer one", async () => {
    const { id, rating } = books();
    await mount({ databaseId: id });
    let release!: () => void;
    vi.spyOn(bridgeApi, "readNotePage").mockImplementationOnce(page => { const old = mockReadPage(page); return new Promise(resolve => { release = () => resolve(old); }); });
    mockUpdateSchema({ id, changes: [{ op: "rename", propertyId: rating, name: "Old rename" }] });
    await later();
    mockUpdateSchema({ id, changes: [{ op: "rename", propertyId: rating, name: "Newest rename" }] });
    await later();
    expect(document.querySelector("[role=columnheader][aria-label='Newest rename']")).not.toBeNull();
    await act(async () => { release(); });
    await later();
    expect(document.querySelector("[role=columnheader][aria-label='Newest rename']")).not.toBeNull();
  });
});


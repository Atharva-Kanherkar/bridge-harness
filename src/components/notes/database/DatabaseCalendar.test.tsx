// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { bridgeApi } from "../../../api";
import { mockCreateDatabase, mockCreatePage, mockReadPage, mockQueryDatabase, mockUpdateNote, resetMockNotes } from "../../../notesMock";
import type { DatabaseRow, NoteDatabaseView, QueryDatabaseParams } from "../../../types";
import { DatabaseCalendar, droppedValue, monthDays, placedDay } from "./DatabaseCalendar";
import { DatabaseList } from "./DatabaseList";
import type { ViewBodyProps } from "./viewBody";

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

const settle = () => act(async () => { for (let i = 0; i < 6; i += 1) await new Promise(resolve => setTimeout(resolve, 5)); });

function books() {
  const made = mockCreateDatabase({ title: "Books", properties: [{ name: "Due", type: "date" }, { name: "Genre", type: "select", options: [{ name: "Fiction" }, { name: "Tech" }] }] });
  const [due, genre] = made.database!.properties;
  const row = (title: string, values: Record<string, unknown>) => { const page = mockCreatePage({ title, parentId: made.note.id }); mockUpdateNote({ id: page.id, values }); return page.id; };
  return { made, due, genre, row };
}

function props(made: ReturnType<typeof books>["made"], view: NoteDatabaseView, overrides: Partial<ViewBodyProps> = {}): ViewBodyProps {
  const database = mockReadPage(made.note.id).database!;
  const result = mockQueryDatabase({ databaseId: made.note.id, groupBy: view.groupBy ?? null, groupLimit: 200 });
  return {
    database, view, query: { filter: null, sorts: [], groupBy: view.groupBy ?? null },
    rows: result.rows, groups: result.groups ?? null,
    run: (params: Omit<QueryDatabaseParams, "databaseId">) => bridgeApi.queryDatabase({ databaseId: made.note.id, ...params }),
    version: 0,
    host: { addOption: vi.fn(async () => null), searchRows: vi.fn(async () => []), rowTitle: () => undefined, workspaces: [] },
    onCommit: vi.fn(async (row: DatabaseRow, column, value) => { mockUpdateNote({ id: row.note.id, values: { [column.id]: value } }); return true; }),
    onNewRow: vi.fn(async () => null),
    onOpenRow: vi.fn(),
    ...overrides,
  };
}

const dayCell = (day: string) => document.querySelector(`[role=gridcell][aria-label='${day}']`)!;
const titlesOn = (day: string) => [...dayCell(day).querySelectorAll("button")].map(button => button.textContent);
const fire = (target: Element, type: string) => act(() => { target.dispatchEvent(new Event(type, { bubbles: true, cancelable: true })); });
const button = (name: string) => [...document.querySelectorAll<HTMLButtonElement>("button")].find(candidate => candidate.textContent === name || candidate.getAttribute("aria-label") === name)!;

describe("DatabaseCalendar", () => {
  const calendar = (dateProperty: string | null): NoteDatabaseView => ({ id: "cal", name: "Calendar", type: "calendar", order: 0, dateProperty });

  it("places single dates, ranges and datetimes on their days", async () => {
    const { made, due, row } = books();
    row("Single", { [due.id]: { start: "2026-11-12" } });
    row("Range", { [due.id]: { start: "2026-11-09", end: "2026-11-11" } });
    row("Late night", { [due.id]: { start: "2026-11-12T23:30:00-08:00" } });
    await act(async () => { root.render(<DatabaseCalendar {...props(made, calendar(due.id))} today="2026-11-05" />); });
    await settle();
    expect(document.querySelector("[role=grid]")!.getAttribute("aria-label")).toBe("November 2026");
    expect(monthDays("2026-11")[0]).toBe("2026-10-26");
    expect(monthDays("2026-11")).toHaveLength(42);
    expect(titlesOn("2026-11-12")).toEqual(["Single"]);
    expect(["2026-11-09", "2026-11-10", "2026-11-11"].map(titlesOn)).toEqual([["Range"], ["Range"], ["Range"]]);
    expect(titlesOn("2026-11-13")).toEqual(["Late night"]);
    expect(placedDay("2026-11-12T23:30:00-08:00")).toBe("2026-11-13");
  });

  it("shows a range that starts before the grid and runs into it", async () => {
    const { made, due, row } = books();
    row("Long read", { [due.id]: { start: "2026-10-25", end: "2026-11-03" } });
    row("Old", { [due.id]: { start: "2026-10-01", end: "2026-10-20" } });
    await act(async () => { root.render(<DatabaseCalendar {...props(made, calendar(due.id))} today="2026-11-05" />); });
    await settle();
    expect(titlesOn("2026-10-26")).toEqual(["Long read"]);
    expect(titlesOn("2026-11-03")).toEqual(["Long read"]);
    expect(titlesOn("2026-11-04")).toEqual([]);
    expect(document.querySelector("[role=grid]")!.textContent).not.toContain("Old");
  });

  it("reads every page of the month and the tray", async () => {
    const { made, due, row } = books();
    for (let index = 0; index < 501; index += 1) row(`Dated ${index}`, { [due.id]: { start: `2026-11-${String(1 + (index % 28)).padStart(2, "0")}` } });
    for (let index = 0; index < 101; index += 1) row(`Loose ${index}`, {});
    await act(async () => { root.render(<DatabaseCalendar {...props(made, calendar(due.id))} today="2026-11-05" />); });
    await settle();
    const placed = new Set([...document.querySelectorAll("[role=gridcell] button")].map(button => button.textContent));
    expect(placed.size).toBe(501);
    expect(document.querySelectorAll("aside[aria-label='No date'] button")).toHaveLength(101);
  });

  it("queries each shown month and moves with prev, next and today", async () => {
    const { made, due } = books();
    const query = vi.spyOn(bridgeApi, "queryDatabase");
    await act(async () => { root.render(<DatabaseCalendar {...props(made, calendar(due.id))} today="2026-11-05" />); });
    await settle();
    // Starts on or before the last shown day; overlap with the grid is kept client-side.
    expect(query.mock.calls[0][0].filter).toEqual({ and: [{ property: due.id, op: "onOrBefore", value: "2026-12-06" }] });
    act(() => button("Next month").click());
    await settle();
    expect(document.querySelector("[role=grid]")!.getAttribute("aria-label")).toBe("December 2026");
    expect(query.mock.calls.at(-2)![0].filter).toEqual({ and: [{ property: due.id, op: "onOrBefore", value: "2027-01-10" }] });
    act(() => button("Previous month").click());
    act(() => button("Previous month").click());
    expect(document.querySelector("[role=grid]")!.getAttribute("aria-label")).toBe("October 2026");
    act(() => button("Today").click());
    expect(document.querySelector("[role=grid]")!.getAttribute("aria-label")).toBe("November 2026");
  });

  it("dropping on a day keeps time and range length", async () => {
    expect(droppedValue({ start: "2026-11-09", end: "2026-11-11" }, "2026-11-20")).toEqual({ start: "2026-11-20", end: "2026-11-22" });
    expect(droppedValue({ start: "2026-11-12T23:30:00-08:00" }, "2026-11-20")).toEqual({ start: "2026-11-19T23:30:00-08:00" });
    expect(placedDay("2026-11-19T23:30:00-08:00")).toBe("2026-11-20");
    const { made, due, row } = books();
    const range = row("Range", { [due.id]: { start: "2026-11-09", end: "2026-11-11" } });
    await act(async () => { root.render(<DatabaseCalendar {...props(made, calendar(due.id))} today="2026-11-05" />); });
    await settle();
    fire(dayCell("2026-11-10").querySelector("button")!, "dragstart");
    fire(dayCell("2026-11-24"), "dragover");
    fire(dayCell("2026-11-24"), "drop");
    await settle();
    // The range starts on the day it is dropped on, three days long as before.
    expect(mockReadPage(range).values).toEqual({ [due.id]: { start: "2026-11-24", end: "2026-11-26" } });
    expect(["2026-11-24", "2026-11-25", "2026-11-26"].map(titlesOn)).toEqual([["Range"], ["Range"], ["Range"]]);
  });

  it("the No date tray holds undated rows and drags onto a day", async () => {
    const { made, due, row } = books();
    const loose = row("Loose", {});
    await act(async () => { root.render(<DatabaseCalendar {...props(made, calendar(due.id))} today="2026-11-05" />); });
    await settle();
    const tray = document.querySelector("aside[aria-label='No date']")!;
    expect(tray.textContent).toContain("Loose");
    fire(tray.querySelector("button")!, "dragstart");
    fire(dayCell("2026-11-17"), "drop");
    await settle();
    expect(mockReadPage(loose).values).toEqual({ [due.id]: { start: "2026-11-17" } });
    expect(titlesOn("2026-11-17")).toEqual(["Loose"]);
    expect(tray.textContent).not.toContain("Loose");
  });

  it("asks for a date property when there is none", async () => {
    const { made } = books();
    await act(async () => { root.render(<DatabaseCalendar {...props(made, calendar(null))} today="2026-11-05" />); });
    expect(document.querySelector("[role=status]")!.textContent).toBe("Add a date property to use a calendar");
  });
});

describe("composed filters", () => {
  const calendar = (dateProperty: string): NoteDatabaseView => ({ id: "cal", name: "Calendar", type: "calendar", order: 0, dateProperty });

  it("keeps a nested or a near-limit view filter within the planner's limits", async () => {
    const { made, due, genre, row } = books();
    const [fiction, tech] = genre.options!;
    row("Novel", { [due.id]: { start: "2026-11-12" }, [genre.id]: fiction.id });
    row("Manual", { [due.id]: { start: "2026-11-13" }, [genre.id]: tech.id });
    const nested = { and: [{ or: [{ property: genre.id, op: "is", value: fiction.id }, { property: genre.id, op: "is", value: tech.id }] }] };
    const given = props(made, calendar(due.id));
    await act(async () => { root.render(<DatabaseCalendar {...given} query={{ ...given.query, filter: nested }} today="2026-11-05" />); });
    await settle();
    expect(titlesOn("2026-11-12")).toEqual(["Novel"]);
    expect(titlesOn("2026-11-13")).toEqual(["Manual"]);
    // Twenty conditions already: the window cannot be added as two more.
    const full = { and: Array.from({ length: 20 }, (_, index) => ({ property: "title", op: "notContains", value: `zz${index}` })) };
    await act(async () => { root.render(<DatabaseCalendar {...given} key="full" query={{ ...given.query, filter: full }} today="2026-11-05" />); });
    await settle();
    expect(titlesOn("2026-11-12")).toEqual(["Novel"]);
    expect(document.querySelector("aside[aria-label='No date']")).not.toBeNull();
  });
});

describe("calendar fallback", () => {
  it("places Created rows when the window cannot be composed", async () => {
    const { made, row } = books();
    const id = row("Fresh", {});
    const day = mockReadPage(id).note.createdAt.slice(0, 10);
    const view: NoteDatabaseView = { id: "cal", name: "Calendar", type: "calendar", order: 0, dateProperty: "created" };
    const given = props(made, view);
    const full = { and: Array.from({ length: 20 }, (_, index) => ({ property: "title", op: "notContains", value: `zz${index}` })) };
    await act(async () => { root.render(<DatabaseCalendar {...given} query={{ ...given.query, filter: full }} today={day} />); });
    await settle();
    expect(titlesOn(new Date(mockReadPage(id).note.createdAt).toISOString().slice(0, 10))).toContain("Fresh");
  });
});

describe("DatabaseList", () => {
  const list = (groupBy: string | null): NoteDatabaseView => ({ id: "list", name: "List", type: "list", order: 0, groupBy: groupBy ? { property: groupBy } : null });

  it("shows rows with their properties and opens one", async () => {
    const { made, genre, row } = books();
    const ddia = row("DDIA", { [genre.id]: genre.options![1].id });
    const given = props(made, list(null));
    await act(async () => { root.render(<DatabaseList {...given} />); });
    const line = document.querySelector("[role=listitem]")!;
    expect(line.textContent).toBe("DDIATech");
    act(() => (line.querySelector("button") as HTMLButtonElement).click());
    expect(given.onOpenRow).toHaveBeenCalledWith(ddia);
  });

  it("groups into collapsible sections", async () => {
    const { made, genre, row } = books();
    row("A", { [genre.id]: genre.options![0].id });
    row("B", { [genre.id]: genre.options![1].id });
    row("C", {});
    await act(async () => { root.render(<DatabaseList {...props(made, list(genre.id))} />); });
    const sections = [...document.querySelectorAll("[aria-label=List] > section")];
    expect(sections.map(section => [section.getAttribute("aria-label"), section.querySelectorAll("[role=listitem]").length])).toEqual([["Fiction", 1], ["Tech", 1], ["No value", 1]]);
    act(() => (sections[0].querySelector("button[aria-expanded]") as HTMLButtonElement).click());
    expect(sections[0].querySelectorAll("[role=listitem]")).toHaveLength(0);
    expect(sections[0].querySelector("button")!.getAttribute("aria-expanded")).toBe("false");
  });

  it("windows rows that arrive after the list was empty against its real height", async () => {
    const { made, row } = books();
    const empty = props(made, list(null));
    await act(async () => { root.render(<DatabaseList {...empty} />); });
    expect(document.body.textContent).toContain("No rows yet");
    for (let index = 0; index < 80; index += 1) row(`Book ${index}`, {});
    await act(async () => { root.render(<DatabaseList {...props(made, list(null))} />); });
    expect(document.querySelectorAll("[role=listitem]")).toHaveLength(27);
  });
});


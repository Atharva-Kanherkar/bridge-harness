// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { DatabaseRow, Note, NoteDatabase, NoteDatabaseView } from "../../../types";
import { DatabaseTable, ROW_HEIGHT, windowOf, type DatabaseTableProps, type TableActions } from "./DatabaseTable";
import type { CellHost } from "./PropertyCell";

let host: HTMLDivElement;
let root: Root;
let height = 600;
beforeEach(() => {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  height = 600;
  Object.defineProperty(HTMLElement.prototype, "clientHeight", { configurable: true, get() { return height; } });
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

const DATABASE: NoteDatabase = {
  id: "db", scope: "children", views: [],
  properties: [
    { id: "p-rating", name: "Rating", type: "number", order: 0 },
    { id: "p-genre", name: "Genre", type: "select", order: 1, options: [{ id: "o-fic", name: "Fiction" }, { id: "o-tech", name: "Tech" }] },
    { id: "p-owned", name: "Owned", type: "checkbox", order: 2 },
    { id: "p-note", name: "Note", type: "text", order: 3 },
  ],
};
const VIEW: NoteDatabaseView = { id: "v", name: "Table", type: "table", order: 0, sorts: [{ property: "title", direction: "asc" }], filter: { and: [] }, properties: [{ id: "p-genre", visible: true }, { id: "p-note", visible: false, width: 300 }] };

const note = (id: string, title: string): Note => ({ id, title, kind: "note", state: "open", tags: [], sessionIds: [], spaceId: "personal", createdAt: "2026-11-01T00:00:00Z", updatedAt: "2026-11-01T00:00:00Z", createdBy: "person", lastActor: "person", preview: "", seq: 1, parentId: "db" });
const row = (index: number, values: Record<string, unknown> = {}): DatabaseRow => ({ note: note(`r${index}`, `Book ${index}`), values });

function actionSpies(): TableActions {
  return { rename: vi.fn(), retype: vi.fn(), setOptions: vi.fn(), sort: vi.fn(), hide: vi.fn(), remove: vi.fn(), resize: vi.fn(), show: vi.fn(), addProperty: vi.fn() };
}
const cellHost: CellHost = { addOption: vi.fn(async () => null), searchRows: vi.fn(async () => []), rowTitle: () => undefined, workspaces: [] };

function mount(overrides: Partial<DatabaseTableProps> = {}) {
  const props: DatabaseTableProps = {
    database: DATABASE, view: VIEW, rows: [row(0, { "p-rating": 4, "p-genre": "o-fic", "p-owned": true }), row(1, { "p-rating": 2 }), row(2)],
    host: cellHost, actions: actionSpies(), onCommit: vi.fn(), onNewRow: vi.fn(async () => null), onOpenRow: vi.fn(),
    ...overrides,
  };
  act(() => root.render(<DatabaseTable {...props} />));
  return { props, rerender: (next: Partial<DatabaseTableProps>) => act(() => root.render(<DatabaseTable {...props} {...next} />)) };
}

const grid = () => document.querySelector("[role=grid]") as HTMLElement;
const headers = () => [...document.querySelectorAll("[role=columnheader]")].map(header => header.getAttribute("aria-label"));
const bodyRows = () => [...document.querySelectorAll("[role=row]")].slice(1);
const cell = (r: number, c: number) => bodyRows()[r].querySelectorAll("[role=gridcell]")[c] as HTMLElement;
const selected = () => {
  const found = document.querySelector("[role=gridcell][aria-selected=true]");
  if (!found) return null;
  const rowEl = found.closest("[role=row]")!;
  return [Number(rowEl.getAttribute("aria-rowindex")) - 2, Number(found.getAttribute("aria-colindex")) - 1];
};
const press = (key: string, init: KeyboardEventInit = {}) => act(() => { (document.activeElement ?? grid()).dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true, ...init })); });
const input = (label: string) => document.querySelector(`input[aria-label="${label}"]`) as HTMLInputElement;
function type(element: HTMLInputElement, text: string) {
  const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!;
  act(() => { setter.call(element, text); element.dispatchEvent(new Event("input", { bubbles: true })); });
}
const click = (element: Element) => act(() => { (element as HTMLElement).click(); });
const menuItem = (name: string) => [...document.querySelectorAll<HTMLButtonElement>("[role=menuitem],[role=menuitemradio],button")].find(button => button.textContent?.trim().startsWith(name))!;

describe("DatabaseTable", () => {
  it("renders the title and visible columns in view order with their cells", () => {
    mount();
    expect(headers()).toEqual(["Title", "Genre", "Rating", "Owned"]);
    expect(bodyRows()).toHaveLength(3);
    expect(cell(0, 0).textContent).toContain("Book 0");
    expect(cell(0, 1).textContent).toBe("Fiction");
    expect(cell(0, 2).textContent).toBe("4");
    expect(cell(0, 3).querySelector("[aria-label=Checked]")).not.toBeNull();
    expect(cell(2, 3).querySelector("[aria-label='Not checked']")).not.toBeNull();
  });

  it("windows 1000 rows to the visible slice and keeps the full scroll height", () => {
    expect(windowOf(1000, 0, 600)).toEqual([0, 27]);
    expect(windowOf(1000, 3600, 600)).toEqual([90, 127]);
    expect(windowOf(5, 0, 600)).toEqual([0, 5]);
    const many = Array.from({ length: 1000 }, (_, index) => row(index));
    mount({ rows: many });
    expect(bodyRows()).toHaveLength(27);
    const spacers = [...grid().querySelectorAll("[aria-hidden=true]")].filter(element => (element as HTMLElement).style.height);
    expect(spacers.map(element => (element as HTMLElement).style.height)).toEqual([`${(1000 - 27) * ROW_HEIGHT}px`]);
    const scroller = grid().parentElement!;
    act(() => { scroller.scrollTop = 3600; scroller.dispatchEvent(new Event("scroll")); });
    expect(bodyRows()).toHaveLength(37);
    expect(bodyRows()[0].getAttribute("aria-rowindex")).toBe(String(90 + 2));
    expect(bodyRows().length).toBeLessThanOrEqual(37);
  });

  it("moves the selection with arrows and Tab, and stops at the edges", () => {
    mount();
    act(() => grid().focus());
    press("ArrowDown");
    expect(selected()).toEqual([0, 0]);
    press("ArrowUp");
    press("ArrowLeft");
    expect(selected()).toEqual([0, 0]);
    press("ArrowRight");
    press("ArrowRight");
    press("ArrowRight");
    press("ArrowRight");
    expect(selected()).toEqual([0, 3]);
    press("Tab");
    expect(selected()).toEqual([1, 0]);
    press("Tab", { shiftKey: true });
    expect(selected()).toEqual([0, 3]);
    press("ArrowDown");
    press("ArrowDown");
    press("ArrowDown");
    expect(selected()).toEqual([2, 3]);
  });

  it("Enter and typing open an editor; Enter commits and moves down; Escape keeps the old value", () => {
    const { props } = mount();
    act(() => grid().focus());
    click(cell(0, 2));
    press("Enter");
    type(input("Rating"), "5");
    act(() => { input("Rating").dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true })); });
    expect(props.onCommit).toHaveBeenCalledWith(props.rows[0], expect.objectContaining({ id: "p-rating" }), 5);
    expect(selected()).toEqual([1, 2]);
    // Typing seeds the editor with the key.
    press("7");
    expect(input("Rating").value).toBe("7");
    act(() => { input("Rating").dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true })); });
    expect(props.onCommit).toHaveBeenCalledTimes(1);
    expect(input("Rating")).toBeNull();
    expect(selected()).toEqual([1, 2]);
    // A title is edited the same way.
    press("ArrowLeft");
    press("ArrowLeft");
    press("Enter");
    type(input("Title"), "Renamed");
    act(() => { input("Title").dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true })); });
    expect(props.onCommit).toHaveBeenLastCalledWith(props.rows[1], expect.objectContaining({ id: "title" }), "Renamed");
    // Space toggles a checkbox, and so does a click on a selected one.
    click(cell(0, 3));
    press(" ");
    expect(props.onCommit).toHaveBeenLastCalledWith(props.rows[0], expect.objectContaining({ id: "p-owned" }), false);
    click(cell(0, 3));
    expect(props.onCommit).toHaveBeenCalledTimes(4);
  });

  it("Cmd+Enter on a title opens the row page", () => {
    const { props } = mount();
    act(() => grid().focus());
    click(cell(1, 0));
    press("Enter", { metaKey: true });
    expect(props.onOpenRow).toHaveBeenCalledWith("r1");
    click(cell(1, 2));
    press("Enter", { ctrlKey: true });
    expect(props.onOpenRow).toHaveBeenCalledTimes(1);
    // The title's Open button opens it too, without selecting the cell.
    click(cell(2, 0).querySelector("button[aria-label='Open Book 2']")!);
    expect(props.onOpenRow).toHaveBeenLastCalledWith("r2");
  });

  it("header menu renames, retypes, edits options and deletes through update_schema", () => {
    const { props } = mount();
    const genre = DATABASE.properties[1];
    const openMenu = (name: string) => click(document.querySelector(`[role=columnheader][aria-label='${name}'] button`)!);
    openMenu("Genre");
    type(input("Property name"), "Kind");
    act(() => { input("Property name").dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true })); });
    expect(props.actions.rename).toHaveBeenCalledWith(genre, "Kind");

    openMenu("Genre");
    click(menuItem("Type: Select"));
    const text = [...document.querySelectorAll("[role=menuitemradio]")].find(item => item.textContent?.startsWith("Text"))!;
    expect(text.textContent).toContain("Keeps every value.");
    expect([...document.querySelectorAll("[role=menuitemradio]")].find(item => item.textContent?.startsWith("Date"))!.textContent).toContain("Clears every value.");
    click(text);
    expect(props.actions.retype).toHaveBeenCalledWith(genre, "text");

    openMenu("Genre");
    click(menuItem("Edit options"));
    type(input("Option 1 name"), "Novels");
    click(document.querySelector("button[aria-label='Delete Tech']")!);
    type(input("New option"), "History");
    act(() => { input("New option").dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true })); });
    click(document.querySelector("button[aria-label='Move History up']")!);
    click(menuItem("Save options"));
    expect(props.actions.setOptions).toHaveBeenCalledWith(genre, [{ name: "History" }, { id: "o-fic", name: "Novels" }], ["o-fic", "o-tech"]);

    openMenu("Rating");
    click(menuItem("Delete property"));
    expect(document.querySelector("[role=alertdialog]")!.textContent).toContain("Delete Rating? 2 rows have a value.");
    click(menuItem("Delete"));
    expect(props.actions.remove).toHaveBeenCalledWith(DATABASE.properties[0]);

    // The title column offers only the sorts.
    openMenu("Title");
    const items = [...document.querySelectorAll("[role=menu] [role=menuitem]")].map(item => item.textContent);
    expect(items).toEqual(["Sort ascending", "Sort descending"]);
  });

  it("sort and hide save the view and keep its other fields", () => {
    const { props } = mount();
    click(document.querySelector("[role=columnheader][aria-label='Rating'] button")!);
    click(menuItem("Sort descending"));
    expect(props.actions.sort).toHaveBeenCalledWith(expect.objectContaining({ id: "p-rating" }), "desc");
    click(document.querySelector("[role=columnheader][aria-label='Genre'] button")!);
    click(menuItem("Hide"));
    expect(props.actions.hide).toHaveBeenCalledWith(expect.objectContaining({ id: "p-genre" }));
  });

  it("a dragged column width saves to the view; a click saves nothing", () => {
    const { props } = mount();
    const handle = document.querySelector("[aria-label='Resize Rating']")!;
    const pointer = (target: EventTarget, kind: string, clientX: number) => act(() => { target.dispatchEvent(Object.assign(new MouseEvent(kind, { bubbles: true, clientX }), { pointerId: 1 })); });
    pointer(handle, "pointerdown", 100);
    pointer(window, "pointerup", 101);
    expect(props.actions.resize).not.toHaveBeenCalled();
    pointer(handle, "pointerdown", 100);
    pointer(window, "pointermove", 140);
    expect((document.querySelector("[role=columnheader][aria-label='Rating']") as HTMLElement).style.width).toBe("160px");
    pointer(window, "pointerup", 160);
    expect(props.actions.resize).toHaveBeenCalledWith(expect.objectContaining({ id: "p-rating" }), 180);
  });

  it("+ New makes a row and selects its title", async () => {
    const onNewRow = vi.fn(async () => "r9");
    const { props, rerender } = mount({ onNewRow });
    await act(async () => { menuItem("New").click(); });
    expect(onNewRow).toHaveBeenCalled();
    rerender({ rows: [...props.rows, row(9)] });
    expect(selected()).toEqual([3, 0]);
    expect(input("Title")).not.toBeNull();
  });

  it("+ shows a hidden column again", () => {
    const { props } = mount();
    click(document.querySelector("button[aria-label='Add a property']")!);
    expect([...document.querySelectorAll("button[aria-label^='Show ']")].map(button => button.getAttribute("aria-label"))).toEqual(["Show Note", "Show Project", "Show Chats", "Show Created", "Show Updated"]);
    click(document.querySelector("button[aria-label='Show Note']")!);
    expect(props.actions.show).toHaveBeenCalledWith(expect.objectContaining({ id: "p-note" }));
    click(document.querySelector("button[aria-label='Add a property']")!);
    type(input("New property name"), "Due");
    click([...document.querySelectorAll("[role=radio]")].find(radio => radio.textContent === "Date")!);
    click(menuItem("Add"));
    expect(props.actions.addProperty).toHaveBeenCalledWith("Due", "date");
  });
});

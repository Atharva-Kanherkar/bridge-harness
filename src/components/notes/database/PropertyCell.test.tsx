// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { NoteProperty } from "../../../types";
import * as links from "../../../externalLinks";
import { PropertyCell, withOffset, type CellHost, type PropertyCellProps } from "./PropertyCell";

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
  document.body.innerHTML = "";
  vi.restoreAllMocks();
});

const cellHost = (overrides: Partial<CellHost> = {}): CellHost => ({
  addOption: vi.fn(async () => null),
  searchRows: vi.fn(async () => []),
  rowTitle: () => undefined,
  workspaces: [],
  ...overrides,
});

const property = (type: string, extra: Partial<NoteProperty> = {}): NoteProperty => ({ id: `p-${type}`, name: type, type, order: 0, ...extra });
const genre = property("select", { name: "Genre", options: [{ id: "o-fic", name: "Fiction" }, { id: "o-tech", name: "Tech" }] });

function mount(props: Partial<PropertyCellProps> & Pick<PropertyCellProps, "column">) {
  const onCommit = props.onCommit ?? vi.fn();
  const given = { value: undefined, host: cellHost(), ...props, onCommit } as PropertyCellProps;
  act(() => root.render(<PropertyCell {...given} />));
  return { onCommit: onCommit as ReturnType<typeof vi.fn>, rerender: (next: Partial<PropertyCellProps>) => act(() => root.render(<PropertyCell {...given} {...next} />)) };
}

const columnOf = (p: NoteProperty) => ({ id: p.id, name: p.name, type: p.type, property: p });
const open = (name: string) => act(() => (document.querySelector(`button[aria-label="${name}"]`) as HTMLButtonElement).click());
const input = (label: string) => document.querySelector(`input[aria-label="${label}"]`) as HTMLInputElement;
function type(element: HTMLInputElement, text: string) {
  const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!;
  act(() => { setter.call(element, text); element.dispatchEvent(new Event("input", { bubbles: true })); });
}
const key = (element: Element, name: string) => act(() => { element.dispatchEvent(new KeyboardEvent("keydown", { key: name, bubbles: true })); });
const option = (name: string) => [...document.querySelectorAll<HTMLButtonElement>("[role=option]")].find(button => button.textContent?.includes(name))!;
const flush = () => act(async () => { await new Promise(resolve => setTimeout(resolve, 0)); });

describe("PropertyCell", () => {
  it("text commits trimmed text and null when emptied", () => {
    const { onCommit } = mount({ column: columnOf(property("text", { name: "Note" })), value: "old" });
    open("Note");
    type(input("Note"), "new words  ");
    key(input("Note"), "Enter");
    expect(onCommit).toHaveBeenLastCalledWith("new words");
    open("Note");
    type(input("Note"), "   ");
    act(() => input("Note").blur());
    expect(onCommit).toHaveBeenLastCalledWith(null);
  });

  it("number refuses NaN and commits a finite number", () => {
    const { onCommit } = mount({ column: columnOf(property("number", { name: "Rating" })), value: 3 });
    open("Rating");
    expect(input("Rating").value).toBe("3");
    type(input("Rating"), "four");
    expect(input("Rating").getAttribute("aria-invalid")).toBe("true");
    key(input("Rating"), "Enter");
    expect(onCommit).not.toHaveBeenCalled();
    expect(input("Rating")).not.toBeNull();
    type(input("Rating"), "4.5");
    key(input("Rating"), "Enter");
    expect(onCommit).toHaveBeenCalledWith(4.5);
    // An unchanged value is never sent.
    onCommit.mockClear();
    open("Rating");
    key(input("Rating"), "Enter");
    expect(onCommit).not.toHaveBeenCalled();
  });

  it("select picks, clears and creates an option against the stored list", async () => {
    const stored = { ...genre, options: [...genre.options!, { id: "o-hist", name: "History" }] };
    const addOption = vi.fn(async () => stored);
    const { onCommit, rerender } = mount({ column: columnOf(genre), value: undefined, host: cellHost({ addOption }) });
    open("Genre");
    act(() => option("Tech").click());
    expect(onCommit).toHaveBeenLastCalledWith("o-tech");
    rerender({ value: "o-tech" });
    open("Genre");
    act(() => option("Tech").click());
    expect(onCommit).toHaveBeenLastCalledWith(null);
    open("Genre");
    type(input("Search Genre options"), "History");
    const create = [...document.querySelectorAll("button")].find(button => button.textContent === "Create History")!;
    await act(async () => { create.click(); });
    expect(addOption).toHaveBeenCalledWith(genre, { name: "History" });
    expect(onCommit).toHaveBeenLastCalledWith("o-hist");
  });

  it("status groups its options under their groups", () => {
    const status = property("status", { name: "Status", options: [{ id: "a", name: "Done", group: "done" }, { id: "b", name: "Reading", group: "inProgress" }, { id: "c", name: "To read", group: "todo" }] });
    mount({ column: columnOf(status) });
    open("Status");
    const groups = [...document.querySelectorAll("[role=group]")].map(group => [group.getAttribute("aria-label"), [...group.querySelectorAll("[role=option]")].map(item => item.textContent)]);
    expect(groups).toEqual([["To do", ["To read"]], ["In progress", ["Reading"]], ["Done", ["Done"]]]);
  });

  it("multiSelect toggles in choice order", () => {
    const labels = property("multiSelect", { name: "Labels", options: [{ id: "s", name: "Short" }, { id: "l", name: "Long" }, { id: "g", name: "Gift" }] });
    const { onCommit, rerender } = mount({ column: columnOf(labels), value: ["g"] });
    open("Labels");
    act(() => option("Short").click());
    expect(onCommit).toHaveBeenLastCalledWith(["g", "s"]);
    rerender({ value: ["g", "s"] });
    act(() => option("Gift").click());
    expect(onCommit).toHaveBeenLastCalledWith(["s"]);
    // The picker stays open for the next choice.
    expect(document.querySelector("[role=dialog][aria-label='Labels editor']")).not.toBeNull();
  });

  it("date commits a day, a range and a datetime, and refuses an end before the start", () => {
    const due = property("date", { name: "Due" });
    const { onCommit, rerender } = mount({ column: columnOf(due) });
    open("Due");
    type(input("Due start"), "2026-11-12");
    key(input("Due start"), "Enter");
    expect(onCommit).toHaveBeenLastCalledWith({ start: "2026-11-12" });

    rerender({ value: { start: "2026-11-12" } });
    open("Due");
    act(() => [...document.querySelectorAll("label")].find(label => label.textContent === "End date")!.querySelector("input")!.click());
    type(input("Due end"), "2026-11-10");
    expect(document.querySelector("[role=alert]")?.textContent).toBe("The end is before the start.");
    key(input("Due end"), "Enter");
    expect(onCommit).toHaveBeenCalledTimes(1);
    type(input("Due end"), "2026-11-14");
    key(input("Due end"), "Enter");
    expect(onCommit).toHaveBeenLastCalledWith({ start: "2026-11-12", end: "2026-11-14" });

    rerender({ value: { start: "2026-11-12" } });
    open("Due");
    act(() => [...document.querySelectorAll("label")].find(label => label.textContent === "Include time")!.querySelector("input")!.click());
    expect(input("Due start").type).toBe("datetime-local");
    type(input("Due start"), "2026-11-12T15:30");
    key(input("Due start"), "Enter");
    expect(onCommit).toHaveBeenLastCalledWith({ start: withOffset("2026-11-12T15:30") });
    expect(withOffset("2026-11-12T15:30")).toMatch(/^2026-11-12T15:30:00[+-]\d{2}:\d{2}$/);
  });

  it("checkbox toggles on click and Space", () => {
    const owned = property("checkbox", { name: "Owned" });
    const { onCommit, rerender } = mount({ column: columnOf(owned) });
    open("Owned");
    expect(onCommit).toHaveBeenLastCalledWith(true);
    rerender({ value: true });
    key(document.querySelector("button[aria-label='Owned']")!, " ");
    expect(onCommit).toHaveBeenLastCalledWith(false);
  });

  it("url opens only http, https and mailto links through openExternalUrl", () => {
    const opened = vi.spyOn(links, "openExternalUrl").mockResolvedValue();
    const link = property("url", { name: "Link" });
    const { rerender } = mount({ column: columnOf(link), value: "https://example.com/a", mode: "cell" });
    act(() => (document.querySelector("button[aria-label^='Open link']") as HTMLButtonElement).click());
    expect(opened).toHaveBeenCalledWith("https://example.com/a");
    rerender({ value: "mailto:a@example.com" });
    expect(document.querySelector("button[aria-label^='Open link']")).not.toBeNull();
    rerender({ value: "javascript:alert(1)" });
    expect(document.querySelector("button[aria-label^='Open link']")).toBeNull();
    rerender({ value: "file:///etc/passwd" });
    expect(document.querySelector("button[aria-label^='Open link']")).toBeNull();
  });

  it("relation searches the target database and toggles a row", async () => {
    const searchRows = vi.fn(async (_db: string, query: string) => [{ id: "r1", title: "DDIA" }, { id: "r2", title: "SICP" }].filter(row => row.title.toLowerCase().includes(query.toLowerCase())));
    const related = property("relation", { name: "Books", relation: { databaseId: "db-books" } });
    const { onCommit } = mount({ column: columnOf(related), value: ["r1"], host: cellHost({ searchRows }) });
    open("Books");
    await flush();
    expect(searchRows).toHaveBeenLastCalledWith("db-books", "");
    type(input("Search Books pages"), "sic");
    await flush();
    expect(searchRows).toHaveBeenLastCalledWith("db-books", "sic");
    act(() => option("SICP").click());
    expect(onCommit).toHaveBeenLastCalledWith(["r1", "r2"]);
    act(() => option("DDIA").click());
    expect(onCommit).toHaveBeenLastCalledWith(["r2"]);
  });

  it("Escape cancels every editor without committing", () => {
    const cases: [NoteProperty, string][] = [
      [property("text", { name: "Note" }), "Note"],
      [property("number", { name: "Rating" }), "Rating"],
      [property("url", { name: "Link" }), "Link"],
      [genre, "Search Genre options"],
      [property("date", { name: "Due" }), "Due start"],
    ];
    for (const [p, field] of cases) {
      const { onCommit } = mount({ column: columnOf(p), value: undefined });
      open(p.name);
      const box = input(field);
      if (p.type !== "select") type(box, p.type === "date" ? "2026-11-12" : "9");
      key(box, "Escape");
      expect(onCommit).not.toHaveBeenCalled();
      expect(document.querySelector(`[role=dialog][aria-label='${p.name} editor']`)).toBeNull();
      expect(input(p.name)).toBeNull();
      act(() => root.render(<></>));
    }
  });
});

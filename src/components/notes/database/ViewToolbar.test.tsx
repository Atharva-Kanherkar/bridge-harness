// @vitest-environment jsdom
import { act, useState } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { NoteDatabase, NoteDatabaseView, Workspace } from "../../../types";
import { FilterBuilder } from "./FilterBuilder";
import { useViewOverride, type ViewOverride, type ViewQuery } from "./useViewOverride";
import { ViewToolbar } from "./ViewToolbar";

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
});

const DATABASE: NoteDatabase = { id: "db", scope: "children", views: [], properties: [
  { id: "p-rating", name: "Rating", type: "number", order: 0 },
  { id: "p-status", name: "Status", type: "status", order: 1, options: [{ id: "o-todo", name: "To read", group: "todo" }, { id: "o-done", name: "Read", group: "done" }] },
  { id: "p-due", name: "Due", type: "date", order: 2 },
  { id: "p-note", name: "Note", type: "text", order: 3 },
] };
const VIEW: NoteDatabaseView = { id: "v", name: "Table", type: "table", order: 0, filter: { and: [{ property: "p-rating", op: "gte", value: 3 }] }, sorts: [{ property: "title", direction: "asc" }], properties: [{ id: "p-note", visible: false }] };

const select = (label: string) => document.querySelector(`select[aria-label="${label}"]`) as HTMLSelectElement;
const input = (label: string) => document.querySelector(`input[aria-label="${label}"]`) as HTMLInputElement;
function choose(element: HTMLSelectElement, value: string) {
  act(() => { element.value = value; element.dispatchEvent(new Event("change", { bubbles: true })); });
}
function type(element: HTMLInputElement, text: string) {
  const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!;
  act(() => { setter.call(element, text); element.dispatchEvent(new Event("input", { bubbles: true })); });
}
const button = (name: string) => [...document.querySelectorAll<HTMLButtonElement>("button")].find(candidate => candidate.textContent?.trim() === name || candidate.getAttribute("aria-label") === name)!;
const click = (element: Element) => act(() => (element as HTMLElement).click());

describe("useViewOverride", () => {
  function Harness({ out }: { out: { current: ViewOverride | null } }) {
    out.current = useViewOverride();
    return null;
  }
  const mount = () => {
    const out: { current: ViewOverride | null } = { current: null };
    act(() => root.render(<Harness out={out} />));
    return out;
  };

  it("applies an override at once and saves it with every other field kept", async () => {
    const out = mount();
    expect(out.current!.effective(VIEW).filter).toEqual({ and: [{ property: "p-rating", op: "gte", value: 3 }] });
    act(() => out.current!.change(VIEW, { sorts: [{ property: "p-rating", direction: "desc" }] }));
    expect(out.current!.dirty(VIEW)).toBe(true);
    expect(out.current!.effective(VIEW).sorts).toEqual([{ property: "p-rating", direction: "desc" }]);
    const saved: ViewQuery[] = [];
    await act(async () => { await out.current!.save(VIEW, query => { saved.push(query); }); });
    expect(saved).toEqual([{ filter: { and: [{ property: "p-rating", op: "gte", value: 3 }] }, sorts: [{ property: "p-rating", direction: "desc" }], groupBy: null }]);
    expect(out.current!.dirty(VIEW)).toBe(false);
  });

  it("reset drops the override, and editing back to the saved view is no override", () => {
    const out = mount();
    act(() => out.current!.change(VIEW, { groupBy: { property: "p-status" } }));
    expect(out.current!.dirty(VIEW)).toBe(true);
    act(() => out.current!.reset(VIEW));
    expect(out.current!.dirty(VIEW)).toBe(false);
    act(() => out.current!.change(VIEW, { sorts: [] }));
    act(() => out.current!.change(VIEW, { sorts: [{ property: "title", direction: "asc" }] }));
    expect(out.current!.dirty(VIEW)).toBe(false);
  });

  it("a save clears only the override it saved", async () => {
    const out = mount();
    act(() => out.current!.change(VIEW, { sorts: [{ property: "p-rating", direction: "desc" }] }));
    let release!: () => void;
    const gate = new Promise<void>(resolve => { release = resolve; });
    let saving!: Promise<void>;
    act(() => { saving = out.current!.save(VIEW, async () => { await gate; }); });
    // Edited again while the save is out.
    act(() => out.current!.change(VIEW, { filter: { and: [{ property: "p-note", op: "contains", value: "x" }] } }));
    await act(async () => { release(); await saving; });
    expect(out.current!.dirty(VIEW)).toBe(true);
    expect(out.current!.effective(VIEW).filter).toEqual({ and: [{ property: "p-note", op: "contains", value: "x" }] });
  });

  it("keeps each tab's override apart", () => {
    const out = mount();
    const other = { ...VIEW, id: "w" };
    act(() => out.current!.change(VIEW, { sorts: [] }));
    expect(out.current!.dirty(VIEW)).toBe(true);
    expect(out.current!.dirty(other)).toBe(false);
    expect(out.current!.effective(other).sorts).toEqual([{ property: "title", direction: "asc" }]);
  });
});

describe("FilterBuilder", () => {
  function mount(value: unknown = null) {
    const onChange = vi.fn();
    act(() => root.render(<FilterBuilder database={DATABASE} value={value} onChange={onChange} />));
    return onChange;
  }

  it("builds each op's JSON from the rows", () => {
    const onChange = mount();
    click(button("Add a filter"));
    expect(onChange).toHaveBeenLastCalledWith({ and: [{ property: "p-rating", op: "eq", value: 0 }] });
    choose(select("Filter 1 op"), "gte");
    type(input("Filter 1 value"), "4");
    expect(onChange).toHaveBeenLastCalledWith({ and: [{ property: "p-rating", op: "gte", value: 4 }] });
    choose(select("Filter 1 property"), "p-status");
    choose(select("Filter 1 op"), "isAnyOf");
    click([...document.querySelectorAll("label")].find(label => label.textContent === "Read")!.querySelector("input")!);
    // "is To read" became "is any of [To read]"; Read joins it.
    expect(onChange).toHaveBeenLastCalledWith({ and: [{ property: "p-status", op: "isAnyOf", value: ["o-todo", "o-done"] }] });
    choose(select("Filter 1 op"), "inGroup");
    choose(select("Filter 1 value"), "done");
    expect(onChange).toHaveBeenLastCalledWith({ and: [{ property: "p-status", op: "inGroup", value: "done" }] });
    choose(select("Filter 1 property"), "p-due");
    choose(select("Filter 1 op"), "within");
    choose(select("Filter 1 value"), "thisMonth");
    expect(onChange).toHaveBeenLastCalledWith({ and: [{ property: "p-due", op: "within", value: "thisMonth" }] });
    choose(select("Filter 1 op"), "isEmpty");
    expect(input("Filter 1 value")).toBeNull();
    expect(onChange).toHaveBeenLastCalledWith({ and: [{ property: "p-due", op: "isEmpty" }] });
    click(button("Remove filter 1"));
    expect(onChange).toHaveBeenLastCalledWith(null);
  });

  it("adds a group one level deep and switches and/or", () => {
    const onChange = mount({ and: [{ property: "p-note", op: "contains", value: "x" }] });
    click(button("Add a filter group"));
    click(button("Add to group"));
    expect(onChange).toHaveBeenLastCalledWith({ and: [{ property: "p-note", op: "contains", value: "x" }, { or: [{ property: "p-rating", op: "eq", value: 0 }, { property: "p-rating", op: "eq", value: 0 }] }] });
    // Groups nest one deep: inside a group there is no "Add a filter group".
    expect([...document.querySelectorAll("li li button")].some(candidate => candidate.textContent === "Add a filter group")).toBe(false);
    click([...document.querySelectorAll("[role=radiogroup][aria-label=Match] [role=radio]")].find(radio => radio.textContent === "Any")!);
    click([...document.querySelectorAll("[role=radiogroup][aria-label='Match in this group'] [role=radio]")].find(radio => radio.textContent === "All")!);
    expect(onChange).toHaveBeenLastCalledWith({ or: [{ property: "p-note", op: "contains", value: "x" }, { and: [{ property: "p-rating", op: "eq", value: 0 }, { property: "p-rating", op: "eq", value: 0 }] }] });
  });

  it("shows the number each row applies when rows change under it", () => {
    const database: NoteDatabase = { ...DATABASE, properties: [...DATABASE.properties, { id: "p-pages", name: "Pages", type: "number", order: 4 }] };
    const onChange = vi.fn();
    act(() => root.render(<FilterBuilder database={database} value={{ and: [{ property: "p-rating", op: "gte", value: 5 }, { property: "p-rating", op: "lte", value: 10 }] }} onChange={onChange} />));
    click(button("Remove filter 1"));
    expect(onChange).toHaveBeenLastCalledWith({ and: [{ property: "p-rating", op: "lte", value: 10 }] });
    expect(input("Filter 1 value").value).toBe("10");
    choose(select("Filter 1 property"), "p-pages");
    expect(onChange).toHaveBeenLastCalledWith({ and: [{ property: "p-pages", op: "eq", value: 0 }] });
    expect(input("Filter 1 value").value).toBe("0");
  });

  it("starts a Project filter on a real project, and shows Choose… for none", () => {
    const onChange = vi.fn();
    const workspaces = [{ id: "w1", title: "Only project" }] as unknown as Workspace[];
    act(() => root.render(<FilterBuilder database={DATABASE} value={null} onChange={onChange} workspaces={workspaces} />));
    click(button("Add a filter"));
    choose(select("Filter 1 property"), "project");
    expect(onChange).toHaveBeenLastCalledWith({ and: [{ property: "project", op: "is", value: "w1" }] });
    act(() => root.render(<FilterBuilder key="none" database={DATABASE} value={{ and: [{ property: "project", op: "is", value: "" }] }} onChange={onChange} workspaces={workspaces} />));
    expect(select("Filter 1 value").value).toBe("");
    expect(select("Filter 1 value").selectedOptions[0].textContent).toBe("Choose…");
  });

  it("keeps unfinished text with its own row when a row above goes", () => {
    const onChange = vi.fn();
    act(() => root.render(<FilterBuilder database={DATABASE} value={{ and: [{ property: "p-rating", op: "gte", value: 5 }, { property: "p-rating", op: "lte", value: 10 }] }} onChange={onChange} />));
    type(input("Filter 2 value"), "4x");
    click(button("Remove filter 1"));
    expect(onChange).toHaveBeenLastCalledWith(null);
    expect(input("Filter 1 value").value).toBe("4x");
    expect(input("Filter 1 value").getAttribute("aria-invalid")).toBe("true");
    expect(select("Filter 1 op").value).toBe("lte");
  });

  it("leaves an unfinished number out", () => {
    const onChange = mount({ and: [{ property: "p-rating", op: "gte", value: 3 }] });
    type(input("Filter 1 value"), "4x");
    expect(input("Filter 1 value").getAttribute("aria-invalid")).toBe("true");
    expect(onChange).toHaveBeenLastCalledWith(null);
    type(input("Filter 1 value"), "4.5");
    expect(onChange).toHaveBeenLastCalledWith({ and: [{ property: "p-rating", op: "gte", value: 4.5 }] });
  });
});

describe("ViewToolbar", () => {
  function Mounted({ view, onSave }: { view: NoteDatabaseView; onSave: (query: ViewQuery) => void }) {
    const override = useViewOverride();
    const [savedView, setSaved] = useState(view);
    return <ViewToolbar
      database={DATABASE}
      view={savedView}
      query={override.effective(savedView)}
      dirty={override.dirty(savedView)}
      onChange={patch => override.change(savedView, patch)}
      onReset={() => override.reset(savedView)}
      onSave={() => void override.save(savedView, query => { onSave(query); setSaved({ ...savedView, ...query }); })}
    />;
  }

  it("sorts reorder and stop at five", () => {
    act(() => root.render(<Mounted view={VIEW} onSave={vi.fn()} />));
    click(button("Sort, 1"));
    for (let index = 0; index < 6; index += 1) if (button("Add a sort")) click(button("Add a sort"));
    expect(document.querySelectorAll("[aria-label=Sorts] li")).toHaveLength(5);
    expect(button("Add a sort")).toBeUndefined();
    expect(select("Sort 2 property").value).toBe("p-rating");
    click(button("Move sort 2 up"));
    expect(select("Sort 1 property").value).toBe("p-rating");
    expect(select("Sort 2 property").value).toBe("title");
    expect(button("Save for everyone")).toBeDefined();
  });

  it("saves for everyone and resets", () => {
    const onSave = vi.fn();
    act(() => root.render(<Mounted view={VIEW} onSave={onSave} />));
    click(button("Group"));
    choose(select("Group by"), "p-status");
    click(button("Reset"));
    expect(button("Save for everyone")).toBeUndefined();
    // The panel stays open through a reset; open it only if it closed.
    if (!select("Group by")) click(button("Group"));
    choose(select("Group by"), "p-due");
    expect(select("Date bucket").value).toBe("month");
    click(button("Save for everyone"));
    expect(onSave).toHaveBeenCalledWith({ filter: { and: [{ property: "p-rating", op: "gte", value: 3 }] }, sorts: [{ property: "title", direction: "asc" }], groupBy: { property: "p-due", dateBucket: "month" } });
  });

  it("board has no None grouping and calendar no group", () => {
    act(() => root.render(<Mounted view={{ ...VIEW, type: "board", groupBy: { property: "p-status" } }} onSave={vi.fn()} />));
    click(button("Group, 1"));
    expect([...select("Group by").options].map(option => option.textContent)).not.toContain("None");
    act(() => root.render(<Mounted key="cal" view={{ ...VIEW, type: "calendar" }} onSave={vi.fn()} />));
    expect(button("Group")).toBeUndefined();
  });
});

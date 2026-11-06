import { describe, expect, it } from "vitest";
import type { NoteDatabase } from "../../../types";
import { addCondition, addGroup, canAdd, completeOnly, countConditions, defaultCondition, filterColumns, groupableColumns, opsFor, readFilter, removeAt, retarget, setCondition, setJoin, sortableColumns, writeFilter, type FieldColumn, type Group } from "./viewModel";

const DATABASE: NoteDatabase = { id: "db", scope: "children", views: [], properties: [
  { id: "p-rating", name: "Rating", type: "number", order: 0 },
  { id: "p-status", name: "Status", type: "status", order: 1, options: [{ id: "o-todo", name: "To read", group: "todo" }, { id: "o-done", name: "Read", group: "done" }] },
  { id: "p-labels", name: "Labels", type: "multiSelect", order: 2, options: [{ id: "o-short", name: "Short" }] },
  { id: "p-due", name: "Due", type: "date", order: 3 },
  { id: "p-owned", name: "Owned", type: "checkbox", order: 4 },
  { id: "p-note", name: "Note", type: "text", order: 5 },
  { id: "p-rel", name: "Pairs", type: "relation", order: 6 },
] };
const columns = filterColumns(DATABASE);
const col = (id: string) => columns.find(column => column.id === id)!;

describe("viewModel", () => {
  it("lists each type's ops as the planner does", () => {
    expect(opsFor(col("p-note"))).toEqual(["contains", "notContains", "equals", "notEquals", "startsWith", "isEmpty", "isNotEmpty"]);
    expect(opsFor(col("title"))).toEqual(opsFor(col("p-note")));
    expect(opsFor(col("p-rating"))).toEqual(["eq", "ne", "lt", "lte", "gt", "gte", "isEmpty", "isNotEmpty"]);
    expect(opsFor(col("p-due"))).toEqual(["before", "after", "onOrBefore", "onOrAfter", "equals", "isEmpty", "isNotEmpty", "within"]);
    expect(opsFor(col("updated"))).toEqual(opsFor(col("p-due")));
    expect(opsFor(col("p-status"))).toEqual(["is", "isNot", "isAnyOf", "isNoneOf", "isEmpty", "isNotEmpty", "inGroup"]);
    expect(opsFor(col("kind"))).toEqual(["is", "isNot", "isAnyOf", "isNoneOf", "isEmpty", "isNotEmpty"]);
    expect(opsFor(col("p-labels"))).toEqual(["contains", "notContains", "containsAny", "isEmpty", "isNotEmpty"]);
    expect(opsFor(col("tags"))).toEqual(opsFor(col("p-rel")));
    expect(opsFor(col("p-owned"))).toEqual(["is"]);
    expect(opsFor(col("chats"))).toEqual(["isEmpty", "isNotEmpty"]);
    expect(opsFor(col("parent"))).toEqual(["isEmpty", "isNotEmpty", "equals", "notEquals"]);
  });

  it("gives every column a default condition of the right shape", () => {
    expect(defaultCondition(col("p-note"))).toEqual({ property: "p-note", op: "contains", value: "" });
    expect(defaultCondition(col("p-rating"))).toEqual({ property: "p-rating", op: "eq", value: 0 });
    expect(defaultCondition(col("p-owned"))).toEqual({ property: "p-owned", op: "is", value: false });
    expect(defaultCondition(col("p-status"))).toEqual({ property: "p-status", op: "is", value: "o-todo" });
    expect(defaultCondition(col("state"))).toEqual({ property: "state", op: "is", value: "inbox" });
    expect(defaultCondition(col("p-labels"))).toEqual({ property: "p-labels", op: "contains", value: "o-short" });
    expect(defaultCondition(col("p-due"))).toEqual({ property: "p-due", op: "before", value: "" });
    expect(defaultCondition(col("chats"))).toEqual({ property: "chats", op: "isEmpty" });
  });

  it("retargets a value when the op changes", () => {
    const is = { property: "p-status", op: "is", value: "o-done" };
    expect(retarget(is, col("p-status"), "isAnyOf")).toEqual({ property: "p-status", op: "isAnyOf", value: ["o-done"] });
    expect(retarget({ property: "p-status", op: "isAnyOf", value: ["o-done", "o-todo"] }, col("p-status"), "isNot")).toEqual({ property: "p-status", op: "isNot", value: "o-done" });
    expect(retarget(is, col("p-status"), "isEmpty")).toEqual({ property: "p-status", op: "isEmpty" });
    expect(retarget(is, col("p-status"), "inGroup")).toEqual({ property: "p-status", op: "inGroup", value: "todo" });
    expect(retarget({ property: "p-due", op: "before", value: "2026-11-12" }, col("p-due"), "after")).toEqual({ property: "p-due", op: "after", value: "2026-11-12" });
    expect(retarget({ property: "p-due", op: "before", value: "2026-11-12" }, col("p-due"), "within")).toEqual({ property: "p-due", op: "within", value: "pastWeek" });
    expect(retarget({ property: "p-due", op: "isEmpty" }, col("p-due"), "after")).toEqual({ property: "p-due", op: "after", value: "" });
  });

  it("edits the filter tree by path, one group deep, at most 20 conditions", () => {
    let tree: Group = readFilter(null);
    expect(writeFilter(tree)).toBeNull();
    tree = addCondition(tree, [], { property: "p-rating", op: "gte", value: 4 });
    tree = addGroup(tree, { property: "p-status", op: "is", value: "o-todo" });
    tree = addCondition(tree, [1], { property: "p-owned", op: "is", value: true });
    expect(writeFilter(tree)).toEqual({ and: [{ property: "p-rating", op: "gte", value: 4 }, { or: [{ property: "p-status", op: "is", value: "o-todo" }, { property: "p-owned", op: "is", value: true }] }] });
    tree = setJoin(tree, [1], "and");
    tree = setJoin(tree, [], "or");
    tree = setCondition(tree, [1, 0], { property: "p-status", op: "isNot", value: "o-done" });
    expect(writeFilter(tree)).toEqual({ or: [{ property: "p-rating", op: "gte", value: 4 }, { and: [{ property: "p-status", op: "isNot", value: "o-done" }, { property: "p-owned", op: "is", value: true }] }] });
    // A group that empties goes.
    tree = removeAt(removeAt(tree, [1, 1]), [1, 0]);
    expect(writeFilter(tree)).toEqual({ or: [{ property: "p-rating", op: "gte", value: 4 }] });
    // Reading back the planner's JSON, a bare condition included.
    expect(readFilter({ property: "kind", op: "is", value: "task" })).toEqual({ join: "and", items: [{ property: "kind", op: "is", value: "task" }] });
    expect(writeFilter(readFilter({ and: [{ or: [{ property: "chats", op: "isEmpty" }] }] }))).toEqual({ and: [{ or: [{ property: "chats", op: "isEmpty" }] }] });
    // At most 20 conditions.
    for (let index = 0; index < 30; index += 1) tree = addCondition(tree, [], { property: "p-rating", op: "eq", value: index });
    expect(countConditions(tree)).toBe(20);
    expect(canAdd(tree)).toBe(false);
    expect(addGroup(tree, { property: "chats", op: "isEmpty" })).toBe(tree);
  });

  it("leaves an unfinished number out of what is sent", () => {
    const tree: Group = { join: "and", items: [{ property: "p-rating", op: "gte", value: Number.NaN }, { property: "p-note", op: "contains", value: "x" }, { join: "or", items: [{ property: "p-rating", op: "eq" }] }] };
    expect(writeFilter(completeOnly(tree, columns))).toEqual({ and: [{ property: "p-note", op: "contains", value: "x" }] });
  });

  it("lists groupable and sortable columns", () => {
    const ids = (list: FieldColumn[]) => list.map(column => column.id);
    expect(ids(groupableColumns(DATABASE))).toEqual(["p-status", "p-labels", "p-due", "p-owned", "kind", "state", "project"]);
    expect(ids(sortableColumns(DATABASE))).toEqual(["p-rating", "p-status", "p-due", "p-owned", "p-note", "title", "kind", "state", "project", "created", "updated", "createdBy"]);
  });
});

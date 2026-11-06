import { describe, expect, it } from "vitest";
import type { NoteDatabase, NoteDatabaseView, NoteProperty } from "../../../types";
import { cellValue, columnsOf, defaultWidth, formatValue, nextWidth, retypeNote } from "./formatValue";

const genre: NoteProperty = { id: "p-genre", name: "Genre", type: "select", order: 2, options: [{ id: "o-fic", name: "Fiction" }, { id: "o-tech", name: "Tech" }] };
const labels: NoteProperty = { id: "p-labels", name: "Labels", type: "multiSelect", order: 3, options: [{ id: "o-short", name: "Short" }, { id: "o-gift", name: "Gift" }] };
const rating: NoteProperty = { id: "p-rating", name: "Rating", type: "number", order: 0 };
const due: NoteProperty = { id: "p-due", name: "Due", type: "date", order: 1 };
const col = (property: NoteProperty) => ({ type: property.type, property });

describe("formatValue", () => {
  it("formats every property type and an empty cell", () => {
    expect(formatValue({ type: "text" }, "big map")).toBe("big map");
    expect(formatValue({ type: "url" }, "https://example.com")).toBe("https://example.com");
    expect(formatValue(col(rating), 2.5)).toBe("2.5");
    expect(formatValue(col(rating), 1000)).toBe("1,000");
    expect(formatValue({ type: "checkbox" }, true)).toBe("Yes");
    expect(formatValue({ type: "checkbox" }, undefined)).toBe("No");
    expect(formatValue(col(genre), "o-tech")).toBe("Tech");
    expect(formatValue({ type: "status", property: { ...genre, type: "status" } }, "o-fic")).toBe("Fiction");
    expect(formatValue(col(labels), ["o-gift", "o-short"])).toBe("Gift, Short");
    expect(formatValue(col(due), { start: "2026-11-12" })).toBe("Nov 12, 2026");
    expect(formatValue({ type: "relation" }, ["r1", "r2"], { rowTitle: id => (id === "r1" ? "DDIA" : null) })).toBe("DDIA, Untitled");
    expect(formatValue({ type: "project" }, "w1", { workspaceTitle: () => "Bridge" })).toBe("Bridge");
    expect(formatValue({ type: "chats" }, ["s1", "s2"])).toBe("2 chats");
    for (const type of ["text", "url", "number", "select", "status", "multiSelect", "date", "relation", "project", "chats"]) {
      expect(formatValue({ type }, undefined)).toBe("");
      expect(formatValue({ type }, null)).toBe("");
    }
    expect(formatValue(col(labels), [])).toBe("");
    expect(formatValue({ type: "chats" }, [])).toBe("");
  });

  it("formats a datetime in the device zone and a range with an arrow", () => {
    expect(formatValue(col(due), { start: "2026-11-12T23:30:00-08:00" }, { timeZone: "UTC" })).toBe("Nov 13, 2026, 7:30 AM");
    expect(formatValue(col(due), { start: "2026-11-12T23:30:00-08:00" }, { timeZone: "America/Los_Angeles" })).toBe("Nov 12, 2026, 11:30 PM");
    expect(formatValue(col(due), { start: "2026-11-01", end: "2026-11-03" })).toBe("Nov 1, 2026 → Nov 3, 2026");
    expect(formatValue({ type: "created" }, "2026-11-05T10:00:00Z", { timeZone: "UTC" })).toBe("Nov 5, 2026, 10:00 AM");
  });

  it("reads an unknown option or row as empty, never as its id", () => {
    expect(formatValue(col(genre), "o-gone")).toBe("");
    expect(formatValue(col(labels), ["o-gone", "o-gift"])).toBe("Gift");
    expect(formatValue({ type: "relation" }, ["r9"])).toBe("1 page");
    expect(formatValue({ type: "relation" }, ["r1", "r8", "r9"], { rowTitle: id => (id === "r1" ? "DDIA" : undefined) })).toBe("DDIA and 2 more pages");
    expect(formatValue(col(rating), Number.NaN)).toBe("");
    expect(formatValue(col(due), { start: "not a date" })).toBe("");
  });
});

describe("columnsOf", () => {
  const database: NoteDatabase = { id: "db", scope: "children", properties: [genre, rating, labels, due], views: [] };
  const view = (properties: NoteDatabaseView["properties"]): NoteDatabaseView => ({ id: "v", name: "Table", type: "table", order: 0, properties });

  it("orders columns by the view, then schema order, skipping deleted ones", () => {
    expect(columnsOf(database, undefined).map(column => column.id)).toEqual(["title", "p-rating", "p-due", "p-genre", "p-labels"]);
    const columns = columnsOf(database, view([
      { id: "p-labels", visible: true },
      { id: "p-gone", visible: true },
      { id: "chats", visible: true },
      { id: "p-rating", visible: false },
    ]));
    expect(columns.map(column => [column.id, column.visible])).toEqual([["title", true], ["p-labels", true], ["chats", true], ["p-rating", false], ["p-due", true], ["p-genre", true]]);
    expect(columns.find(column => column.id === "chats")).toMatchObject({ name: "Chats", type: "chats", property: undefined });
  });

  it("gives each type its default width and clamps saved widths", () => {
    expect(defaultWidth("title")).toBe(280);
    expect(defaultWidth("checkbox")).toBe(96);
    expect(defaultWidth("number")).toBe(120);
    expect(defaultWidth("chats")).toBe(160);
    const columns = columnsOf(database, view([{ id: "title", visible: true, width: 20 }, { id: "p-genre", visible: true, width: 9000 }, { id: "p-due", visible: true, width: 222 }]));
    expect(columns.slice(0, 4).map(column => column.width)).toEqual([80, 640, 222, 120]);
    expect(nextWidth(200, 37)).toBe(237);
    expect(nextWidth(100, -90)).toBe(80);
  });
});

describe("cellValue", () => {
  it("reads cells by id and system columns from the page", () => {
    const note = { id: "n", title: "DDIA", workspaceId: "w1", sessionIds: ["s1"], createdAt: "c", updatedAt: "u" } as never;
    expect(cellValue({ id: "p-rating", property: rating }, note, { "p-rating": 4 })).toBe(4);
    expect(["title", "project", "chats", "created", "updated"].map(id => cellValue({ id }, note, {}))).toEqual(["DDIA", "w1", ["s1"], "c", "u"]);
  });
});

describe("retypeNote", () => {
  it("names what every retype keeps", () => {
    expect(retypeNote("number", "number")).toBe("Keeps every value.");
    for (const [from, to] of [["number", "text"], ["checkbox", "text"], ["select", "text"], ["status", "text"], ["select", "multiSelect"]]) expect(retypeNote(from, to)).toBe("Keeps every value.");
    expect(retypeNote("text", "number")).toBe("Keeps values that read as numbers and clears the rest.");
    expect(retypeNote("text", "checkbox")).toBe("Keeps true and false and clears the rest.");
    expect(retypeNote("text", "select")).toBe("Keeps every value, adding an option for each one.");
    expect(retypeNote("text", "status")).toBe("Keeps every value, adding an option for each one.");
    for (const [from, to] of [["date", "text"], ["multiSelect", "select"], ["relation", "text"], ["number", "date"]]) expect(retypeNote(from, to)).toBe("Clears every value.");
  });
});

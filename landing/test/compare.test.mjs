import assert from "node:assert/strict";
import { test } from "node:test";
import { edges, products, tables } from "../app/content/compare.ts";

test("every row carries one cell per product", () => {
  for (const table of tables) {
    for (const row of table.rows) {
      assert.equal(row.cells.length, products.length, `${table.id}: ${row.label}`);
      for (const cell of row.cells) assert.ok(cell.trim().length > 0, `${table.id}: ${row.label}`);
    }
  }
});

test("Bridge is the first column, so the highlight lands on us", () => {
  assert.equal(products[0].id, "bridge");
});

test("no row claims a capability Bridge does not ship", () => {
  const notOurs = /\b(ios|android|mobile app|windows|cloud workspace|sso|scim|soc 2)\b/i;
  for (const table of tables) {
    for (const row of table.rows) {
      assert.ok(!notOurs.test(row.label), `${table.id}: ${row.label}`);
      assert.ok(!notOurs.test(row.cells[0]), `${table.id}: ${row.label}`);
    }
  }
});

test("copy keeps to the house style: no em dashes", () => {
  const text = [
    ...tables.flatMap((table) => [table.title, table.text, ...table.rows.flatMap((row) => [row.label, row.hint ?? "", ...row.cells])]),
    ...edges.flatMap((edge) => [edge.title, edge.body]),
  ].join(" ");
  assert.ok(!text.includes("—"), "found an em dash");
});

test("row labels are unique inside a table", () => {
  for (const table of tables) {
    const labels = table.rows.map((row) => row.label);
    assert.equal(new Set(labels).size, labels.length, table.id);
  }
});

// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { mockCreateDatabase, mockCreatePage, mockReadPage, mockUpdateNote, mockUpdateSchema, resetMockNotes } from "../../../notesMock";
import { RowProperties } from "./RowProperties";

let host: HTMLDivElement;
let root: Root;
beforeEach(() => {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  resetMockNotes();
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

const settle = () => act(async () => { for (let i = 0; i < 6; i += 1) await new Promise(resolve => setTimeout(resolve, 5)); });
function type(element: HTMLInputElement, text: string) {
  const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!;
  act(() => { setter.call(element, text); element.dispatchEvent(new Event("input", { bubbles: true })); });
}

describe("RowProperties", () => {
  it("creates an option against the stored schema, after someone else changed it", async () => {
    const made = mockCreateDatabase({ title: "Books", properties: [{ name: "Genre", type: "select", options: [{ name: "A" }] }] });
    const genre = made.database!.properties[0];
    const mine = mockCreatePage({ title: "Mine", parentId: made.note.id });
    const other = mockCreatePage({ title: "Other", parentId: made.note.id });
    await act(async () => { root.render(<RowProperties page={mockReadPage(mine.id)} workspaces={[]} onChanged={vi.fn()} onError={vi.fn()} />); });
    await settle();
    // Someone adds B and uses it on another row; this row's own seq never moves.
    const stored = mockUpdateSchema({ id: made.note.id, changes: [{ op: "setOptions", propertyId: genre.id, options: [{ id: genre.options![0].id, name: "A" }, { name: "B" }] }] });
    const b = stored.properties[0].options!.find(option => option.name === "B")!.id;
    mockUpdateNote({ id: other.id, values: { [genre.id]: b } });
    await settle();
    act(() => (host.querySelector("button[aria-label=Genre]") as HTMLButtonElement).click());
    // The panel follows the schema: B is offered without reopening the row.
    expect([...document.querySelectorAll("[role=option]")].map(option => option.textContent)).toEqual(["A", "B"]);
    type(document.querySelector("input[aria-label='Search Genre options']") as HTMLInputElement, "C");
    await act(async () => { [...document.querySelectorAll<HTMLButtonElement>("button")].find(button => button.textContent === "Create C")!.click(); });
    await settle();
    expect(mockReadPage(made.note.id).database!.properties[0].options!.map(option => option.name)).toEqual(["A", "B", "C"]);
    expect(mockReadPage(other.id).values).toEqual({ [genre.id]: b });
    const c = mockReadPage(made.note.id).database!.properties[0].options![2].id;
    expect(mockReadPage(mine.id).values).toEqual({ [genre.id]: c });
  });
});

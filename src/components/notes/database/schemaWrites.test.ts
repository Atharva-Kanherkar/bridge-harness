// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { bridgeApi } from "../../../api";
import { mockCreateDatabase, mockCreatePage, mockReadPage, mockUpdateNote, resetMockNotes } from "../../../notesMock";
import { addOption, reconcileOptions, saveOptions } from "./schemaWrites";

beforeEach(() => resetMockNotes());
afterEach(() => vi.restoreAllMocks());

describe("reconcileOptions", () => {
  it("keeps options added since the editor opened and drops ones deleted since", () => {
    const stored = [{ id: "a", name: "A" }, { id: "c", name: "C" }, { id: "n", name: "New elsewhere", group: "todo" }];
    const edited = [{ id: "c", name: "C renamed" }, { id: "b", name: "B" }, { name: "Fresh" }];
    expect(reconcileOptions(["a", "b", "c"], edited, stored)).toEqual([
      { id: "c", name: "C renamed" },
      { name: "Fresh" },
      { id: "n", name: "New elsewhere", color: null, group: "todo" },
    ]);
  });

  it("deletes what the editor saw and removed", () => {
    expect(reconcileOptions(["a", "b"], [{ id: "a", name: "A" }], [{ id: "a", name: "A" }, { id: "b", name: "B" }])).toEqual([{ id: "a", name: "A" }]);
  });
});

describe("option writes", () => {
  /** Hold the second schema write until released, as a slow request would. */
  function holdSecondWrite() {
    const real = bridgeApi.updateDatabaseSchema.bind(bridgeApi);
    let release!: () => void;
    const gate = new Promise<void>(resolve => { release = resolve; });
    let calls = 0;
    vi.spyOn(bridgeApi, "updateDatabaseSchema").mockImplementation(async params => {
      if (++calls === 2) await gate;
      return real(params);
    });
    return () => release();
  }

  it("never loses an option to a concurrent addOption", async () => {
    const made = mockCreateDatabase({ title: "Books", properties: [{ name: "Genre", type: "select", options: [{ name: "A" }] }] });
    const genre = made.database!.properties[0];
    const row = mockCreatePage({ title: "Book", parentId: made.note.id });
    const release = holdSecondWrite();
    const left = addOption(made.note.id, genre.id, { name: "B" });
    const right = addOption(made.note.id, genre.id, { name: "C" });
    const b = (await left).properties[0].options!.find(option => option.name === "B")!.id;
    mockUpdateNote({ id: row.id, values: { [genre.id]: b } });
    release();
    await right;
    expect(mockReadPage(made.note.id).database!.properties[0].options!.map(option => option.name)).toEqual(["A", "B", "C"]);
    expect(mockReadPage(row.id).values).toEqual({ [genre.id]: b });
  });

  it("never loses an option to a saveOptions built before it", async () => {
    const made = mockCreateDatabase({ title: "Books", properties: [{ name: "Genre", type: "select", options: [{ name: "A" }] }] });
    const genre = made.database!.properties[0];
    const a = genre.options![0].id;
    const release = holdSecondWrite();
    const added = addOption(made.note.id, genre.id, { name: "B" });
    const renamed = saveOptions(made.note.id, genre.id, [{ id: a, name: "A renamed" }], [a]);
    await added;
    release();
    await renamed;
    expect(mockReadPage(made.note.id).database!.properties[0].options!.map(option => option.name)).toEqual(["A renamed", "B"]);
  });

  it("retries a conflict however the host words it, and gives up after three", async () => {
    const made = mockCreateDatabase({ title: "Books", properties: [{ name: "Genre", type: "select", options: [{ name: "A" }] }] });
    const genre = made.database!.properties[0];
    const real = bridgeApi.updateDatabaseSchema.bind(bridgeApi);
    const write = vi.spyOn(bridgeApi, "updateDatabaseSchema");
    write.mockRejectedValueOnce({ code: "invalid_params", message: "the options of Genre changed since they were read; read them again" });
    write.mockRejectedValueOnce("the options of Genre changed since they were read; read them again");
    write.mockImplementation(params => real(params));
    await addOption(made.note.id, genre.id, { name: "B" });
    expect(write).toHaveBeenCalledTimes(3);
    write.mockReset();
    write.mockRejectedValue(new Error("the options of Genre changed since they were read; read them again"));
    await expect(addOption(made.note.id, genre.id, { name: "C" })).rejects.toThrow("changed since they were read");
    expect(write).toHaveBeenCalledTimes(3);
  });
});


import { describe, expect, it } from "vitest";
import { reconcileOptions } from "./schemaWrites";

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

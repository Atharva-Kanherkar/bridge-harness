import { expect, it } from "vitest";
import { layoutExample, parseLayout } from "./layout";

it("round trips ordered two-line layouts including literal spaces and separators", () => {
  const layout = [["icon", "space", "used"], ["provider", "dot", "weeklyRemaining"]] as const;
  expect(parseLayout(JSON.stringify(layout))).toEqual(layout);
  expect(layoutExample(parseLayout(JSON.stringify(layout)))).toBe("▥ 42%\nCodex · 7d 26%");
});
it("rejects executable, unknown, and oversized layouts", () => {
  for (const value of [null, {}, [["runScript"]], [["icon"], ["icon"]], [[], [], []], [Array(13).fill("space")], ["provider"]]) {
    expect(() => parseLayout(JSON.stringify(value))).toThrow();
  }
  expect(parseLayout("[]")).toEqual([]);
});

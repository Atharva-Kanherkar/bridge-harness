import { describe, expect, it } from "vitest";
import { welcomeHeadline } from "./greetings";

describe("welcomeHeadline", () => {
  it("asks one plain question when no project is known", () => {
    for (const project of [undefined, null, "", "   "]) {
      const greeting = welcomeHeadline(project);
      expect(greeting.headline).toBe("What should we build?");
      expect(greeting.parts.every(part => part.kind === "text")).toBe(true);
    }
  });

  it("names the project as its own part", () => {
    const greeting = welcomeHeadline(" harness ");
    expect(greeting.headline).toBe("What should we build in harness?");
    expect(greeting.parts.map(part => part.text).join("")).toBe(greeting.headline);
    expect(greeting.parts.filter(part => part.kind === "project")).toEqual([{ text: "harness", kind: "project" }]);
  });

  it("is the same line every time, not a random pick", () => {
    expect(new Set(Array.from({ length: 20 }, () => welcomeHeadline("Bridge").headline)).size).toBe(1);
  });
});

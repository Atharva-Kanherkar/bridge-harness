import { describe, expect, it } from "vitest";
import { readHideAiAttribution } from "./attributionSettings";

describe("readHideAiAttribution", () => {
  it("defaults off for missing, null, or undefined flags", () => {
    expect(readHideAiAttribution(null)).toBe(false);
    expect(readHideAiAttribution(undefined)).toBe(false);
    expect(readHideAiAttribution({})).toBe(false);
    expect(readHideAiAttribution({ hideAiAttribution: null })).toBe(false);
    expect(readHideAiAttribution({ hideAiAttribution: false })).toBe(false);
  });

  it("is on only when the stored flag is true", () => {
    expect(readHideAiAttribution({ hideAiAttribution: true })).toBe(true);
  });
});

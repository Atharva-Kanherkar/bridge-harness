import { describe, expect, it } from "vitest";
import {
  INITIAL_TRANSCRIPT_ROWS,
  TRANSCRIPT_HYDRATE_CHUNK,
  initialWindowStart,
  nextWindowStart,
} from "./transcriptWindow";

describe("transcript window", () => {
  it("keeps only the newest rows in the opening window", () => {
    expect(INITIAL_TRANSCRIPT_ROWS).toBe(32);
    expect(initialWindowStart(1500)).toBe(1500 - INITIAL_TRANSCRIPT_ROWS);
  });

  it("has nothing to hydrate for a short chat", () => {
    expect(initialWindowStart(INITIAL_TRANSCRIPT_ROWS)).toBe(0);
    expect(initialWindowStart(10)).toBe(0);
    expect(initialWindowStart(0)).toBe(0);
  });

  it("mounts everything when the window is forced", () => {
    expect(initialWindowStart(1500, true)).toBe(0);
    expect(initialWindowStart(0, true)).toBe(0);
  });

  it("moves the start one chunk toward the head per slice", () => {
    expect(TRANSCRIPT_HYDRATE_CHUNK).toBe(48);
    expect(nextWindowStart(1468)).toBe(1468 - TRANSCRIPT_HYDRATE_CHUNK);
    expect(nextWindowStart(20)).toBe(0);
    expect(nextWindowStart(0)).toBe(0);
  });

  it("keeps the window start stable when tail rows arrive", () => {
    const start = initialWindowStart(100);
    expect(start).toBe(68);
    const first = Array.from({ length: 100 }, (_, index) => `row-${index}`);
    const grown = [...first, "streamed-1", "streamed-2"];
    // The same leading rows stay unmounted; the new rows join the mounted tail.
    expect(grown.slice(0, start)).toEqual(first.slice(0, start));
    expect(grown.slice(start)).toHaveLength(34);
  });
});

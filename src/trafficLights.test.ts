import { describe, expect, it } from "vitest";
import { REVEAL_HEIGHT, REVEAL_WIDTH, inTrafficLightCorner, watchTrafficLights } from "./trafficLights";

describe("inTrafficLightCorner", () => {
  it("covers the buttons and a little slack around them", () => {
    // trafficLightPosition is (18,15) and the cluster is ~60px wide.
    expect(inTrafficLightCorner(18, 15)).toBe(true);
    expect(inTrafficLightCorner(78, 22)).toBe(true);
    expect(inTrafficLightCorner(0, 0)).toBe(true);
    expect(inTrafficLightCorner(REVEAL_WIDTH, REVEAL_HEIGHT)).toBe(true);
  });

  it("stops short of the rail's own controls", () => {
    // The wordmark and search sit past the corner; hovering them must not light
    // the buttons up.
    expect(inTrafficLightCorner(REVEAL_WIDTH + 1, 20)).toBe(false);
    expect(inTrafficLightCorner(40, REVEAL_HEIGHT + 1)).toBe(false);
    expect(inTrafficLightCorner(240, 300)).toBe(false);
  });
});

describe("watchTrafficLights", () => {
  it("is inert outside the desktop shell", () => {
    // In a browser there is no window chrome to hide, and no invoke to call.
    expect(() => watchTrafficLights()()).not.toThrow();
  });
});

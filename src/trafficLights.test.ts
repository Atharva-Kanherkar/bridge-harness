// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { REVEAL_HEIGHT, REVEAL_WIDTH, inTrafficLightCorner, watchTrafficLights } from "./trafficLights";

const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));

const move = (x: number, y: number) => {
  const event = new Event("pointermove") as Event & { clientX: number; clientY: number };
  Object.assign(event, { clientX: x, clientY: y });
  window.dispatchEvent(event);
};

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
    expect(invoke).not.toHaveBeenCalled();
  });
});

describe("watchTrafficLights inside the shell", () => {
  let stop: () => void;

  beforeEach(() => {
    invoke.mockReset();
    invoke.mockResolvedValue(undefined);
    (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
    stop = watchTrafficLights();
  });

  afterEach(() => {
    stop();
    delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__;
  });

  it("sends only the transitions, not every move", () => {
    move(10, 10);
    move(20, 20);
    move(30, 12);
    expect(invoke).toHaveBeenCalledTimes(1);
    expect(invoke).toHaveBeenLastCalledWith("set_traffic_lights_visible", { visible: true });

    move(400, 300);
    move(500, 400);
    expect(invoke).toHaveBeenCalledTimes(2);
    expect(invoke).toHaveBeenLastCalledWith("set_traffic_lights_visible", { visible: false });
  });

  it("hides them again when the window loses focus", () => {
    move(10, 10);
    window.dispatchEvent(new Event("blur"));
    expect(invoke).toHaveBeenLastCalledWith("set_traffic_lights_visible", { visible: false });
  });

  it("gives up for good once the command is missing, rather than retrying per move", async () => {
    invoke.mockRejectedValue(new Error("unknown command"));
    move(10, 10);
    await Promise.resolve();
    await Promise.resolve();

    move(400, 400);
    move(12, 12);
    move(600, 600);
    // One attempt, then silence: a build without the command will never grow it.
    expect(invoke).toHaveBeenCalledTimes(1);
  });

  it("stops listening once torn down", () => {
    stop();
    move(10, 10);
    expect(invoke).not.toHaveBeenCalled();
  });
});

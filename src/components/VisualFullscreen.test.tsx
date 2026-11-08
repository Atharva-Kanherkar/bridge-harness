// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { nextZoom, ZOOM_DEFAULT, ZOOM_MAX, ZOOM_MIN, VisualFullscreen } from "./VisualFullscreen";

describe("nextZoom", () => {
  it("steps the ladder by a quarter of the baseline", () => {
    expect(nextZoom(ZOOM_DEFAULT, "in")).toBe(125);
    expect(nextZoom(ZOOM_DEFAULT, "out")).toBe(75);
  });

  it("clamps at both ends so the controls never dead-end past readable", () => {
    expect(nextZoom(ZOOM_MAX, "in")).toBe(ZOOM_MAX);
    expect(nextZoom(ZOOM_MIN, "out")).toBe(ZOOM_MIN);
  });
});

describe("VisualFullscreen", () => {
  let container: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    container = document.createElement("div");
    document.body.appendChild(container);
    root = createRoot(container);
  });

  afterEach(() => {
    act(() => root.unmount());
    container.remove();
  });

  it("renders nothing at all while closed", () => {
    act(() => {
      root.render(
        <VisualFullscreen open={false} onClose={() => {}} label="diagram">
          <svg data-testid="visual" />
        </VisualFullscreen>,
      );
    });
    expect(container.innerHTML).toBe("");
    expect(document.body.querySelector('[role="dialog"]')).toBeNull();
  });

  it("portals an overlay labelled for its kind, and closes through the button", () => {
    act(() => {
      root.render(
        <VisualFullscreen open onClose={() => {}} label="diagram">
          <svg data-testid="visual" />
        </VisualFullscreen>,
      );
    });
    // Portal: outside the message container, so a transformed ancestor cannot
    // become the containing block for its `position: fixed`.
    const overlay = document.body.querySelector('[role="dialog"][aria-label="diagram fullscreen"]') as HTMLElement;
    expect(overlay).toBeTruthy();
    expect(container.contains(overlay)).toBe(false);
    expect(overlay.querySelector('[data-testid="visual"]')).toBeTruthy();
    expect(overlay.textContent).toContain("100%");
    expect(overlay.querySelector('[aria-label="Exit fullscreen"]')).toBeTruthy();
  });

  it("zooms the canvas and reports the level", () => {
    act(() => {
      root.render(
        <VisualFullscreen open onClose={() => {}} label="diagram">
          <svg data-testid="visual" />
        </VisualFullscreen>,
      );
    });
    const zoomIn = document.body.querySelector('[aria-label="Zoom in"]') as HTMLButtonElement;
    act(() => { zoomIn.click(); });
    expect(document.body.querySelector('[role="dialog"]')!.textContent).toContain("125%");
    act(() => { (document.body.querySelector('[aria-label="Zoom out"]') as HTMLButtonElement).click(); });
    expect(document.body.querySelector('[role="dialog"]')!.textContent).toContain("100%");
  });

  it("calls onClose when dismissed", () => {
    let closed = 0;
    act(() => {
      root.render(
        <VisualFullscreen open onClose={() => { closed += 1; }} label="mermaid">
          <svg />
        </VisualFullscreen>,
      );
    });
    act(() => { (document.body.querySelector('[aria-label="Exit fullscreen"]') as HTMLButtonElement).click(); });
    expect(closed).toBe(1);
  });
});

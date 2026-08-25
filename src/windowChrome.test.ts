// @vitest-environment jsdom
import { describe, expect, it } from "vitest";
import { FLUSH_WINDOW_EVENT, isFlushWindowDocument, setLayoutFullscreenDocument } from "./windowChrome";

describe("window chrome flags", () => {
  it("toggles the in-app fullscreen token on the document", () => {
    document.documentElement.removeAttribute("data-fullscreen");
    setLayoutFullscreenDocument(true);
    expect(document.documentElement.hasAttribute("data-fullscreen")).toBe(true);
    setLayoutFullscreenDocument(false);
    expect(document.documentElement.hasAttribute("data-fullscreen")).toBe(false);
  });

  it("reads the native flush-window stamp the shell writes", () => {
    document.documentElement.removeAttribute("data-flush-window");
    expect(isFlushWindowDocument()).toBe(false);
    document.documentElement.setAttribute("data-flush-window", "");
    expect(isFlushWindowDocument()).toBe(true);
    document.documentElement.dispatchEvent(new Event(FLUSH_WINDOW_EVENT));
    document.documentElement.removeAttribute("data-flush-window");
  });
});

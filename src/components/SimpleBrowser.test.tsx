// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { describe, expect, it } from "vitest";
import { SimpleBrowser } from "./SimpleBrowser";

const nativeInputValueSetter = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, "value")!.set!;

function submit(container: HTMLDivElement, value: string) {
  const input = container.querySelector<HTMLInputElement>('input[aria-label="Address"]')!;
  const form = container.querySelector("form")!;
  nativeInputValueSetter.call(input, value);
  input.dispatchEvent(new Event("input", { bubbles: true }));
  form.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
}

describe("SimpleBrowser", () => {
  it("defaults bare loopback hosts to http, not https", async () => {
    (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
    const container = document.createElement("div");
    const root = createRoot(container);
    await act(async () => root.render(<SimpleBrowser />));

    await act(async () => submit(container, "localhost:1420"));
    expect(container.querySelector("iframe")?.getAttribute("src")).toBe("http://localhost:1420");

    await act(async () => submit(container, "127.0.0.1:3000"));
    expect(container.querySelector("iframe")?.getAttribute("src")).toBe("http://127.0.0.1:3000");

    await act(async () => root.unmount());
  });

  it("still defaults a non-loopback bare host to https", async () => {
    (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
    const container = document.createElement("div");
    const root = createRoot(container);
    await act(async () => root.render(<SimpleBrowser />));

    await act(async () => submit(container, "example.com"));
    expect(container.querySelector("iframe")?.getAttribute("src")).toBe("https://example.com");

    await act(async () => root.unmount());
  });

  it("does not advance the history index when re-submitting the current URL", async () => {
    (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
    const container = document.createElement("div");
    const root = createRoot(container);
    await act(async () => root.render(<SimpleBrowser />));

    await act(async () => submit(container, "example.com"));
    await act(async () => submit(container, "example.org"));
    const back = container.querySelector<HTMLButtonElement>('button[aria-label="Back"]')!;
    expect(back.disabled).toBe(false);

    // Re-submitting the URL that's already current must dedupe the history
    // entry and must NOT bump the index -- otherwise the index walks past
    // the end of the history array and Back navigates to `undefined`.
    await act(async () => submit(container, "example.org"));
    await act(async () => submit(container, "example.org"));
    await act(async () => submit(container, "example.org"));

    await act(async () => back.click());
    expect(container.querySelector("iframe")?.getAttribute("src")).toBe("https://example.com");
    expect(container.textContent).not.toContain("No page open");

    await act(async () => root.unmount());
  });
});

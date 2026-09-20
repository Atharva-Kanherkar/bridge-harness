// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import { installExternalLinkHandler, isExternalUrl, openExternalUrl, openInSystemBrowser, setInternalLinkRouter } from "./externalLinks";

const open = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/plugin-shell", () => ({ open }));

afterEach(() => {
  open.mockClear();
  setInternalLinkRouter(undefined);
  document.body.innerHTML = "";
});

/** One anchor in the document, clicked the way a reader would. Returns whether
 * the click was prevented — i.e. whether the webview was stopped from
 * navigating itself. */
function clickAnchor(href: string, attributes: Record<string, string> = {}): boolean {
  const anchor = document.createElement("a");
  anchor.href = href;
  for (const [name, value] of Object.entries(attributes)) anchor.setAttribute(name, value);
  anchor.textContent = "link";
  document.body.appendChild(anchor);
  return !anchor.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true, button: 0 }));
}

describe("isExternalUrl", () => {
  it("accepts http, https, and mailto", () => {
    expect(isExternalUrl("https://example.com")).toBe(true);
    expect(isExternalUrl("http://example.com")).toBe(true);
    expect(isExternalUrl("mailto:a@example.com")).toBe(true);
  });

  it("rejects everything else, including schemes a webview could otherwise hand off to another app", () => {
    expect(isExternalUrl("javascript:alert(1)")).toBe(false);
    expect(isExternalUrl("file:///etc/passwd")).toBe(false);
    expect(isExternalUrl("#anchor")).toBe(false);
    expect(isExternalUrl("/local/path")).toBe(false);
  });
});

describe("openExternalUrl", () => {
  it("hands external URLs to the shell plugin", async () => {
    await openExternalUrl("https://example.com/docs");
    expect(open).toHaveBeenCalledWith("https://example.com/docs");
  });

  it("silently ignores non-external URLs", async () => {
    await openExternalUrl("javascript:alert(1)");
    expect(open).not.toHaveBeenCalled();
  });
});

describe("installExternalLinkHandler", () => {
  it("intercepts a left-click on an external anchor and routes it through the shell plugin", () => {
    installExternalLinkHandler();
    const anchor = document.createElement("a");
    anchor.href = "https://example.com";
    anchor.textContent = "link";
    document.body.appendChild(anchor);

    const event = new MouseEvent("click", { bubbles: true, cancelable: true, button: 0 });
    const prevented = !anchor.dispatchEvent(event);

    expect(prevented).toBe(true);
    expect(open).toHaveBeenCalledWith("https://example.com");
  });

  it("leaves non-external anchors alone", () => {
    installExternalLinkHandler();
    const anchor = document.createElement("a");
    anchor.href = "#section";
    document.body.appendChild(anchor);

    const event = new MouseEvent("click", { bubbles: true, cancelable: true, button: 0 });
    const prevented = !anchor.dispatchEvent(event);

    expect(prevented).toBe(false);
    expect(open).not.toHaveBeenCalled();
  });
});

describe("the internal link router", () => {
  it("keeps a URL it takes inside the app", async () => {
    const router = vi.fn().mockReturnValue(true);
    setInternalLinkRouter(router);

    await openExternalUrl("https://github.com/bridge/harness/pull/12");

    expect(router).toHaveBeenCalledWith("https://github.com/bridge/harness/pull/12");
    expect(open).not.toHaveBeenCalled();
  });

  it("falls through to the browser when the router declines", async () => {
    setInternalLinkRouter(vi.fn().mockReturnValue(false));

    await openExternalUrl("https://example.com/docs");

    expect(open).toHaveBeenCalledWith("https://example.com/docs");
  });

  it("waits for a router that answers asynchronously", async () => {
    setInternalLinkRouter(() => new Promise<boolean>(resolve => setTimeout(() => resolve(true), 0)));

    await openExternalUrl("https://github.com/bridge/harness/pull/12");

    expect(open).not.toHaveBeenCalled();
  });

  it("never strands a link on a router that throws or rejects", async () => {
    setInternalLinkRouter(() => { throw new Error("no repository"); });
    await openExternalUrl("https://example.com/one");
    expect(open).toHaveBeenCalledWith("https://example.com/one");

    setInternalLinkRouter(() => Promise.reject(new Error("gh is gone")));
    await openExternalUrl("https://example.com/two");
    expect(open).toHaveBeenCalledWith("https://example.com/two");
  });

  it("never offers the router a URL it would not have opened anyway", async () => {
    const router = vi.fn().mockReturnValue(true);
    setInternalLinkRouter(router);

    await openExternalUrl("javascript:alert(1)");

    expect(router).not.toHaveBeenCalled();
    expect(open).not.toHaveBeenCalled();
  });

  it("goes back to the browser once the router is cleared", async () => {
    setInternalLinkRouter(vi.fn().mockReturnValue(true));
    setInternalLinkRouter(undefined);

    await openExternalUrl("https://example.com/docs");

    expect(open).toHaveBeenCalledWith("https://example.com/docs");
  });

  it("is bypassed by an explicit system-browser open", async () => {
    setInternalLinkRouter(vi.fn().mockReturnValue(true));

    await openInSystemBrowser("https://github.com/bridge/harness/pull/12");

    expect(open).toHaveBeenCalledWith("https://github.com/bridge/harness/pull/12");
  });

  it("offers a clicked anchor to the router before the browser", async () => {
    installExternalLinkHandler();
    const router = vi.fn().mockReturnValue(true);
    setInternalLinkRouter(router);

    const prevented = clickAnchor("https://github.com/bridge/harness/pull/12");
    await Promise.resolve();

    expect(prevented).toBe(true);
    expect(router).toHaveBeenCalledWith("https://github.com/bridge/harness/pull/12");
    expect(open).not.toHaveBeenCalled();
  });

  it("leaves an anchor marked as a deliberate escape to the browser", async () => {
    installExternalLinkHandler();
    const router = vi.fn().mockReturnValue(true);
    setInternalLinkRouter(router);

    // A check's log URL is often shaped like a page the pane can render; the
    // affordance says it means to leave, so it leaves.
    const prevented = clickAnchor("https://github.com/bridge/harness/pull/12/checks", { "data-system-browser": "" });
    await Promise.resolve();

    expect(prevented).toBe(true);
    expect(router).not.toHaveBeenCalled();
    expect(open).toHaveBeenCalledWith("https://github.com/bridge/harness/pull/12/checks");
  });

  it("still declines the clicks it always declined", async () => {
    installExternalLinkHandler();
    const router = vi.fn().mockReturnValue(true);
    setInternalLinkRouter(router);

    const anchor = document.createElement("a");
    anchor.href = "https://github.com/bridge/harness/pull/12";
    document.body.appendChild(anchor);
    // Registered after the interceptor, so it runs after it: the click still
    // reaches the guard, jsdom just does not try to navigate afterwards.
    const swallow = (event: Event) => event.preventDefault();
    document.addEventListener("click", swallow);
    // A middle click, which the reader means as "open somewhere else".
    anchor.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true, button: 1 }));
    // A click something nearer the target already handled.
    anchor.addEventListener("click", event => event.preventDefault());
    anchor.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true, button: 0 }));
    // And an in-page anchor, which was never the browser's to open.
    clickAnchor("#section");
    await Promise.resolve();
    document.removeEventListener("click", swallow);

    expect(router).not.toHaveBeenCalled();
    expect(open).not.toHaveBeenCalled();
  });
});

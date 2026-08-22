// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import { installExternalLinkHandler, isExternalUrl, openExternalUrl } from "./externalLinks";

const open = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/plugin-shell", () => ({ open }));

afterEach(() => {
  open.mockClear();
  document.body.innerHTML = "";
});

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

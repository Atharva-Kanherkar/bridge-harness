// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { notifyInBrowser, resetWebNotify, WEB_NOTIFICATION_OPEN, webCanPostBanners } from "./webNotify";

type Posted = { title: string; options: NotificationOptions; onclick: (() => void) | null; close: ReturnType<typeof vi.fn> };

function stubNotification(permission: NotificationPermission, answer: NotificationPermission = "granted") {
  const posted: Posted[] = [];
  const requestPermission = vi.fn(async () => { stub.permission = answer; return answer; });
  const stub = Object.assign(function Notification(this: Posted, title: string, options: NotificationOptions) {
    this.title = title;
    this.options = options;
    this.onclick = null;
    this.close = vi.fn();
    posted.push(this);
  }, { permission, requestPermission });
  vi.stubGlobal("Notification", stub);
  return { posted, requestPermission, stub };
}

describe("browser notifications", () => {
  beforeEach(() => resetWebNotify());
  afterEach(() => vi.unstubAllGlobals());

  it("posts with the chat as the tag when allowed", async () => {
    const { posted, requestPermission } = stubNotification("granted");
    await notifyInBrowser("Bridge needs you", "Fix login · Claude is waiting for your input", "chat-1");
    expect(posted).toHaveLength(1);
    expect(posted[0].title).toBe("Bridge needs you");
    expect(posted[0].options).toEqual({ body: "Fix login · Claude is waiting for your input", tag: "chat-1" });
    expect(requestPermission).not.toHaveBeenCalled();
  });

  it("asks once for two banners, and posts both when allowed", async () => {
    const { posted, requestPermission } = stubNotification("default", "granted");
    await Promise.all([notifyInBrowser("One", "a", "c1"), notifyInBrowser("Two", "b", "c2")]);
    expect(requestPermission).toHaveBeenCalledTimes(1);
    expect(posted.map(item => item.title).sort()).toEqual(["One", "Two"]);
  });

  it("posts nothing and never asks again once denied", async () => {
    const { posted, requestPermission } = stubNotification("default", "denied");
    await notifyInBrowser("One", "a", "c1");
    await notifyInBrowser("Two", "b", "c1");
    expect(requestPermission).toHaveBeenCalledTimes(1);
    expect(posted).toHaveLength(0);
    expect(webCanPostBanners()).toBe(false);
  });

  it("focuses the tab and asks App to open the chat on click", async () => {
    const { posted } = stubNotification("granted");
    const focus = vi.spyOn(window, "focus").mockImplementation(() => undefined);
    const opened: string[] = [];
    const listener = (event: Event) => opened.push((event as CustomEvent<{ sessionId: string }>).detail.sessionId);
    window.addEventListener(WEB_NOTIFICATION_OPEN, listener);
    await notifyInBrowser("Turn completed", "Fix login · Claude finished its turn", "chat-9");
    posted[0].onclick?.();
    expect(focus).toHaveBeenCalled();
    expect(opened).toEqual(["chat-9"]);
    expect(posted[0].close).toHaveBeenCalled();
    window.removeEventListener(WEB_NOTIFICATION_OPEN, listener);
  });

  it("does nothing in a browser without the API", async () => {
    vi.stubGlobal("Notification", undefined);
    expect(webCanPostBanners()).toBe(false);
    await expect(notifyInBrowser("One", "a")).resolves.toBeUndefined();
  });
});

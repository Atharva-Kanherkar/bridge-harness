// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { McpAppFrame, VIEW_CSP, viewDocument } from "./McpAppFrame";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const input = { version: 1, title: "Spend", blocks: [] };

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  // The SDK transport narrates every message at debug level.
  vi.spyOn(console, "debug").mockImplementation(() => undefined);
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.restoreAllMocks();
});

const settle = () => act(async () => { await new Promise(resolve => setTimeout(resolve, 0)); });
async function until(check: () => boolean) {
  for (let attempt = 0; attempt < 100 && !check(); attempt += 1) await settle();
  expect(check()).toBe(true);
}

/**
 * Plays the view's side of MCP Apps. jsdom does not set `event.source` on
 * postMessage, so messages "from the view" are dispatched with the frame's
 * window as their source, and the host's replies are read off that window.
 */
async function mountView(props: Partial<Parameters<typeof McpAppFrame>[0]> = {}) {
  const ask = vi.fn();
  const open = vi.fn();
  const render = (extra: Partial<Parameters<typeof McpAppFrame>[0]> = {}) => act(() => root.render(
    <McpAppFrame input={input} theme="dark" title="Spend" actions={{ ask, open }} {...props} {...extra} />,
  ));
  render();
  const frame = container.querySelector("iframe")!;
  await until(() => !!frame.getAttribute("srcdoc"));
  const view = frame.contentWindow!;
  const received: { method?: string; id?: number; result?: unknown; params?: Record<string, unknown> }[] = [];
  view.addEventListener("message", event => received.push(event.data));
  const send = (message: unknown, source: Window = view) => act(() => {
    window.dispatchEvent(new MessageEvent("message", { data: message, source }));
  });
  let id = 0;
  const request = async (method: string, params: unknown) => {
    id += 1;
    const at = id;
    await send({ jsonrpc: "2.0", id: at, method, params });
    await until(() => received.some(message => message.id === at));
    return received.find(message => message.id === at)!;
  };
  const initialize = async () => {
    const reply = await request("ui/initialize", { appInfo: { name: "test-view", version: "1" }, appCapabilities: {}, protocolVersion: "2026-01-26" });
    await send({ jsonrpc: "2.0", method: "ui/notifications/initialized", params: {} });
    return reply;
  };
  return { frame, view, received, send, request, initialize, ask, open, render };
}

describe("McpAppFrame", () => {
  it("the_frame_is_sandboxed_allow_scripts_only", async () => {
    const { frame } = await mountView();
    expect(frame.getAttribute("sandbox")).toBe("allow-scripts");
    expect(frame.getAttribute("referrerpolicy")).toBe("no-referrer");
  });

  it("the_srcdoc_starts_with_the_csp_meta", async () => {
    const { frame } = await mountView();
    const doc = frame.getAttribute("srcdoc")!;
    const csp = doc.indexOf("Content-Security-Policy");
    expect(csp).toBeGreaterThan(0);
    expect(csp).toBeLessThan(doc.indexOf("<style"));
    expect(csp).toBeLessThan(doc.indexOf("<script"));
    expect(VIEW_CSP).toContain("connect-src 'none'");
    expect(VIEW_CSP).toContain("default-src 'none'");
    expect(viewDocument("<html><head><title>x</title></head></html>")).toMatch(/^<html><head>\n<meta http-equiv="Content-Security-Policy"/);
  });

  it("tool_input_is_sent_after_the_view_initializes", async () => {
    const { received, initialize } = await mountView();
    const reply = await initialize();
    expect((reply.result as { hostInfo: { name: string } }).hostInfo.name).toBe("bridge");
    expect((reply.result as { hostContext: { theme: string } }).hostContext.theme).toBe("dark");
    await until(() => received.some(message => message.method === "ui/notifications/tool-input"));
    const toolInput = received.find(message => message.method === "ui/notifications/tool-input")!;
    expect(toolInput.params?.arguments).toEqual(input);
  });

  it("messages_from_another_window_are_dropped", async () => {
    const { received, send, ask } = await mountView();
    await send({ jsonrpc: "2.0", id: 99, method: "ui/initialize", params: { appInfo: { name: "x", version: "1" }, appCapabilities: {}, protocolVersion: "2026-01-26" } }, window);
    await send({ jsonrpc: "2.0", id: 100, method: "ui/message", params: { role: "user", content: [{ type: "text", text: "spoofed" }] } }, window);
    for (let attempt = 0; attempt < 5; attempt += 1) await settle();
    expect(received.some(message => message.id === 99 || message.id === 100)).toBe(false);
    expect(ask).not.toHaveBeenCalled();
  });

  it("size_changed_resizes_the_frame", async () => {
    const { frame, send, initialize } = await mountView();
    await initialize();
    await send({ jsonrpc: "2.0", method: "ui/notifications/size-changed", params: { width: 700, height: 481.2 } });
    await until(() => frame.style.height === "482px");
  });

  it("a_theme_flip_resends_host_context", async () => {
    const { received, initialize, render } = await mountView();
    await initialize();
    render({ theme: "light" });
    await until(() => received.some(message => message.method === "ui/notifications/host-context-changed"));
    const change = received.find(message => message.method === "ui/notifications/host-context-changed")!;
    expect(change.params?.theme).toBe("light");
  });

  it("a_follow_up_message_fills_the_composer_and_does_not_send", async () => {
    const { request, initialize, ask } = await mountView();
    await initialize();
    const reply = await request("ui/message", { role: "user", content: [{ type: "text", text: "Split Sep 30 by model" }] });
    expect(reply.result).toBeDefined();
    // The host's only move is to hand the text to the composer callback.
    expect(ask).toHaveBeenCalledWith("Split Sep 30 by model");
  });

  it("open_link_goes_through_the_external_open_path", async () => {
    const { request, initialize, open } = await mountView();
    await initialize();
    await request("ui/open-link", { url: "https://www.iea.org/reports/renewables-2024" });
    expect(open).toHaveBeenCalledWith("https://www.iea.org/reports/renewables-2024");
    await request("ui/open-link", { url: "javascript:alert(1)" });
    expect(open).toHaveBeenCalledTimes(1);
  });
});

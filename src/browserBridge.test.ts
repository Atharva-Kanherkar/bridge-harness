import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { runInNewContext } from "node:vm";
import { describe, expect, it, vi } from "vitest";
// @ts-expect-error jsdom is an existing test dependency without bundled declarations.
import { JSDOM } from "jsdom";

const root = resolve(import.meta.dirname, "..");
const read = (path: string) => readFileSync(resolve(root, path), "utf8");

describe("authenticated browser bridge artifacts", () => {
  it("creates the redaction worker before first attach and stops the old capture before switching tabs", async () => {
    const events: string[] = [];
    let offscreenReady = false;
    const listener = { addListener: vi.fn() };
    const chrome = {
      runtime: {
        connectNative: () => ({ postMessage: vi.fn(), onMessage: listener, onDisconnect: listener }),
        getManifest: () => ({ version: "test" }), getURL: (path: string) => path,
        getContexts: async () => offscreenReady ? [{}] : [],
        onMessage: listener,
        sendMessage: async (message: { type: string }) => {
          if (!offscreenReady) throw new Error("Receiving end does not exist");
          events.push(message.type);
          return { ok: true };
        },
      },
      offscreen: { createDocument: async () => { events.push("create-offscreen"); offscreenReady = true; } },
      action: { setBadgeText: async () => {}, setBadgeBackgroundColor: async () => {} },
      tabs: {
        get: async (id: number) => ({ id, title: "Example", url: "https://example.com" }),
        sendMessage: async () => ({ ok: true, result: { elements: [], regions: [], viewport: { width: 100 } } }),
        onUpdated: listener, onRemoved: listener,
      },
      debugger: { onEvent: listener },
      tabCapture: { getMediaStreamId: async () => "stream-id" },
    };
    const context = { chrome, URL, crypto: { randomUUID: () => "lease" }, setTimeout, clearTimeout, testBridge: undefined as unknown as { attach: (id: number) => Promise<unknown> } };
    runInNewContext(`${read("browser-extension/background.js")}\nglobalThis.testBridge = { attach };`, context);
    await context.testBridge.attach(1);
    expect(events).toEqual(["create-offscreen", "bridge-update-redactions", "bridge-start-capture"]);
    events.length = 0;
    await context.testBridge.attach(2);
    expect(events).toEqual(["bridge-stop-capture", "bridge-update-redactions", "bridge-start-capture"]);
  });

  it("omits generic form values and marks displayed secrets for pixel redaction", async () => {
    const dom = new JSDOM('<input id="generic" value="sk_live_abcdefghijklmnopqrstuvwxyz123456"><div>Token sk_live_abcdefghijklmnopqrstuvwxyz123456</div>', { runScripts: "outside-only", url: "https://example.com" });
    const listeners: Array<(message: unknown, sender: unknown, reply: (value: unknown) => void) => boolean | void> = [];
    Object.defineProperty(dom.window.HTMLElement.prototype, "getBoundingClientRect", { value: () => ({ x: 0, y: 0, width: 100, height: 20 }) });
    Object.defineProperty(dom.window.document.querySelector("div"), "innerText", { value: "Token sk_live_abcdefghijklmnopqrstuvwxyz123456" });
    Object.assign(dom.window, { TextEncoder, chrome: { runtime: { sendMessage: () => Promise.resolve({ ok: true }), onMessage: { addListener: (listener: typeof listeners[number]) => listeners.push(listener) } } } });
    dom.window.eval(read("browser-extension/content.js"));
    const result = await new Promise<Record<string, unknown>>(resolve => {
      listeners[0]({ type: "bridge-page-action", action: { kind: "snapshot", delta: false } }, null, value => {
        const response = value as { ok: boolean; result?: Record<string, unknown>; error?: string };
        if (!response.ok || !response.result) throw new Error(response.error ?? "snapshot failed");
        resolve(response.result);
      });
    });
    expect((result.elements as Array<{ value: string | null }>)[0].value).toBeNull();
    const region = (result.regions as Array<{ text: string; sensitiveText: boolean }>)[0];
    expect(region.text).toContain("[credential-like value redacted]");
    expect(region.sensitiveText).toBe(true);
    dom.window.close();
  });

  it("rejects a delayed page action after the authorized DOM generation changes", async () => {
    const dom = new JSDOM('<button id="target">Continue</button>', { runScripts: "outside-only", url: "https://example.com" });
    const listeners: Array<(message: unknown, sender: unknown, reply: (value: unknown) => void) => boolean | void> = [];
    Object.defineProperty(dom.window.HTMLElement.prototype, "getBoundingClientRect", { value: () => ({ x: 0, y: 0, width: 100, height: 20 }) });
    Object.defineProperty(dom.window.document.querySelector("button"), "innerText", { value: "Continue" });
    Object.assign(dom.window, { TextEncoder, chrome: { runtime: { sendMessage: () => Promise.resolve({ ok: true }), onMessage: { addListener: (listener: typeof listeners[number]) => listeners.push(listener) } } } });
    dom.window.eval(read("browser-extension/content.js"));
    const invoke = (message: unknown) => new Promise<{ ok: boolean; result?: Record<string, unknown>; error?: string }>(resolve => listeners[0](message, null, value => resolve(value as { ok: boolean; result?: Record<string, unknown>; error?: string })));
    const snapshot = await invoke({ type: "bridge-page-action", action: { kind: "snapshot", delta: false } });
    const generation = snapshot.result?.pageGeneration;
    const elementId = (snapshot.result?.elements as Array<{ id: string }>)[0].id;
    dom.window.document.querySelector("button")?.setAttribute("aria-label", "Changed");
    await new Promise(resolve => dom.window.setTimeout(resolve, 0));
    const delayed = await invoke({ type: "bridge-page-action", action: { kind: "click", elementId }, expectedPageGeneration: generation });
    expect(delayed.ok).toBe(false);
    expect(delayed.error).toContain("page changed");
    dom.window.close();
  });

  it("ships a stable Chrome MV3 identity with narrowly declared browser capabilities", () => {
    const manifest = JSON.parse(read("browser-extension/manifest.json"));
    expect(manifest.manifest_version).toBe(3);
    expect(manifest.permissions).toEqual(expect.arrayContaining(["nativeMessaging", "debugger", "tabCapture", "activeTab"]));
    expect(manifest.key).toMatch(/^MIIB/);
  });

  it("redacts sensitive screenshots and deduplicates commands across reconnects", () => {
    const background = read("browser-extension/background.js");
    expect(background).toContain("completedCommands");
    expect(background).toContain("redactScreenshot");
    expect(background).toContain("redactionApplied");
    expect(background).toContain("Runtime.consoleAPICalled");
    expect(background).toContain("Network.responseReceived");
  });

  it("streams backpressured pre-redacted WebP frames without double encoding", () => {
    const offscreen = read("browser-extension/offscreen.js");
    const background = read("browser-extension/background.js");
    const surface = read("src/components/BrowserSurface.tsx");
    expect(offscreen).toContain("const FRAME_INTERVAL_MS = 100");
    expect(offscreen).toContain("if (!stream?.active || encoding)");
    expect(offscreen).toContain('canvas.toBlob(resolve, "image/webp", 0.52)');
    expect(offscreen).toContain('message.type === "bridge-update-redactions"');
    expect(offscreen).toContain("redactionEpoch: frameEpoch");
    expect(background).toContain('queueFrame({ leaseId, dataUrl: latestRedactedFrame');
    expect(background).toContain("if (frameInFlight) { pendingFrame = frame; return; }");
    expect(background).toContain('message.type === "frame_ack"');
    expect(background).toContain('post("page_invalidated", { leaseId');
    expect(background).toContain("attachedTabId !== expectedTabId || leaseId !== expectedLeaseId");
    expect(background).toContain("message.redactionEpoch === acknowledgedRedactionEpoch");
    expect(background).toContain("snapshotMaxTimer = setTimeout");
    expect(background).toContain("expectedSnapshotGeneration !== snapshotGeneration");
    expect(background).toContain('action.kind === "snapshot"');
    expect(background).toContain("result = await sendSnapshot(Boolean(action.delta))");
    expect(surface).toContain("bridgeApi.browserFrame(frameRevision.current)");
  });

  it("exposes a session and lease scoped agent browser tool through application context", () => {
    const supervisor = read("src-tauri/bridge-core/src/browser_bridge.rs");
    const host = read("src-tauri/bridge-core/src/live_turn.rs");
    expect(supervisor).toContain("pub fn capability_context(&self, session_id: &str, runtime_pid: u32)");
    expect(supervisor).toContain("capability.lease_id != lease.id");
    expect(supervisor).toContain("lease.status != \"active\"");
    expect(supervisor).toContain("Bridge authenticated-browser capability: AVAILABLE");
    expect(host).toContain("state.browser_bridge.capability_context(session_id, runtime.process_id())");
  });

  it("labels web content as untrusted and emits stable DOM deltas", () => {
    const content = read("browser-extension/content.js");
    expect(content).toContain('contentBoundary: "untrusted_web_content"');
    expect(content).toContain("promptInjectionSuspected");
    expect(content).toContain("const ids = new WeakMap()");
    expect(content).toContain("MutationObserver");
    expect(content).toContain("removedIds");
    expect(content).toContain("semanticRegions");
    expect(content).toContain("[one-time code redacted]");
  });

  it("includes a Safari Web Extension and deterministic common-site skills", () => {
    const manifest = JSON.parse(read("safari-extension/Resources/manifest.json"));
    expect(manifest.permissions).toContain("nativeMessaging");
    for (const name of ["github", "google-workspace", "notion"]) {
      const skill = JSON.parse(read(`browser-skills/${name}.json`));
      expect(skill.domains.length).toBeGreaterThan(0);
      expect(skill.steps.some((step: { action: string }) => step.action === "approval")).toBe(true);
    }
  });
});

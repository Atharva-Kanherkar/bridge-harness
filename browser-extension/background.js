const HOST = "dev.bridge.deck.browser";
let port;
let connected = false;
let attachedTabId = null;
let attachedTitle = null;
let leaseId = null;
let reconnectTimer;
let debuggerAttached = false;
let sensitiveElements = new Map();
let latestRawFrame = null;
let captureSourceWidth = null;
let snapshotReady = false;
let snapshotTimer;

async function completedCommands() {
  return (await chrome.storage.session.get("completedCommands")).completedCommands ?? {};
}

async function rememberCommand(id, result) {
  const values = await completedCommands();
  values[id] = result;
  const entries = Object.entries(values).slice(-200);
  await chrome.storage.session.set({ completedCommands: Object.fromEntries(entries) });
}

function post(type, payload = {}) {
  if (port) port.postMessage({ type, payload });
}

function safeDebugUrl(value) {
  try { const url = new URL(value); url.search = ""; url.hash = ""; return url.toString(); } catch { return "[invalid URL]"; }
}

function safeDebugText(value) {
  return String(value).replace(/(bearer\s+|password[=:]\s*|token[=:]\s*)[^\s,;]+/gi, "$1[redacted]").slice(0, 1000);
}

function connect() {
  clearTimeout(reconnectTimer);
  try {
    port = chrome.runtime.connectNative(HOST);
    port.onMessage.addListener(message => void handleHostMessage(message));
    port.onDisconnect.addListener(() => {
      connected = false; port = undefined;
      reconnectTimer = setTimeout(connect, 1500);
    });
    connected = true;
    post("paired", { extensionVersion: chrome.runtime.getManifest().version, attachedTabId, leaseId });
    if (attachedTabId != null) chrome.tabs.get(attachedTabId).then(async tab => post("attached", { tab: await tabSummary(tab), leaseId, reconnected: true })).catch(() => void detach("tab_unavailable"));
  } catch { reconnectTimer = setTimeout(connect, 1500); }
}

async function tabSummary(tab) {
  let domain = null;
  try { domain = new URL(tab.url).hostname; } catch { /* Chrome pages are intentionally unavailable. */ }
  return { id: tab.id, title: tab.title ?? "Untitled tab", url: tab.url ?? "", domain, favIconUrl: tab.favIconUrl ?? null, attached: tab.id === attachedTabId };
}

async function listTabs() {
  const tabs = await chrome.tabs.query({});
  post("tab_catalog", { tabs: await Promise.all(tabs.filter(tab => /^https?:/.test(tab.url ?? "")).map(tabSummary)) });
}

async function attach(tabId, requestedLeaseId) {
  const tab = await chrome.tabs.get(tabId);
  if (!tab.id || !/^https?:/.test(tab.url ?? "")) throw new Error("Only HTTP(S) tabs can be attached");
  attachedTabId = tab.id; attachedTitle = tab.title ?? "Untitled tab"; leaseId = requestedLeaseId ?? crypto.randomUUID();
  await chrome.action.setBadgeText({ tabId, text: "ON" });
  await chrome.action.setBadgeBackgroundColor({ tabId, color: "#16a34a" });
  post("attached", { tab: await tabSummary(tab), leaseId });
  await sendSnapshot(false);
  await startLiveCapture(tab.id).catch(error => post("capture_status", { active: false, error: error.message }));
}

async function detach(reason = "manual") {
  if (attachedTabId != null) {
    await chrome.action.setBadgeText({ tabId: attachedTabId, text: "" }).catch(() => {});
    if (debuggerAttached) await chrome.debugger.detach({ tabId: attachedTabId }).catch(() => {});
  }
  await chrome.runtime.sendMessage({ type: "bridge-stop-capture" }).catch(() => {});
  const previousTabId = attachedTabId;
  attachedTabId = null; attachedTitle = null; leaseId = null; debuggerAttached = false;
  post("detached", { tabId: previousTabId, reason });
}

async function sendSnapshot(delta) {
  if (attachedTabId == null) throw new Error("No attached tab");
  const response = await chrome.tabs.sendMessage(attachedTabId, { type: "bridge-page-action", action: { kind: "snapshot", delta } });
  if (!response?.ok) throw new Error(response?.error ?? "Snapshot failed");
  if (!delta) sensitiveElements.clear();
  for (const element of response.result.elements) {
    if (element.sensitiveKind) sensitiveElements.set(element.id, element.bounds);
    else sensitiveElements.delete(element.id);
  }
  snapshotReady = true;
  post("snapshot", response.result);
  return response.result;
}

async function screenshot(commandId) {
  if (attachedTabId == null) throw new Error("No attached tab");
  const tab = await chrome.tabs.get(attachedTabId);
  await sendSnapshot(false);
  const sensitiveBounds = [...sensitiveElements.values()];
  const captured = latestRawFrame ?? await chrome.tabs.captureVisibleTab(tab.windowId, { format: "jpeg", quality: 72 });
  const dataUrl = sensitiveBounds.length ? await redactScreenshot(captured, sensitiveBounds, tab, captureSourceWidth) : captured;
  post("screenshot", { commandId, dataUrl, redactedRegions: sensitiveBounds.length, redactionApplied: sensitiveBounds.length > 0 });
  return { redactedRegions: sensitiveBounds.length };
}

async function redactScreenshot(dataUrl, regions, tab, sourceWidth) {
  const bitmap = await createImageBitmap(await (await fetch(dataUrl)).blob());
  const canvas = new OffscreenCanvas(bitmap.width, bitmap.height);
  const context = canvas.getContext("2d");
  context.drawImage(bitmap, 0, 0);
  const scale = bitmap.width / Math.max(1, Number(sourceWidth ?? tab.width ?? bitmap.width));
  context.fillStyle = "#151517";
  for (const region of regions) context.fillRect(region.x * scale, region.y * scale, region.width * scale, region.height * scale);
  const bytes = new Uint8Array(await (await canvas.convertToBlob({ type: "image/jpeg", quality: 0.72 })).arrayBuffer());
  let binary = "";
  for (let offset = 0; offset < bytes.length; offset += 0x8000) binary += String.fromCharCode(...bytes.subarray(offset, offset + 0x8000));
  return `data:image/jpeg;base64,${btoa(binary)}`;
}

async function startLiveCapture(tabId) {
  const contexts = await chrome.runtime.getContexts({ contextTypes: ["OFFSCREEN_DOCUMENT"], documentUrls: [chrome.runtime.getURL("offscreen.html")] });
  if (!contexts.length) await chrome.offscreen.createDocument({ url: "offscreen.html", reasons: ["USER_MEDIA"], justification: "Mirror one user-approved tab inside Bridge" });
  const streamId = await chrome.tabCapture.getMediaStreamId({ targetTabId: tabId });
  const result = await chrome.runtime.sendMessage({ type: "bridge-start-capture", streamId });
  if (!result?.ok) throw new Error(result?.error ?? "Tab capture failed");
  post("capture_status", { active: true });
}

async function enableDebugger() {
  if (attachedTabId == null || debuggerAttached) return;
  await chrome.debugger.attach({ tabId: attachedTabId }, "1.3");
  debuggerAttached = true;
  await chrome.debugger.sendCommand({ tabId: attachedTabId }, "Network.enable");
  await chrome.debugger.sendCommand({ tabId: attachedTabId }, "Runtime.enable");
  post("debugger_status", { attached: true });
}

chrome.debugger.onEvent.addListener((source, method, params) => {
  if (source.tabId !== attachedTabId || !debuggerAttached) return;
  if (method === "Runtime.consoleAPICalled") {
    post("debug_event", { kind: "console", level: params.type, text: safeDebugText((params.args ?? []).map(arg => arg.value ?? arg.description ?? "").join(" ")), timestamp: params.timestamp });
  } else if (method === "Network.responseReceived") {
    post("debug_event", { kind: "network", method: params.response?.requestHeadersText ? "request" : null, url: safeDebugUrl(params.response?.url), status: params.response?.status, mimeType: params.response?.mimeType });
  }
});

async function executeCommand(message) {
  const { id, action } = message;
  const cached = (await completedCommands())[id];
  if (cached) { post("command_result", { id, ...cached, replayed: true }); return; }
  let result;
  if (action.kind === "list_tabs") result = await listTabs();
  else if (action.kind === "attach") result = await attach(action.tabId, action.leaseId);
  else if (action.kind === "detach") result = await detach(action.reason);
  else if (action.kind === "screenshot") result = await screenshot(id);
  else if (action.kind === "debugger") result = await enableDebugger();
  else if (action.kind === "focus") {
    if (attachedTabId == null) throw new Error("No attached tab");
    const tab = await chrome.tabs.get(attachedTabId);
    await chrome.windows.update(tab.windowId, { focused: true });
    await chrome.tabs.update(attachedTabId, { active: true });
    result = { focused: true };
  } else if (["click", "click_at", "type", "scroll", "navigate", "snapshot"].includes(action.kind)) {
    if (attachedTabId == null) throw new Error("No attached tab");
    const response = await chrome.tabs.sendMessage(attachedTabId, { type: "bridge-page-action", action });
    if (!response?.ok) throw new Error(response?.error ?? "Page action failed");
    result = response.result;
  } else throw new Error(`Unsupported command: ${action.kind}`);
  const value = { ok: true, result: result ?? null };
  await rememberCommand(id, value);
  post("command_result", { id, ...value, replayed: false });
  if (!["detach", "list_tabs", "snapshot", "screenshot"].includes(action.kind) && attachedTabId != null) setTimeout(() => void sendSnapshot(true).catch(() => {}), 250);
}

async function handleHostMessage(message) {
  try { await executeCommand(message); }
  catch (error) { post("command_result", { id: message.id, ok: false, error: error.message }); }
}

chrome.tabs.onUpdated.addListener(async (tabId, change, tab) => {
  if (tabId !== attachedTabId || !change.url) return;
  snapshotReady = false; sensitiveElements.clear();
  post("navigation", { tab: await tabSummary(tab), leaseId });
});
chrome.tabs.onRemoved.addListener(tabId => { if (tabId === attachedTabId) void detach("tab_closed"); });

chrome.runtime.onMessage.addListener((message, _sender, reply) => {
  if (message.type === "bridge-live-frame") {
    latestRawFrame = message.dataUrl; captureSourceWidth = message.sourceWidth;
    const sensitiveBounds = [...sensitiveElements.values()];
    if (attachedTabId != null && snapshotReady) chrome.tabs.get(attachedTabId).then(tab => sensitiveBounds.length ? redactScreenshot(message.dataUrl, sensitiveBounds, tab, captureSourceWidth) : message.dataUrl).then(dataUrl => post("frame", { dataUrl, redactedRegions: sensitiveBounds.length })).catch(() => {});
    reply({ ok: true }); return;
  }
  if (message.type === "bridge-dom-dirty") {
    snapshotReady = false; clearTimeout(snapshotTimer);
    snapshotTimer = setTimeout(() => void sendSnapshot(!message.full).catch(() => {}), 100);
    reply({ ok: true }); return;
  }
  if (message.type === "status") { reply({ connected, attached: attachedTabId != null, title: attachedTitle }); return; }
  if (message.type === "attach-active") {
    chrome.tabs.query({ active: true, currentWindow: true }).then(([tab]) => attach(tab.id)).then(() => reply({ ok: true, message: "This tab is attached to Bridge." })).catch(error => reply({ ok: false, message: error.message }));
    return true;
  }
  if (message.type === "detach-active") { detach().then(() => reply({ ok: true, message: "Detached." })); return true; }
});

connect();

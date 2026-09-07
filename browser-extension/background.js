const HOST = "dev.bridge.deck.browser";
let port;
let connected = false;
let attachedTabId = null;
let attachedTitle = null;
let leaseId = null;
let reconnectTimer;
let debuggerAttached = false;
let sensitiveElements = new Map();
let latestRedactedFrame = null;
let snapshotReady = false;
let snapshotTimer;
let snapshotMaxTimer;
let snapshotRefreshRunning = false;
let snapshotNeedsFull = false;
let snapshotGeneration = 0;
let acknowledgedRedactionEpoch = -1;
let frameInFlight = false;
let pendingFrame = null;
let commandQueue = Promise.resolve();
let offscreenCreation;

async function completedCommands() {
  return (await chrome.storage.session.get("completedCommands")).completedCommands ?? {};
}

async function rememberCommand(id, result, replayResult = result) {
  const values = await completedCommands();
  values[id] = replayResult;
  const entries = Object.entries(values).slice(-200);
  await chrome.storage.session.set({ completedCommands: Object.fromEntries(entries) });
}

function post(type, payload = {}) {
  if (port) port.postMessage({ type, payload });
}

function queueFrame(frame) {
  if (frameInFlight) { pendingFrame = frame; return; }
  frameInFlight = true;
  post("frame", frame);
}

function safeDebugUrl(value) {
  try { const url = new URL(value); url.search = ""; url.hash = ""; return url.toString(); } catch { return "[invalid URL]"; }
}

function safeDebugText(value) {
  return String(value).replace(/(bearer\s+|password[=:]\s*|token[=:]\s*)[^\s,;]+/gi, "$1[redacted]").slice(0, 1000);
}

function safePageText(value) {
  let text = String(value ?? "").replace(/\b(?:\d[ -]*?){13,19}\b/g, "[payment value redacted]");
  if (/\b(one[- ]?time|verification|security|otp)\s*(code|pin)?\b/i.test(text)) text = text.replace(/\b\d{4,8}\b/g, "[one-time code redacted]");
  return text.replace(/\b(?:bearer\s+)?[A-Za-z0-9_-]{32,}\b/gi, "[credential-like value redacted]").slice(0, 300);
}

function connect() {
  clearTimeout(reconnectTimer);
  try {
    port = chrome.runtime.connectNative(HOST);
    port.onMessage.addListener(message => void handleHostMessage(message));
    port.onDisconnect.addListener(() => {
      connected = false; port = undefined;
      frameInFlight = false; pendingFrame = null;
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
  return { id: tab.id, title: safePageText(tab.title ?? "Untitled tab"), url: tab.url ?? "", domain, favIconUrl: tab.favIconUrl ?? null, attached: tab.id === attachedTabId };
}

async function listTabs() {
  const tabs = await chrome.tabs.query({});
  post("tab_catalog", { tabs: await Promise.all(tabs.filter(tab => /^https?:/.test(tab.url ?? "")).map(tabSummary)) });
}

async function attach(tabId, requestedLeaseId) {
  const tab = await chrome.tabs.get(tabId);
  if (!tab.id || !/^https?:/.test(tab.url ?? "")) throw new Error("Only HTTP(S) tabs can be attached");
  if (attachedTabId != null) await detach("tab_switched");
  await ensureOffscreenDocument();
  attachedTabId = tab.id; attachedTitle = tab.title ?? "Untitled tab"; leaseId = requestedLeaseId ?? crypto.randomUUID();
  snapshotReady = false; snapshotGeneration += 1; acknowledgedRedactionEpoch = -1; sensitiveElements.clear(); latestRedactedFrame = null;
  await chrome.action.setBadgeText({ tabId, text: "ON" });
  await chrome.action.setBadgeBackgroundColor({ tabId, color: "#16a34a" });
  post("attached", { tab: await tabSummary(tab), leaseId });
  await sendSnapshot(false);
  try {
    await startLiveCapture(tab.id);
    return { captureActive: true, captureError: null };
  } catch (error) {
    post("capture_status", { active: false, error: error.message });
    return { captureActive: false, captureError: error.message };
  }
}

async function detach(reason = "manual") {
  if (attachedTabId != null) {
    await chrome.action.setBadgeText({ tabId: attachedTabId, text: "" }).catch(() => {});
    if (debuggerAttached) await chrome.debugger.detach({ tabId: attachedTabId }).catch(() => {});
  }
  await chrome.runtime.sendMessage({ type: "bridge-stop-capture" }).catch(() => {});
  const previousTabId = attachedTabId;
  attachedTabId = null; attachedTitle = null; leaseId = null; debuggerAttached = false;
  frameInFlight = false; pendingFrame = null; latestRedactedFrame = null; acknowledgedRedactionEpoch = -1;
  post("detached", { tabId: previousTabId, reason });
}

async function sendSnapshot(delta) {
  if (attachedTabId == null) throw new Error("No attached tab");
  const expectedTabId = attachedTabId;
  const expectedLeaseId = leaseId;
  const generation = snapshotGeneration;
  const response = await chrome.tabs.sendMessage(expectedTabId, { type: "bridge-page-action", action: { kind: "snapshot", delta } });
  if (attachedTabId !== expectedTabId || leaseId !== expectedLeaseId || snapshotGeneration !== generation) throw new Error("Page changed during snapshot");
  if (!response?.ok) throw new Error(response?.error ?? "Snapshot failed");
  if (!delta) sensitiveElements.clear();
  for (const id of response.result.removedIds ?? []) sensitiveElements.delete(id);
  for (const element of response.result.elements) {
    if (element.sensitiveKind) sensitiveElements.set(element.id, element.bounds);
    else sensitiveElements.delete(element.id);
  }
  for (const region of response.result.regions ?? []) {
    if (region.sensitiveText) sensitiveElements.set(region.id, region.bounds);
    else sensitiveElements.delete(region.id);
  }
  const redactionUpdate = await chrome.runtime.sendMessage({
    type: "bridge-update-redactions",
    regions: [...sensitiveElements.values()],
    viewportWidth: response.result.viewport?.width ?? 1,
    redactionEpoch: generation
  });
  if (attachedTabId !== expectedTabId || leaseId !== expectedLeaseId || snapshotGeneration !== generation) throw new Error("Page changed during redaction update");
  if (!redactionUpdate?.ok) throw new Error("Live-frame redaction update failed");
  acknowledgedRedactionEpoch = generation;
  snapshotReady = true;
  post("snapshot", { ...response.result, leaseId: expectedLeaseId, snapshotGeneration: generation });
  return response.result;
}

function scheduleSnapshotRefresh(full = false) {
  snapshotNeedsFull ||= full;
  clearTimeout(snapshotTimer);
  snapshotTimer = setTimeout(() => void runScheduledSnapshotRefresh(), 100);
  if (!snapshotMaxTimer) snapshotMaxTimer = setTimeout(() => void runScheduledSnapshotRefresh(), 500);
}

async function runScheduledSnapshotRefresh() {
  if (attachedTabId == null) return;
  if (snapshotRefreshRunning) {
    snapshotNeedsFull = true;
    clearTimeout(snapshotTimer);
    snapshotTimer = setTimeout(() => void runScheduledSnapshotRefresh(), 100);
    return;
  }
  snapshotRefreshRunning = true;
  clearTimeout(snapshotTimer); snapshotTimer = undefined;
  clearTimeout(snapshotMaxTimer); snapshotMaxTimer = undefined;
  const full = snapshotNeedsFull; snapshotNeedsFull = false;
  try { await sendSnapshot(!full); }
  catch (error) {
    if (attachedTabId != null) scheduleSnapshotRefresh(true);
    post("capture_status", { active: true, error: `Page snapshot retrying: ${error.message}` });
  } finally {
    snapshotRefreshRunning = false;
    if (snapshotNeedsFull && !snapshotTimer) scheduleSnapshotRefresh(true);
  }
}

async function refreshAfterNavigation(attempt = 0) {
  try { await sendSnapshot(false); }
  catch (error) {
    if (attempt < 5 && attachedTabId != null) {
      snapshotTimer = setTimeout(() => void refreshAfterNavigation(attempt + 1), 150 * (attempt + 1));
    } else {
      post("capture_status", { active: true, error: `Page snapshot unavailable: ${error.message}` });
    }
  }
}

async function screenshot(commandId, publishToMirror = true) {
  if (attachedTabId == null) throw new Error("No attached tab");
  const expectedTabId = attachedTabId;
  const expectedLeaseId = leaseId;
  const generation = snapshotGeneration;
  const tab = await chrome.tabs.get(expectedTabId);
  if (attachedTabId !== expectedTabId || leaseId !== expectedLeaseId || snapshotGeneration !== generation) throw new Error("Tab changed during screenshot");
  await sendSnapshot(false);
  if (attachedTabId !== expectedTabId || leaseId !== expectedLeaseId || snapshotGeneration !== generation) throw new Error("Page changed during screenshot");
  const sensitiveBounds = [...sensitiveElements.values()];
  const captured = await chrome.tabs.captureVisibleTab(tab.windowId, { format: "jpeg", quality: 72 });
  if (attachedTabId !== expectedTabId || leaseId !== expectedLeaseId || snapshotGeneration !== generation) throw new Error("Page changed during screenshot capture");
  const dataUrl = sensitiveBounds.length ? await redactScreenshot(captured, sensitiveBounds, tab, tab.width) : captured;
  if (attachedTabId !== expectedTabId || leaseId !== expectedLeaseId || snapshotGeneration !== generation) throw new Error("Page changed during screenshot redaction");
  if (publishToMirror) post("screenshot", { commandId, leaseId: expectedLeaseId, dataUrl, redactedRegions: sensitiveBounds.length, redactionApplied: sensitiveBounds.length > 0 });
  return { redactedRegions: sensitiveBounds.length, dataUrl };
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

async function ensureOffscreenDocument() {
  if (!offscreenCreation) {
    offscreenCreation = (async () => {
      const contexts = await chrome.runtime.getContexts({ contextTypes: ["OFFSCREEN_DOCUMENT"], documentUrls: [chrome.runtime.getURL("offscreen.html")] });
      if (!contexts.length) await chrome.offscreen.createDocument({ url: "offscreen.html", reasons: ["USER_MEDIA"], justification: "Mirror one user-approved tab inside Bridge" });
    })();
  }
  try { await offscreenCreation; }
  finally { offscreenCreation = undefined; }
}

async function startLiveCapture(tabId) {
  await ensureOffscreenDocument();
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
  const authorizeLease = () => {
    if (attachedTabId == null) throw new Error("No attached tab");
    if (message.expectedLeaseId !== leaseId || message.expectedTabId !== attachedTabId) throw new Error("The authorized tab lease changed before this action ran");
  };
  const authorizePageAction = () => {
    authorizeLease();
    if (!snapshotReady) throw new Error("The attached page is not ready");
    if (message.expectedLeaseId !== leaseId || message.expectedTabId !== attachedTabId || message.expectedSnapshotGeneration !== snapshotGeneration) throw new Error("The authorized page changed before this action ran");
  };
  if (action.kind === "list_tabs") result = await listTabs();
  else if (action.kind === "attach") result = await attach(action.tabId, action.leaseId);
  else if (action.kind === "detach") result = await detach(action.reason);
  else if (action.kind === "screenshot") {
    authorizeLease();
    const captured = await screenshot(id, !action.includeDataUrl);
    result = action.includeDataUrl ? captured : { redactedRegions: captured.redactedRegions };
  }
  else if (action.kind === "snapshot") {
    authorizeLease();
    result = await sendSnapshot(Boolean(action.delta));
  }
  else if (action.kind === "debugger") result = await enableDebugger();
  else if (action.kind === "focus") {
    authorizeLease();
    const expectedTabId = attachedTabId;
    const tab = await chrome.tabs.get(attachedTabId);
    authorizeLease();
    await chrome.windows.update(tab.windowId, { focused: true });
    authorizeLease();
    await chrome.tabs.update(expectedTabId, { active: true });
    authorizeLease();
    result = { focused: true };
  } else if (["click", "click_at", "type", "scroll", "navigate"].includes(action.kind)) {
    authorizePageAction();
    const expectedTabId = attachedTabId;
    const response = await chrome.tabs.sendMessage(expectedTabId, { type: "bridge-page-action", action, expectedPageGeneration: action.kind === "snapshot" ? undefined : message.expectedPageGeneration });
    if (!response?.ok) throw new Error(response?.error ?? "Page action failed");
    result = response.result;
  } else throw new Error(`Unsupported command: ${action.kind}`);
  const value = { ok: true, result: result ?? null };
  const replayValue = action.kind === "screenshot" && action.includeDataUrl
    ? { ok: false, error: "Screenshot result expired; request a fresh screenshot" }
    : value;
  await rememberCommand(id, value, replayValue);
  post("command_result", { id, ...value, replayed: false });
  if (!["detach", "list_tabs", "snapshot", "screenshot"].includes(action.kind) && attachedTabId != null) setTimeout(() => void sendSnapshot(true).catch(() => {}), 250);
}

async function handleHostMessage(message) {
  if (message.type === "frame_ack") {
    frameInFlight = false;
    if (pendingFrame) { const frame = pendingFrame; pendingFrame = null; queueFrame(frame); }
    return;
  }
  commandQueue = commandQueue.then(() => executeCommand(message)).catch(error => post("command_result", { id: message.id, ok: false, error: error.message }));
  await commandQueue;
}

chrome.tabs.onUpdated.addListener(async (tabId, change, tab) => {
  if (tabId !== attachedTabId) return;
  if (change.url) {
    const wasReady = snapshotReady; snapshotReady = false; snapshotGeneration += 1; acknowledgedRedactionEpoch = -1; sensitiveElements.clear(); latestRedactedFrame = null;
    if (wasReady) post("page_invalidated", { leaseId, reason: "navigation" });
    post("navigation", { tab: await tabSummary(tab), leaseId });
  }
  if (change.status === "complete") {
    scheduleSnapshotRefresh(true);
  }
});
chrome.tabs.onRemoved.addListener(tabId => { if (tabId === attachedTabId) void detach("tab_closed"); });

chrome.runtime.onMessage.addListener((message, _sender, reply) => {
  if (message.type === "bridge-live-frame") {
    latestRedactedFrame = message.dataUrl;
    if (attachedTabId != null && snapshotReady && message.redactionEpoch === acknowledgedRedactionEpoch) queueFrame({ leaseId, dataUrl: latestRedactedFrame, redactedRegions: message.redactedRegions ?? 0, sequence: message.sequence ?? 0, snapshotGeneration: acknowledgedRedactionEpoch });
    reply({ ok: true }); return;
  }
  if (message.type === "bridge-capture-error") {
    post("capture_status", { active: true, error: message.error ?? "Live-frame capture failed" });
    reply({ ok: true }); return;
  }
  if (message.type === "bridge-dom-dirty") {
    const wasReady = snapshotReady; snapshotReady = false; snapshotGeneration += 1; acknowledgedRedactionEpoch = -1;
    if (wasReady) post("page_invalidated", { leaseId, reason: "dom_dirty" });
    scheduleSnapshotRefresh(Boolean(message.full));
    reply({ ok: true }); return;
  }
  if (message.type === "status") { reply({ connected, attached: attachedTabId != null, title: attachedTitle }); return; }
  if (message.type === "attach-active") {
    commandQueue = commandQueue.then(async () => { const [tab] = await chrome.tabs.query({ active: true, currentWindow: true }); return attach(tab.id); });
    commandQueue.then(result => reply({ ok: true, message: result.captureActive ? "Attached with live mirror enabled." : `Attached, but the live mirror could not start: ${result.captureError}` })).catch(error => reply({ ok: false, message: error.message }));
    return true;
  }
  if (message.type === "detach-active") { commandQueue = commandQueue.then(() => detach()); commandQueue.then(() => reply({ ok: true, message: "Detached." })).catch(error => reply({ ok: false, message: error.message })); return true; }
});

connect();

let attachedTabId = null;
let leaseId = null;

async function native(message) {
  return browser.runtime.sendNativeMessage("dev.bridge.deck.browser", message);
}

async function tabSummary(tab) {
  let domain = null;
  try { domain = new URL(tab.url).hostname; } catch { /* Protected Safari pages are excluded. */ }
  return { id: tab.id, title: tab.title ?? "Untitled tab", url: tab.url ?? "", domain, favIconUrl: tab.favIconUrl ?? null, attached: tab.id === attachedTabId };
}

async function attach(tabId) {
  const tab = await browser.tabs.get(tabId);
  if (!/^https?:/.test(tab.url ?? "")) throw new Error("Only HTTP(S) tabs can be attached");
  attachedTabId = tabId; leaseId = crypto.randomUUID();
  await native({ type: "attached", payload: { tab: await tabSummary(tab), leaseId, browser: "safari" } });
  await snapshot(false);
}

async function snapshot(delta) {
  const response = await browser.tabs.sendMessage(attachedTabId, { type: "bridge-page-action", action: { kind: "snapshot", delta } });
  if (!response?.ok) throw new Error(response?.error ?? "Snapshot failed");
  await native({ type: "snapshot", payload: response.result });
  return response.result;
}

async function execute(command) {
  if (!command?.id || !command.action) return;
  const action = command.action;
  try {
    let result = null;
    if (action.kind === "list_tabs") {
      const tabs = await browser.tabs.query({});
      await native({ type: "tab_catalog", payload: { tabs: await Promise.all(tabs.filter(tab => /^https?:/.test(tab.url ?? "")).map(tabSummary)) } });
    } else if (action.kind === "attach") result = await attach(action.tabId);
    else if (action.kind === "detach") { attachedTabId = null; leaseId = null; await native({ type: "detached", payload: { reason: action.reason } }); }
    else if (action.kind === "screenshot") {
      const tab = await browser.tabs.get(attachedTabId);
      const page = await snapshot(false);
      const sensitive = page.elements.filter(element => element.sensitiveKind).map(element => element.bounds);
      const captured = await browser.tabs.captureVisibleTab();
      const dataUrl = sensitive.length ? await redactScreenshot(captured, sensitive, tab) : captured;
      await native({ type: "screenshot", payload: { commandId: command.id, dataUrl, redactedRegions: sensitive.length, redactionApplied: sensitive.length > 0 } });
    } else if (["click", "click_at", "type", "scroll", "navigate", "snapshot"].includes(action.kind)) {
      const response = await browser.tabs.sendMessage(attachedTabId, { type: "bridge-page-action", action });
      if (!response?.ok) throw new Error(response?.error ?? "Page action failed");
      result = response.result;
    }
    await native({ type: "command_result", payload: { id: command.id, ok: true, result } });
  } catch (error) {
    await native({ type: "command_result", payload: { id: command.id, ok: false, error: error.message } });
  }
}

async function redactScreenshot(dataUrl, regions, tab) {
  const bitmap = await createImageBitmap(await (await fetch(dataUrl)).blob());
  const canvas = new OffscreenCanvas(bitmap.width, bitmap.height);
  const context = canvas.getContext("2d");
  context.drawImage(bitmap, 0, 0);
  const scale = bitmap.width / Math.max(1, Number(tab.width ?? bitmap.width));
  context.fillStyle = "#151517";
  for (const region of regions) context.fillRect(region.x * scale, region.y * scale, region.width * scale, region.height * scale);
  const bytes = new Uint8Array(await (await canvas.convertToBlob({ type: "image/jpeg", quality: 0.72 })).arrayBuffer());
  let binary = "";
  for (let offset = 0; offset < bytes.length; offset += 0x8000) binary += String.fromCharCode(...bytes.subarray(offset, offset + 0x8000));
  return `data:image/jpeg;base64,${btoa(binary)}`;
}

browser.runtime.onMessage.addListener(message => {
  if (message.type === "status") return Promise.resolve({ connected: true, attached: attachedTabId != null });
  if (message.type === "attach-active") return browser.tabs.query({ active: true, currentWindow: true }).then(([tab]) => attach(tab.id)).then(() => ({ ok: true, message: "This Safari tab is attached to Bridge." }));
  if (message.type === "detach-active") { attachedTabId = null; leaseId = null; return native({ type: "detached", payload: { reason: "manual" } }).then(() => ({ ok: true, message: "Detached." })); }
});

async function poll() {
  try { await execute(await native({ type: "safari_poll", payload: { leaseId } })); } finally { setTimeout(poll, 400); }
}
void native({ type: "paired", payload: { browser: "safari", extensionVersion: browser.runtime.getManifest().version } }).then(poll);

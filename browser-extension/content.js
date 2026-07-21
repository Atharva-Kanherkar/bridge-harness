(() => {
  const ids = new WeakMap();
  let nextId = 1;
  let revision = 0;
  const changed = new Set();
  const removedIds = new Set();
  let dirtyTimer;
  const injectionPatterns = [
    ["override_previous_instructions", /ignore (all|any|the) previous instructions/i],
    ["system_prompt_extraction", /system prompt/i],
    ["secret_extraction", /reveal (your|the) (secret|credential|token)/i],
    ["authority_impersonation", /act as (an? )?(administrator|system)/i],
    ["credential_exfiltration", /send (the|your) (cookie|password|token)/i]
  ];

  function elementId(element) {
    let id = ids.get(element);
    if (!id) { id = `e${nextId++}`; ids.set(element, id); }
    return id;
  }

  function isVisible(element) {
    const style = getComputedStyle(element);
    const rect = element.getBoundingClientRect();
    return style.visibility !== "hidden" && style.display !== "none" && rect.width > 0 && rect.height > 0;
  }

  function sensitiveKind(element) {
    if (!(element instanceof HTMLInputElement || element instanceof HTMLTextAreaElement)) return null;
    const value = `${element.type} ${element.name} ${element.autocomplete} ${element.getAttribute("aria-label") ?? ""}`.toLowerCase();
    if (/password|current-password|new-password|one-time-code/.test(value)) return "credential";
    if (/cc-|card|payment|iban|routing|account-number/.test(value)) return "payment";
    return null;
  }

  function roleOf(element) {
    return element.getAttribute("role") || ({ A: "link", BUTTON: "button", INPUT: "textbox", SELECT: "combobox", TEXTAREA: "textbox" }[element.tagName] ?? "element");
  }

  function compactElement(element) {
    if (!(element instanceof HTMLElement) || !isVisible(element)) return null;
    const rect = element.getBoundingClientRect();
    const kind = sensitiveKind(element);
    const rawText = kind ? "[sensitive field]" : (element.innerText || element.getAttribute("aria-label") || element.getAttribute("alt") || "").replace(/\s+/g, " ").trim().slice(0, 240);
    return {
      id: elementId(element),
      role: roleOf(element),
      name: rawText,
      tag: element.tagName.toLowerCase(),
      value: kind ? "[redacted]" : ("value" in element ? String(element.value ?? "").slice(0, 160) : null),
      disabled: "disabled" in element ? Boolean(element.disabled) : false,
      sensitiveKind: kind,
      contentBoundary: "untrusted_web_content",
      promptInjectionSuspected: injectionPatterns.some(([, pattern]) => pattern.test(rawText)),
      bounds: { x: Math.round(rect.x), y: Math.round(rect.y), width: Math.round(rect.width), height: Math.round(rect.height) }
    };
  }

  function interactiveElements() {
    return [...document.querySelectorAll("a[href],button,input,select,textarea,[role=button],[role=link],[contenteditable=true],[tabindex]")]
      .map(compactElement).filter(Boolean).slice(0, 500);
  }

  function snapshot(delta = false) {
    const elements = delta
      ? [...changed].flatMap(node => node instanceof Element ? [compactElement(node), ...[...node.querySelectorAll("a[href],button,input,select,textarea,[role=button],[role=link],[contenteditable=true],[tabindex]")].map(compactElement)] : []).filter(Boolean).slice(0, 300)
      : interactiveElements();
    const removed = [...removedIds];
    changed.clear(); removedIds.clear();
    revision += 1;
    const pageText = (document.body?.innerText ?? "").slice(0, 50000);
    const promptInjectionSignals = injectionPatterns.filter(([, pattern]) => pattern.test(pageText)).map(([id]) => id);
    const payload = { revision, delta, url: location.href, title: document.title, elements, removedIds: removed, promptInjectionSignals, contentBoundary: "untrusted_web_content", viewport: { width: innerWidth, height: innerHeight, scrollX, scrollY } };
    const serializedBytes = new TextEncoder().encode(JSON.stringify(payload)).byteLength;
    return { ...payload, serializedBytes, estimatedTokens: Math.ceil(serializedBytes / 4) };
  }

  function target(id) {
    return [...document.querySelectorAll("*")].find(element => ids.get(element) === id);
  }

  function outwardEffect(element) {
    const label = `${element?.innerText ?? ""} ${element?.getAttribute?.("aria-label") ?? ""} ${element?.getAttribute?.("name") ?? ""}`.toLowerCase();
    return ["submit", "send", "delete", "purchase", "buy", "pay", "publish", "merge"].find(word => label.includes(word)) ?? null;
  }

  async function execute(action) {
    const element = action.elementId ? target(action.elementId) : null;
    if (action.kind === "snapshot") return snapshot(Boolean(action.delta));
    if (action.kind === "click") { if (!element) throw new Error("Element is no longer available"); if (outwardEffect(element) && !action.approvalGranted) throw new Error("Sensitive click was blocked because Bridge approval is missing"); element.click(); }
    else if (action.kind === "type") {
      if (!(element instanceof HTMLInputElement || element instanceof HTMLTextAreaElement || element?.isContentEditable)) throw new Error("Element is not editable");
      if (sensitiveKind(element)) throw new Error("Sensitive fields require user takeover");
      element.focus();
      if (element.isContentEditable) element.textContent = action.text ?? "";
      else element.value = action.text ?? "";
      element.dispatchEvent(new InputEvent("input", { bubbles: true, inputType: "insertText", data: action.text ?? "" }));
      element.dispatchEvent(new Event("change", { bubbles: true }));
    } else if (action.kind === "click_at") {
      const pointTarget = document.elementFromPoint(Number(action.x), Number(action.y));
      if (!(pointTarget instanceof HTMLElement)) throw new Error("Nothing actionable is at that point");
      if (sensitiveKind(pointTarget)) throw new Error("Sensitive fields require user takeover");
      if (outwardEffect(pointTarget) && !action.approvalGranted) throw new Error("Sensitive click was blocked because Bridge approval is missing");
      pointTarget.click();
    } else if (action.kind === "scroll") window.scrollBy({ top: Number(action.y ?? 480), left: Number(action.x ?? 0), behavior: "smooth" });
    else if (action.kind === "navigate") location.assign(action.url);
    else throw new Error(`Unsupported page action: ${action.kind}`);
    return { ok: true, revision };
  }

  const observer = new MutationObserver(records => records.forEach(record => {
    if (record.target instanceof Element) changed.add(record.target);
    record.addedNodes.forEach(node => {
      if (!(node instanceof Element)) return;
      changed.add(node);
      [node, ...node.querySelectorAll("*")].forEach(element => { const id = ids.get(element); if (id) removedIds.delete(id); });
    });
    record.removedNodes.forEach(node => {
      if (!(node instanceof Element)) return;
      [node, ...node.querySelectorAll("*")].forEach(element => { const id = ids.get(element); if (id) removedIds.add(id); });
    });
    clearTimeout(dirtyTimer);
    dirtyTimer = setTimeout(() => chrome.runtime.sendMessage({ type: "bridge-dom-dirty" }).catch(() => {}), 80);
  }));
  observer.observe(document, { subtree: true, childList: true, attributes: true, characterData: true });
  addEventListener("scroll", () => chrome.runtime.sendMessage({ type: "bridge-dom-dirty", full: true }).catch(() => {}), { passive: true });
  addEventListener("resize", () => chrome.runtime.sendMessage({ type: "bridge-dom-dirty", full: true }).catch(() => {}), { passive: true });

  chrome.runtime.onMessage.addListener((message, _sender, reply) => {
    if (message?.type !== "bridge-page-action") return;
    execute(message.action).then(result => reply({ ok: true, result })).catch(error => reply({ ok: false, error: error.message }));
    return true;
  });
})();

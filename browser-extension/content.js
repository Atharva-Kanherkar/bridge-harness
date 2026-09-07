(() => {
  const ids = new WeakMap();
  let nextId = 1;
  let revision = 0;
  let pageGeneration = 0;
  const changed = new Set();
  const removedIds = new Set();
  const injectionPatterns = [
    ["override_previous_instructions", /ignore (all|any|the) previous instructions/i],
    ["system_prompt_extraction", /system prompt/i],
    ["secret_extraction", /reveal (your|the) (secret|credential|token)/i],
    ["authority_impersonation", /act as (an? )?(administrator|system)/i],
    ["credential_exfiltration", /send (the|your) (cookie|password|token)/i]
  ];
  const semanticSelector = "h1,h2,h3,h4,p,li,dt,dd,caption,[role=heading],[role=status],[role=alert]";

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

  function redactSensitiveText(value) {
    let text = normalizedText(value);
    text = text.replace(/\b(?:\d[ -]*?){13,19}\b/g, "[payment value redacted]");
    if (/\b(one[- ]?time|verification|security|otp)\s*(code|pin)?\b/i.test(text)) text = text.replace(/\b\d{4,8}\b/g, "[one-time code redacted]");
    return text.replace(/\b(?:bearer\s+)?[A-Za-z0-9_-]{32,}\b/gi, "[credential-like value redacted]").slice(0, 320);
  }

  function normalizedText(value) {
    return String(value ?? "").replace(/\s+/g, " ").trim().slice(0, 5000);
  }

  function containsSensitiveText(value) {
    const text = normalizedText(value);
    return /\b(?:\d[ -]*?){13,19}\b/.test(text)
      || /\b(?:bearer\s+)?[A-Za-z0-9_-]{32,}\b/i.test(text)
      || (/\b(one[- ]?time|verification|security|otp)\s*(code|pin)?\b/i.test(text) && /\b\d{4,8}\b/.test(text));
  }

  function compactElement(element) {
    if (!(element instanceof HTMLElement) || !isVisible(element)) return null;
    const rect = element.getBoundingClientRect();
    const kind = sensitiveKind(element);
    const rawText = kind ? "[sensitive field]" : redactSensitiveText(element.innerText || element.getAttribute("aria-label") || element.getAttribute("alt") || "").slice(0, 240);
    return {
      id: elementId(element),
      role: roleOf(element),
      name: rawText,
      tag: element.tagName.toLowerCase(),
      // Form values are private by default. Labels and surrounding semantic
      // text are enough for agent targeting without copying user-entered data.
      value: "value" in element ? (kind ? "[redacted]" : null) : null,
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

  function semanticRegions(delta = false) {
    const candidates = delta
      ? [...changed].flatMap(node => node instanceof Element ? [node, ...node.querySelectorAll(semanticSelector)] : [])
      : [...document.querySelectorAll(semanticSelector)];
    const semantic = candidates.map(element => {
      if (!(element instanceof HTMLElement) || !isVisible(element)) return null;
      const sourceText = normalizedText(element.innerText || element.getAttribute("aria-label") || "");
      const text = redactSensitiveText(sourceText);
      if (!text) return null;
      const rect = element.getBoundingClientRect();
      return {
        id: elementId(element), role: roleOf(element), text, interactive: false,
        sensitiveText: containsSensitiveText(sourceText),
        contentBoundary: "untrusted_web_content",
        promptInjectionSuspected: injectionPatterns.some(([, pattern]) => pattern.test(sourceText)),
        bounds: { x: Math.round(rect.x), y: Math.round(rect.y), width: Math.round(rect.width), height: Math.round(rect.height) }
      };
    }).filter(Boolean);
    const roots = delta ? [...changed].filter(node => node instanceof Element) : [document.body].filter(Boolean);
    const sensitive = [];
    const seen = new Set();
    for (const root of roots) {
      const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
      for (let node = walker.nextNode(); node && sensitive.length < 160; node = walker.nextNode()) {
        const element = node.parentElement;
        const sourceText = normalizedText(node.nodeValue);
        if (!element || !sourceText || !containsSensitiveText(sourceText) || !isVisible(element)) continue;
        const id = elementId(element);
        if (seen.has(id)) continue;
        seen.add(id);
        const rect = element.getBoundingClientRect();
        sensitive.push({
          id, role: roleOf(element), text: redactSensitiveText(sourceText), interactive: false,
          sensitiveText: true, contentBoundary: "untrusted_web_content",
          promptInjectionSuspected: injectionPatterns.some(([, pattern]) => pattern.test(sourceText)),
          bounds: { x: Math.round(rect.x), y: Math.round(rect.y), width: Math.round(rect.width), height: Math.round(rect.height) }
        });
      }
    }
    return [...new Map([...semantic, ...sensitive].map(region => [region.id, region])).values()]
      .sort((left, right) => Number(Boolean(right.sensitiveText)) - Number(Boolean(left.sensitiveText)))
      .slice(0, delta ? 160 : 240);
  }

  function snapshot(delta = false) {
    const elements = delta
      ? [...changed].flatMap(node => node instanceof Element ? [compactElement(node), ...[...node.querySelectorAll("a[href],button,input,select,textarea,[role=button],[role=link],[contenteditable=true],[tabindex]")].map(compactElement)] : []).filter(Boolean).slice(0, 300)
      : interactiveElements();
    const regions = semanticRegions(delta);
    const removed = [...removedIds];
    changed.clear(); removedIds.clear();
    revision += 1;
    const pageText = (document.body?.innerText ?? "").slice(0, 50000);
    const promptInjectionSignals = injectionPatterns.filter(([, pattern]) => pattern.test(pageText)).map(([id]) => id);
    const payload = { revision, pageGeneration, delta, url: location.href, title: redactSensitiveText(document.title), elements, regions, removedIds: removed, promptInjectionSignals, contentBoundary: "untrusted_web_content", viewport: { width: innerWidth, height: innerHeight, scrollX, scrollY } };
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

  async function execute(action, expectedPageGeneration) {
    const element = action.elementId ? target(action.elementId) : null;
    if (action.kind === "snapshot") return snapshot(Boolean(action.delta));
    if (expectedPageGeneration !== pageGeneration) throw new Error("The page changed after this action was authorized");
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

  const observer = new MutationObserver(records => {
    const relevant = records.filter(record => record.type !== "attributes" || ["type", "name", "autocomplete", "aria-label", "role", "href", "disabled", "contenteditable", "tabindex", "style", "class", "hidden"].includes(record.attributeName));
    if (!relevant.length) return;
    pageGeneration += 1;
    relevant.forEach(record => {
      if (record.target instanceof Element) changed.add(record.target);
      else if (record.target.parentElement) changed.add(record.target.parentElement);
      record.addedNodes.forEach(node => {
        if (!(node instanceof Element)) return;
        changed.add(node);
        [node, ...node.querySelectorAll("*")].forEach(element => { const id = ids.get(element); if (id) removedIds.delete(id); });
      });
      record.removedNodes.forEach(node => {
        if (!(node instanceof Element)) return;
        [node, ...node.querySelectorAll("*")].forEach(element => { const id = ids.get(element); if (id) removedIds.add(id); });
      });
    });
    // One callback can contain hundreds of records from a single render.
    // Invalidate once per batch so busy pages cannot flood the service worker.
    chrome.runtime.sendMessage({ type: "bridge-dom-dirty" }).catch(() => {});
  });
  observer.observe(document, { subtree: true, childList: true, attributes: true, characterData: true });
  const invalidateViewport = () => { pageGeneration += 1; chrome.runtime.sendMessage({ type: "bridge-dom-dirty", full: true }).catch(() => {}); };
  addEventListener("scroll", invalidateViewport, { passive: true });
  addEventListener("resize", invalidateViewport, { passive: true });

  chrome.runtime.onMessage.addListener((message, _sender, reply) => {
    if (message?.type !== "bridge-page-action") return;
    execute(message.action, message.expectedPageGeneration).then(result => reply({ ok: true, result })).catch(error => reply({ ok: false, error: error.message }));
    return true;
  });
})();

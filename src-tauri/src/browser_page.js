// Runs in the untrusted page. It has no Tauri permissions or IPC bridge.
// Rust reads the bounded result explicitly; page data is never an instruction.
(() => {
  if (window.__bridgeBrowser || window.top !== window) return;
  let revision = 0;
  let selection;
  let shortcut;
  let popupUrl;
  let historyAction;
  let cancelled = false;
  let inspecting = false;
  let highlighted;
  let outline;
  const bounded = (value, max) => String(value ?? '').slice(0, max);
  const sensitive = element => element.closest('input,textarea,select,[contenteditable]:not([contenteditable="false"]),[data-private],[data-sensitive],[autocomplete="current-password"],[autocomplete="new-password"]');
  const escape = value => CSS.escape(bounded(value, 160));
  function visibleText(element) {
    if (element.closest('[hidden],[aria-hidden="true"],script,style,noscript,template')) return false;
    // Checking the immediate node misses hidden wrappers and opacity on an
    // ancestor. Do not include content the user cannot see in the selection.
    for (let node = element; node; node = node.parentElement) {
      const style = getComputedStyle(node);
      if (style.display === 'none' || style.visibility === 'hidden' || style.visibility === 'collapse' || (style.opacity !== '' && Number(style.opacity) === 0)) return false;
    }
    return true;
  }
  function selectorFor(element) {
    const parts = [];
    for (let node = element; node && node.nodeType === 1 && parts.length < 7; node = node.parentElement) {
      if (node.id) {
        parts.unshift(`#${escape(node.id)}`);
        break;
      }
      let part = node.localName;
      if (node.parentElement) {
        const siblings = [...node.parentElement.children].filter(child => child.localName === node.localName);
        if (siblings.length > 1) part += `:nth-of-type(${siblings.indexOf(node) + 1})`;
      }
      parts.unshift(part);
    }
    return bounded(parts.join(' > '), 1024);
  }
  function snippetFor(element) {
    const encode = value => bounded(value, 240).replace(/[&<>"']/g, character => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[character]));
    if (sensitive(element)) return `<${element.localName}>[editable or private content omitted]</${element.localName}>`;
    // Never serialize outerHTML: even invisible descendants can hold credentials.
    const attributes = ['id', 'class', 'role', 'aria-label', 'data-testid']
      .filter(name => element.hasAttribute(name))
      .map(name => `${name}="${encode(element.getAttribute(name))}"`).join(' ');
    const walker = document.createTreeWalker(element, NodeFilter.SHOW_TEXT);
    let text = '';
    let visited = 0;
    while (walker.nextNode() && visited++ < 300 && text.length < 700) {
      const parent = walker.currentNode.parentElement;
      if (!parent || sensitive(parent) || !visibleText(parent)) continue;
      text += ` ${walker.currentNode.textContent ?? ''}`;
    }
    return bounded(`<${element.localName}${attributes ? ` ${attributes}` : ''}>${encode(text.replace(/\s+/g, ' ').trim())}</${element.localName}>`, 1600);
  }
  function removeOutline() {
    outline?.remove();
    outline = undefined;
    highlighted = undefined;
  }
  function setInspect(enabled) {
    inspecting = Boolean(enabled);
    selection = undefined;
    cancelled = false;
    removeOutline();
  }
  function draw(element) {
    if (!element || element === outline || !element.getBoundingClientRect) return;
    highlighted = element;
    const bounds = element.getBoundingClientRect();
    if (!outline) {
      outline = document.createElement('div');
      outline.setAttribute('data-bridge-picker-overlay', '');
      outline.setAttribute('aria-hidden', 'true');
      document.documentElement.appendChild(outline);
    }
    // This page script cannot use Bridge's Tailwind stylesheet. Every value is
    // runtime geometry; no product chrome or styles are injected into the app.
    outline.style.cssText = `position:fixed;pointer-events:none;z-index:2147483647;border:2px solid Highlight;background:transparent;box-sizing:border-box;left:${bounds.x}px;top:${bounds.y}px;width:${bounds.width}px;height:${bounds.height}px;`;
  }
  function routeChanged(action) {
    historyAction = action;
    revision += 1;
    selection = undefined;
    if (inspecting) { inspecting = false; cancelled = true; }
    removeOutline();
  }
  for (const method of ['pushState', 'replaceState']) {
    const original = history[method];
    history[method] = function (...args) {
      const result = Reflect.apply(original, this, args);
      routeChanged(method === 'replaceState' ? 'replace' : 'push');
      return result;
    };
  }
  addEventListener('popstate', () => routeChanged('traverse'));
  addEventListener('hashchange', () => routeChanged(historyAction === 'traverse' ? 'traverse' : 'push'));
  addEventListener('pointermove', event => {
    if (inspecting) draw(event.composedPath().find(node => node instanceof Element));
  }, true);
  addEventListener('scroll', () => { if (inspecting && highlighted) draw(highlighted); }, true);
  // Some controls activate on press/release rather than click. Keep inspection
  // observational throughout the gesture; the click handler still picks its node.
  for (const type of ['pointerdown', 'pointerup', 'mousedown', 'mouseup']) {
    addEventListener(type, event => {
      if (!inspecting || !event.isTrusted) return;
      event.preventDefault();
      event.stopImmediatePropagation();
    }, true);
  }
  addEventListener('click', event => {
    if (!event.isTrusted) return;
    if (!inspecting) {
      // WKWebView can suppress target=_blank before its new-window delegate is
      // called. Capture the user's anchor intent without creating an OS window.
      const anchor = event.composedPath().find(node => node instanceof HTMLAnchorElement);
      const target = anchor?.target || document.querySelector('base[target]')?.target;
      if (!anchor || anchor.hasAttribute('download') || (target?.toLowerCase() !== '_blank' && !event.metaKey && !event.ctrlKey)) return;
      let url;
      try { url = new URL(anchor.href); } catch { return; }
      if (!['http:', 'https:'].includes(url.protocol) || url.username || url.password) return;
      event.preventDefault();
      event.stopImmediatePropagation();
      popupUrl = bounded(url.href, 8192);
      return;
    }
    event.preventDefault();
    event.stopImmediatePropagation();
    const element = event.composedPath().find(node => node instanceof Element);
    if (!element || element === outline) return;
    const bounds = element.getBoundingClientRect();
    selection = { selector: selectorFor(element), snippet: snippetFor(element), bounds: { x: bounds.x, y: bounds.y, width: bounds.width, height: bounds.height } };
    inspecting = false;
    removeOutline();
  }, true);
  addEventListener('keydown', event => {
    if (!event.isTrusted) return;
    if ((event.metaKey || event.ctrlKey) && !event.altKey) {
      const key = event.key.toLowerCase();
      const command = key === 'l' && !event.shiftKey ? 'address' : key === 't' ? (event.shiftKey ? 'reopen_tab' : 'new_tab') : key === 'w' && !event.shiftKey ? 'close_tab' : undefined;
      if (command) { event.preventDefault(); event.stopImmediatePropagation(); shortcut = command; return; }
    }
    if (!inspecting || event.key !== 'Escape') return;
    event.preventDefault();
    event.stopImmediatePropagation();
    setInspect(false);
    cancelled = true;
  }, true);
  Object.defineProperty(window, '__bridgeBrowser', { value: Object.freeze({
    setInspect,
    snapshot() {
      const result = { url: bounded(location.href, 8192), title: bounded(document.title, 512), revision, selection, cancelled, shortcut, historyAction, popupUrl };
      selection = undefined;
      cancelled = false;
      shortcut = undefined;
      popupUrl = undefined;
      historyAction = undefined;
      return result;
    }
  }), configurable: false, writable: false });
})();

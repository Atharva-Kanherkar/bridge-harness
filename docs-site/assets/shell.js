// Shared chrome for every docs-site page: sidebar nav, icon sprite, theme
// toggle, mobile overlay, code-copy buttons, and TOC/pager auto-generated
// from each page's own content. Every page under pages/ is one level
// deeper than index.html, so every relative link this file builds accounts
// for that via pathPrefix() — a page file itself never hardcodes ../ or ./.

var NAV_GROUPS = [
  { label: 'Getting started', items: [
    { id: 'intro', label: 'Introduction' },
    { id: 'install', label: 'Install & first run' },
    { id: 'first-agent', label: 'Your first agent' },
  ]},
  { label: 'Core concepts', items: [
    { id: 'hierarchies', label: 'Three hierarchies' },
    { id: 'session-forest', label: 'Session forest' },
    { id: 'compaction', label: 'Compaction & resume' },
    { id: 'policy', label: 'Policy & delegation' },
  ]},
  { label: 'Agents & runtimes', items: [
    { id: 'claude-code', label: 'Claude Code' },
    { id: 'codex', label: 'Codex' },
    { id: 'opencode', label: 'OpenCode' },
    { id: 'managed-runtimes', label: 'Managed runtimes & the catalog' },
  ]},
  { label: 'Git & worktrees', items: [
    { id: 'worktrees', label: 'Worktree isolation' },
  ]},
  { label: 'Work, memory & learning', items: [
    { id: 'work-board', label: 'The work board' },
    { id: 'memory-and-learning', label: 'Memory & adaptive learning' },
  ]},
  { label: 'Prompts', items: [
    { id: 'prompt-studio', label: 'Prompt Studio & Context Lens' },
  ]},
  { label: 'Extensibility', items: [
    { id: 'marketplace', label: 'Marketplace & automations' },
    { id: 'browser-bridge', label: 'Authenticated browser bridge' },
  ]},
  { label: 'Reference', items: [
    { id: 'cli-exec', label: 'bridge exec --json' },
    { id: 'diagrams', label: 'Diagrams' },
    { id: 'protocol', label: 'Protocol / RPC' },
    { id: 'env-vars', label: 'Environment variables' },
  ]},
];

function flatten() {
  var out = [];
  NAV_GROUPS.forEach(function (g) { g.items.forEach(function (i) { out.push(i); }); });
  return out;
}
var FLAT = flatten();

function inPagesDir() { return window.location.pathname.indexOf('/pages/') !== -1; }
function pathPrefix() { return inPagesDir() ? '../' : ''; }
function hrefFor(id) {
  return id === 'intro' ? pathPrefix() + 'index.html' : pathPrefix() + 'pages/' + id + '.html';
}

var ICONS = {
  'i-sun': '<circle cx="12" cy="12" r="4"/><path d="M12 2v2.5M12 19.5V22M4.2 4.2l1.8 1.8M18 18l1.8 1.8M2 12h2.5M19.5 12H22M4.2 19.8L6 18M18 6l1.8-1.8"/>',
  'i-moon': '<path d="M20 13.2A8.5 8.5 0 1 1 10.8 4a6.6 6.6 0 0 0 9.2 9.2Z"/>',
  'i-search': '<circle cx="11" cy="11" r="7"/><path d="M21 21l-4.4-4.4"/>',
  'i-book': '<path d="M4 5.5A1.5 1.5 0 0 1 5.5 4H11v16H5.5A1.5 1.5 0 0 1 4 18.5Z"/><path d="M20 5.5A1.5 1.5 0 0 0 18.5 4H13v16h5.5a1.5 1.5 0 0 0 1.5-1.5Z"/>',
  'i-branch': '<circle cx="6" cy="5" r="2.1"/><circle cx="6" cy="19" r="2.1"/><circle cx="17.5" cy="12" r="2.1"/><path d="M6 7.1V16.9M6 12.5c0-2.7 2.4-3.6 5-3.7h2.6"/>',
  'i-shield': '<path d="M12 3l7 3v5c0 4.6-3 7.7-7 9-4-1.3-7-4.4-7-9V6Z"/>',
  'i-layers': '<path d="M12 3 21 8l-9 5-9-5Z"/><path d="M3 12l9 5 9-5"/><path d="M3 16l9 5 9-5"/>',
  'i-terminal': '<rect x="3" y="4" width="18" height="16" rx="2"/><path d="M7 9.5l3 2.5-3 2.5M13 15h4"/>',
  'i-cpu': '<rect x="6" y="6" width="12" height="12" rx="1.5"/><path d="M9 3v3M15 3v3M9 18v3M15 18v3M3 9h3M3 15h3M18 9h3M18 15h3"/>',
  'i-route': '<path d="M3 6h4l6 12h5"/><path d="M17 4l4 2-4 2M8 18H6a2 2 0 0 1 0-4h1"/>',
  'i-info': '<circle cx="12" cy="12" r="9"/><path d="M12 11v5.5M12 7.5v.01"/>',
  'i-chevron': '<path d="M9 5l7 7-7 7"/>',
  'i-copy': '<rect x="9" y="9" width="11" height="11" rx="2"/><path d="M5 15V6a2 2 0 0 1 2-2h9"/>',
  'i-check': '<path d="M4 12.5l5 5L20 6"/>',
  'i-menu': '<path d="M3 6h18M3 12h18M3 18h18"/>',
  'i-clock': '<circle cx="12" cy="12" r="9"/><path d="M12 7v5l3.5 2"/>',
  'i-brain': '<path d="M9 4.5a2.5 2.5 0 0 0-2.5 2.5v.2A3 3 0 0 0 5 10a3 3 0 0 0 1 5.6V16a3 3 0 0 0 3 3h.5"/><path d="M15 4.5a2.5 2.5 0 0 1 2.5 2.5v.2A3 3 0 0 1 19 10a3 3 0 0 1-1 5.6V16a3 3 0 0 1-3 3h-.5"/><path d="M9.5 4.7V18a1.5 1.5 0 0 0 3 0V4.7"/>',
  'i-package': '<path d="M12 3 4 7v10l8 4 8-4V7Z"/><path d="M4 7l8 4 8-4M12 11v10"/>',
  'i-sliders': '<path d="M4 6h9M17 6h3M4 12h3M11 12h9M4 18h13M21 18h-1"/><circle cx="15" cy="6" r="2"/><circle cx="7" cy="12" r="2"/><circle cx="19" cy="18" r="2"/>',
  'i-globe': '<circle cx="12" cy="12" r="9"/><path d="M3 12h18M12 3a14 14 0 0 1 0 18M12 3a14 14 0 0 0 0 18"/>',
};

function buildIconSprite() {
  var svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
  svg.setAttribute('style', 'position:absolute;width:0;height:0');
  svg.setAttribute('aria-hidden', 'true');
  var markup = Object.keys(ICONS).map(function (id) {
    return '<symbol id="' + id + '" viewBox="0 0 24 24">' + ICONS[id] + '</symbol>';
  }).join('');
  svg.innerHTML = markup;
  document.body.insertBefore(svg, document.body.firstChild);
}

function icon(id, size) {
  return '<svg class="icon" style="width:' + (size || 15) + 'px;height:' + (size || 15) + 'px"><use href="#' + id + '"></use></svg>';
}

function footerTag() {
  var isLocal = ['localhost', '127.0.0.1', ''].indexOf(window.location.hostname) !== -1;
  return isLocal ? 'BRIDGE DOCS &mdash; RUNS LOCALLY' : 'BRIDGE DOCS &mdash; PUBLISHED FROM A LOCAL-FIRST APP';
}

function buildSidebar() {
  var mount = document.getElementById('sidebar');
  if (!mount) return;
  var current = document.body.dataset.page;
  var navHtml = NAV_GROUPS.map(function (g) {
    var items = g.items.map(function (item) {
      var active = item.id === current;
      return '<a class="nav-link' + (active ? ' is-active' : '') + '" href="' + hrefFor(item.id) + '"' +
        (active ? ' aria-current="page"' : '') + '>' + item.label + '</a>';
    }).join('');
    return '<div class="nav-group"><div class="nav-group-label">' + g.label + '</div>' + items + '</div>';
  }).join('');

  mount.innerHTML =
    '<div class="sidebar-head">' +
      '<div class="brand-row">' +
        '<span class="brand-name">Bridge</span><span class="brand-tag">docs</span><span class="brand-version">0.1.0</span>' +
      '</div>' +
      '<div class="theme-seg" role="tablist" aria-label="Theme">' +
        '<button class="theme-seg-item" data-theme-btn="light" role="tab">' + icon('i-sun') + 'Light</button>' +
        '<button class="theme-seg-item" data-theme-btn="dark" role="tab">' + icon('i-moon') + 'Dark</button>' +
      '</div>' +
      '<button class="search-pill" type="button">' + icon('i-search') + '<span>Search docs&hellip;</span><kbd>&#8984;K</kbd></button>' +
    '</div>' +
    '<nav class="sidebar-scroll" aria-label="Docs navigation">' + navHtml + '</nav>' +
    '<div class="sidebar-foot"><span>' + footerTag() + '</span></div>' +
    '<div class="sidebar-resize" id="sidebarResize" role="separator" aria-orientation="vertical" aria-label="Resize sidebar"></div>';
}

var SIDEBAR_WIDTH_KEY = 'bridge-docs-sidebar-width';
var SIDEBAR_MIN = 220;
var SIDEBAR_MAX = 420;
var SIDEBAR_DEFAULT = 272;

function applySidebarWidth(px) {
  document.getElementById('sidebar').style.setProperty('--sidebar-width', px + 'px');
}

function initSidebarResize() {
  var sidebar = document.getElementById('sidebar');
  var handle = document.getElementById('sidebarResize');
  if (!sidebar || !handle) return;

  var stored = null;
  try { stored = parseInt(localStorage.getItem(SIDEBAR_WIDTH_KEY), 10); } catch (e) {}
  var initial = (stored && stored >= SIDEBAR_MIN && stored <= SIDEBAR_MAX) ? stored : SIDEBAR_DEFAULT;
  applySidebarWidth(initial);

  var dragging = false;
  var startX = 0;
  var startWidth = initial;

  handle.addEventListener('mousedown', function (e) {
    dragging = true;
    startX = e.clientX;
    startWidth = sidebar.getBoundingClientRect().width;
    handle.classList.add('is-dragging');
    document.body.classList.add('is-resizing-sidebar');
    e.preventDefault();
  });

  window.addEventListener('mousemove', function (e) {
    if (!dragging) return;
    var next = Math.min(SIDEBAR_MAX, Math.max(SIDEBAR_MIN, startWidth + (e.clientX - startX)));
    applySidebarWidth(next);
  });

  window.addEventListener('mouseup', function () {
    if (!dragging) return;
    dragging = false;
    handle.classList.remove('is-dragging');
    document.body.classList.remove('is-resizing-sidebar');
    var current = sidebar.getBoundingClientRect().width;
    try { localStorage.setItem(SIDEBAR_WIDTH_KEY, String(Math.round(current))); } catch (e) {}
  });

  handle.addEventListener('dblclick', function () {
    applySidebarWidth(SIDEBAR_DEFAULT);
    try { localStorage.setItem(SIDEBAR_WIDTH_KEY, String(SIDEBAR_DEFAULT)); } catch (e) {}
  });
}

function buildToc() {
  var mount = document.getElementById('tocMount');
  var grid = document.querySelector('.page-grid');
  if (!mount || !grid) return;
  var headings = document.querySelectorAll('.prose h2[id]');
  if (headings.length === 0) {
    grid.classList.add('no-toc');
    mount.remove();
    return;
  }
  var links = Array.prototype.map.call(headings, function (h) {
    return '<a href="#' + h.id + '">' + h.textContent + '</a>';
  }).join('');
  mount.innerHTML = '<div class="toc-label">On this page</div>' + links;
}

function buildPager() {
  var mount = document.getElementById('pagerMount');
  if (!mount) return;
  var current = document.body.dataset.page;
  var i = FLAT.map(function (n) { return n.id; }).indexOf(current);
  var prev = FLAT[i - 1], next = FLAT[i + 1];
  var left = prev
    ? '<a class="pager-link prev" href="' + hrefFor(prev.id) + '">' + icon('i-chevron') + '<span><em>Previous</em>' + prev.label + '</span></a>'
    : '<span></span>';
  var right = next
    ? '<a class="pager-link next" href="' + hrefFor(next.id) + '"><span style="text-align:right"><em>Next</em>' + next.label + '</span>' + icon('i-chevron').replace('class="icon"', 'class="icon icon-flip"') + '</a>'
    : '<span></span>';
  mount.outerHTML = '<nav class="pager">' + left + right + '</nav>';
}

var THEME_KEY = 'bridge-docs-theme';
function setTheme(mode, persist) {
  document.documentElement.setAttribute('data-bridge-theme', mode);
  document.querySelectorAll('[data-theme-btn]').forEach(function (b) {
    var on = b.dataset.themeBtn === mode;
    b.classList.toggle('is-active', on);
    b.setAttribute('aria-selected', on ? 'true' : 'false');
  });
  if (persist) { try { localStorage.setItem(THEME_KEY, mode); } catch (e) {} }
}

function closeMobileNav() {
  var sidebar = document.getElementById('sidebar');
  var scrim = document.getElementById('scrim');
  if (sidebar) sidebar.classList.remove('is-open');
  if (scrim) scrim.classList.remove('is-open');
}

document.addEventListener('click', function (e) {
  var themeBtn = e.target.closest('[data-theme-btn]');
  if (themeBtn) { setTheme(themeBtn.dataset.themeBtn, true); return; }

  var copyBtn = e.target.closest('.code-copy');
  if (copyBtn) {
    var codeEl = copyBtn.closest('.code-block').querySelector('code');
    var text = codeEl.innerText;
    if (navigator.clipboard && navigator.clipboard.writeText) {
      navigator.clipboard.writeText(text).catch(function () {});
    }
    var label = copyBtn.querySelector('.copy-label');
    var original = label.textContent;
    label.textContent = 'Copied';
    copyBtn.classList.add('is-copied');
    setTimeout(function () { label.textContent = original; copyBtn.classList.remove('is-copied'); }, 1400);
    return;
  }

  if (e.target.closest('#menuBtn')) {
    document.getElementById('sidebar').classList.add('is-open');
    document.getElementById('scrim').classList.add('is-open');
    return;
  }
  if (e.target.closest('#scrim')) { closeMobileNav(); return; }
});

buildIconSprite();
buildSidebar();
buildToc();
buildPager();
initSidebarResize();

var storedTheme = null;
try { storedTheme = localStorage.getItem(THEME_KEY); } catch (e) {}
var initial = storedTheme || (window.matchMedia && window.matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light');
setTheme(initial, false);

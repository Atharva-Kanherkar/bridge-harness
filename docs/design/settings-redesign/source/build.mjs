// Generates the Settings redesign artboards (.dc.html) + canvas.json.
// Every value below is lifted from src/index.css (Graphite tokens) and the
// current component source; nothing is snapped to a grid.
import { writeFileSync, mkdirSync } from "node:fs";

const OUT = "canvas";
mkdirSync(OUT, { recursive: true });

// ── Tokens (dark · graphite) ─────────────────────────────────────────────
const T = {
  bg: "#000000", card: "#0f0f0f", popover: "#161616", muted: "#1c1c1c", code: "#0a0a0a",
  fg: "#e4e4e4", body: "#c9c9c9", mutedFg: "#8b8b8b", faint: "#6b6b6b", faint2: "#4a4a4a",
  border: "#1e1e1e", borderCard: "#242424", input: "#242424",
  ring: "#6a9bcc", success: "#5db872", warning: "#d4a957", info: "#6a9bcc", destructive: "#e5695e",
  claude: "#d97757", codex: "#4bc39c", opencode: "#9b8cf0",
};

const CSS = `
  x-dc, helmet { display: block; }
  * { box-sizing: border-box; }
  body { margin: 0; background: ${T.bg}; color: ${T.fg}; font-family: "Geist", "Geist Variable", ui-sans-serif, system-ui, -apple-system, sans-serif; -webkit-font-smoothing: antialiased; font-size: 13px; line-height: 1.35; }
  a { color: ${T.info}; } a:hover { color: ${T.fg}; }
  .mono { font-family: "Geist Mono", ui-monospace, SFMono-Regular, Menlo, monospace; }
  .app { display: flex; width: 1440px; background: ${T.bg}; }
  /* App sidebar (BridgeSidebar) */
  .sb { width: 248px; flex: none; background: ${T.bg}; border-right: 1px solid ${T.border}; display: flex; flex-direction: column; padding: 14px 10px 10px; gap: 10px; }
  .sb-top { display: flex; align-items: center; justify-content: space-between; padding: 0 6px; color: ${T.mutedFg}; height: 22px; }
  .sb-search { display: flex; gap: 6px; }
  .sb-search .f { flex: 1; height: 30px; border: 1px solid ${T.borderCard}; background: ${T.card}; border-radius: 9px; display: flex; align-items: center; gap: 8px; padding: 0 10px; color: ${T.faint}; font-size: 12.5px; }
  .sb-search .b { width: 30px; height: 30px; border: 1px solid ${T.borderCard}; background: ${T.card}; border-radius: 9px; display: grid; place-items: center; color: ${T.mutedFg}; }
  .sb-item { display: flex; align-items: center; gap: 9px; height: 28px; padding: 0 8px; border-radius: 8px; color: ${T.body}; font-size: 12.5px; }
  .sb-item svg { color: ${T.mutedFg}; }
  .sb-sec { display: flex; align-items: center; justify-content: space-between; padding: 0 8px; margin-top: 8px; color: ${T.mutedFg}; font-size: 12.5px; }
  .sb-repo { display: flex; align-items: center; gap: 8px; padding: 0 8px; height: 26px; color: ${T.body}; font-size: 12px; }
  .sb-sess { padding: 2px 8px 6px 24px; font-size: 12px; color: ${T.body}; display: flex; justify-content: space-between; }
  .sb-sess small { display: block; color: ${T.faint}; font-size: 11px; margin-top: 2px; }
  .sb-bottom { margin-top: auto; display: flex; gap: 4px; padding-top: 8px; border-top: 1px solid ${T.border}; }
  .sb-bottom span { width: 32px; height: 32px; border-radius: 9px; display: grid; place-items: center; color: ${T.mutedFg}; }
  .sb-bottom span.on { background: ${T.muted}; color: ${T.fg}; }
  /* Settings rail */
  .rail { width: 208px; flex: none; border-right: 1px solid ${T.border}; padding: 18px 12px 14px; display: flex; flex-direction: column; }
  .rail h1 { margin: 0 8px 12px; font-size: 15px; font-weight: 600; letter-spacing: -0.01em; color: ${T.fg}; }
  .rail .search { height: 28px; border: 1px solid ${T.borderCard}; background: ${T.card}; border-radius: 8px; display: flex; align-items: center; gap: 7px; padding: 0 9px; color: ${T.faint}; font-size: 12px; margin-bottom: 14px; }
  .rail .grp { font-size: 10.5px; font-weight: 600; letter-spacing: 0.08em; text-transform: uppercase; color: ${T.faint}; padding: 0 8px; margin: 10px 0 4px; }
  .rail .it { display: flex; align-items: center; gap: 9px; height: 30px; padding: 0 8px; border-radius: 8px; color: ${T.mutedFg}; font-size: 13px; }
  .rail .it svg { flex: none; }
  .rail .it.on { background: rgba(228,228,228,0.08); color: ${T.fg}; }
  .rail .foot { margin-top: auto; padding: 0 8px; font-size: 11.5px; color: ${T.faint}; display: flex; align-items: center; gap: 6px; }
  /* Content column */
  .main { flex: 1; min-width: 0; padding: 30px 40px 40px; overflow: hidden; }
  .col { max-width: 720px; margin: 0 auto; position: relative; }
  .crumbs { display: flex; align-items: center; gap: 6px; font-size: 12px; color: ${T.mutedFg}; margin-bottom: 14px; }
  .crumbs b { color: ${T.fg}; font-weight: 500; }
  .crumbs .sep { color: ${T.faint2}; }
  .ph { display: flex; align-items: flex-start; justify-content: space-between; gap: 16px; }
  .ph h2 { margin: 0; font-size: 17px; font-weight: 600; letter-spacing: -0.012em; color: ${T.fg}; }
  .ph p { margin: 4px 0 0; font-size: 12px; color: ${T.mutedFg}; max-width: 560px; line-height: 1.45; }
  .ph .act { display: flex; gap: 8px; align-items: center; flex: none; padding-top: 2px; }
  .g { margin-top: 26px; }
  .g-h { display: flex; align-items: baseline; justify-content: space-between; margin: 0 2px 8px; }
  .g-h h3 { margin: 0; font-size: 11px; font-weight: 600; letter-spacing: 0.06em; text-transform: uppercase; color: ${T.mutedFg}; }
  .g-h span { font-size: 11.5px; color: ${T.faint}; }
  .card { background: ${T.card}; border: 1px solid ${T.borderCard}; border-radius: 12px; overflow: hidden; }
  .row { display: flex; align-items: center; gap: 16px; padding: 10px 14px; min-height: 44px; }
  .row + .row, .row + .editor, .editor + .row { border-top: 1px solid ${T.border}; }
  .row .l { flex: 1; min-width: 0; }
  .row .t { font-size: 13px; color: ${T.fg}; display: flex; align-items: center; gap: 8px; }
  .row .d { font-size: 11.5px; color: ${T.mutedFg}; margin-top: 2px; line-height: 1.4; }
  .row .d.mono { font-size: 10.5px; }
  .row .c { display: flex; align-items: center; gap: 8px; flex: none; }
  .row .ico { width: 28px; height: 28px; border-radius: 8px; display: grid; place-items: center; background: ${T.popover}; flex: none; }
  .chev { color: ${T.faint2}; display: inline-flex; }
  /* Controls */
  .sw { width: 32px; height: 18px; border-radius: 9px; background: ${T.input}; position: relative; flex: none; }
  .sw::after { content: ""; position: absolute; top: 2px; left: 2px; width: 14px; height: 14px; border-radius: 7px; background: ${T.mutedFg}; }
  .sw.on { background: ${T.fg}; } .sw.on::after { left: 16px; background: ${T.bg}; }
  .sel, .fld { height: 28px; border: 1px solid ${T.borderCard}; background: ${T.popover}; border-radius: 8px; font-size: 12px; color: ${T.fg}; display: inline-flex; align-items: center; padding: 0 10px; white-space: nowrap; }
  .sel { padding-right: 26px; position: relative; gap: 7px; } .sel > .cv { position: absolute; right: 8px; color: ${T.mutedFg}; }
  .sel.dim, .fld.dim { color: ${T.faint}; }
  .fld.ph { color: ${T.faint}; display: inline-flex; }
  .btn { height: 28px; border-radius: 8px; padding: 0 10px; font-size: 12px; font-weight: 500; display: inline-flex; align-items: center; gap: 6px; border: 1px solid transparent; white-space: nowrap; }
  .btn.pri { background: ${T.fg}; color: ${T.bg}; }
  .btn.gho { border-color: ${T.borderCard}; color: ${T.body}; background: transparent; }
  .btn.des { color: ${T.destructive}; }
  .btn.qui { color: ${T.mutedFg}; }
  .pill { font-size: 10.5px; font-weight: 500; letter-spacing: 0.01em; padding: 2px 7px; border-radius: 999px; border: 1px solid ${T.borderCard}; color: ${T.mutedFg}; display: inline-flex; align-items: center; gap: 5px; line-height: 1.3; }
  .pill i { width: 6px; height: 6px; border-radius: 3px; background: currentColor; display: inline-block; }
  .pill.ok { color: ${T.success}; border-color: rgba(93,184,114,0.35); }
  .pill.warn { color: ${T.warning}; border-color: rgba(212,169,87,0.35); }
  .pill.info { color: ${T.info}; border-color: rgba(106,155,204,0.35); }
  .pill.solid { background: ${T.popover}; border-color: transparent; color: ${T.mutedFg}; }
  .editor { background: ${T.code}; }
  .editor .bar { height: 30px; display: flex; align-items: center; gap: 10px; padding: 0 14px; border-bottom: 1px solid ${T.border}; font-size: 10.5px; color: ${T.mutedFg}; }
  .editor pre { margin: 0; padding: 12px 14px; font-size: 12px; line-height: 1.6; color: ${T.body}; white-space: pre-wrap; min-height: 96px; }
  .editor pre .ln { color: ${T.faint2}; display: inline-block; width: 22px; }
  .savebar { position: absolute; left: 0; right: 0; bottom: 0; display: flex; align-items: center; gap: 8px; padding: 10px 14px; background: ${T.popover}; border: 1px solid ${T.borderCard}; border-radius: 12px; box-shadow: 0 16px 40px -18px rgba(0,0,0,0.45), 0 2px 8px -4px rgba(0,0,0,0.22); font-size: 12px; color: ${T.mutedFg}; }
  .savebar .sp { flex: 1; }
  .savebar .dot { width: 6px; height: 6px; border-radius: 3px; background: ${T.info}; }
  .tiles { display: grid; grid-template-columns: repeat(3, minmax(0, 1fr)); gap: 10px; }
  .tiles.two { grid-template-columns: repeat(2, minmax(0, 1fr)); }
  .tile { border: 1px solid ${T.borderCard}; background: ${T.card}; border-radius: 12px; padding: 10px; }
  .tile.on { border-color: ${T.fg}; }
  .tile .prev { height: 76px; border-radius: 8px; overflow: hidden; display: flex; border: 1px solid ${T.border}; }
  .tile .cap { display: flex; align-items: center; justify-content: space-between; margin-top: 10px; padding: 0 2px; }
  .tile .cap b { font-size: 12.5px; font-weight: 500; color: ${T.fg}; display: block; }
  .tile .cap span { font-size: 11px; color: ${T.mutedFg}; }
  .radio { width: 14px; height: 14px; border-radius: 7px; border: 1.5px solid ${T.faint2}; }
  .radio.on { border-color: ${T.fg}; background: radial-gradient(circle, ${T.fg} 0 3px, transparent 3.5px); }
  .kv { display: grid; grid-template-columns: 88px 1fr; gap: 4px 12px; font-size: 12px; color: ${T.mutedFg}; }
  .kv b { font-weight: 400; color: ${T.body}; }
  .list-item { display: flex; align-items: center; gap: 12px; }
  .badge-dot { width: 6px; height: 6px; border-radius: 3px; background: ${T.warning}; display: inline-block; }
  .stack { display: flex; flex-direction: column; }
  .hr { height: 1px; background: ${T.border}; }
  .muted { color: ${T.mutedFg}; } .faint { color: ${T.faint}; }
  .sheet { padding: 40px; width: 1440px; }
  .sheet h2 { font-size: 22px; margin: 0 0 6px; font-weight: 600; letter-spacing: -0.014em; }
  .sheet .lede { color: ${T.mutedFg}; font-size: 13px; max-width: 720px; line-height: 1.5; margin: 0 0 28px; }
  .sheet .sec { display: grid; grid-template-columns: 200px 1fr; gap: 24px; padding: 22px 0; border-top: 1px solid ${T.border}; }
  .sheet .sec h4 { margin: 0; font-size: 13px; font-weight: 600; color: ${T.fg}; }
  .sheet .sec p { margin: 4px 0 0; font-size: 12px; color: ${T.mutedFg}; line-height: 1.5; }
  .sheet .demo { display: flex; flex-wrap: wrap; gap: 14px; align-items: center; }
  .spec { font-size: 10.5px; color: ${T.faint}; }
  .before { background: ${T.bg}; padding: 0; }
  .before img { display: block; width: 1200px; height: auto; }
`;

// ── Icons: pulled from a real library (see iconlibs/) ───────────────────
import { makeIcons, SET_NAMES } from "./iconlibs/icons.mjs";
const SET = process.env.ICONS ?? "phosphor";
const I = makeIcons(SET);

// Harness marks: the real brand marks. OpenAI and Claude from simple-icons;
// Cursor and OpenCode rebuilt to the exact geometry of the reference logos
// (faceted cube, two-tone inner block). harnessMarks.tsx's radial stand-ins go.
import { readFileSync } from "node:fs";
import { CURSOR, OPENCODE } from "./logos/exact.mjs";
const LOGO_INNER = name => readFileSync(`logos/${name}.svg`, "utf8").replace(/^[\s\S]*?<svg[^>]*>/, "").replace(/<title>.*?<\/title>/, "").replace(/<\/svg>\s*$/, "").trim();
const LOGOS = { claude: LOGO_INNER("claude"), codex: LOGO_INNER("openai") };
function mark(h, size = 16) {
  if (h === "cursor") return `<svg width="${size}" height="${size}" viewBox="0 0 24 24" aria-hidden="true">${CURSOR()}</svg>`;
  if (h === "opencode") return `<svg width="${size}" height="${size}" viewBox="0 0 24 24" aria-hidden="true">${OPENCODE(T.opencode, "rgba(155,140,240,0.45)")}</svg>`;
  const color = { claude: T.claude, codex: T.fg }[h] ?? T.mutedFg;
  const inner = LOGOS[h] ?? `<path d="M12 3.4A8.6 8.6 0 1 1 3.4 12" stroke="${color}" stroke-width="2.2" stroke-linecap="round" fill="none"></path>`;
  return `<svg width="${size}" height="${size}" viewBox="0 0 24 24" fill="${color}" aria-hidden="true">${inner}</svg>`;
}

// ── Chrome ───────────────────────────────────────────────────────────────
function sidebar() {
  return `<aside class="sb">
    <div class="sb-top">${I.panel(15)}<span style="display:flex;gap:10px">${I.chevL(13)}${I.chevR(13)}</span></div>
    <div class="sb-search"><div class="f">${I.search(13)}<span>Search</span></div><div class="b">${I.edit(13)}</div></div>
    <div class="stack" style="gap:2px">
      <div class="sb-item">${I.store(14)}<span>Marketplace</span></div>
      <div class="sb-item">${I.projects(14)}<span>Projects</span></div>
      <div class="sb-item">${I.memory(14)}<span>Memory</span></div>
    </div>
    <div class="sb-sec"><span style="display:flex;gap:8px;align-items:center">${I.folder(13)}Repositories</span><span style="display:flex;gap:10px">${I.filter(12)}${I.plus(12)}</span></div>
    <div class="stack">
      <div class="sb-repo">${I.chevD(11)}${I.folder(13)}<span style="flex:1">Build session supervisor</span><span class="faint" style="font-size:11px">1</span></div>
      <div class="sb-sess"><span>Orchestrator<small><span style="display:inline-block;width:5px;height:5px;border-radius:3px;background:${T.success};margin-right:5px"></span>now</small></span>${mark("codex", 12)}</div>
      <div class="sb-repo">${I.chevD(11)}${I.folder(13)}<span style="flex:1">Polish the Deck shell</span><span class="faint" style="font-size:11px">1</span></div>
      <div class="sb-sess"><span>Orchestrator<small>now</small></span>${mark("codex", 12)}</div>
    </div>
    <div class="sb-bottom"><span class="on">${I.sliders(15)}</span><span>${I.branch(15)}</span><span>${I.chart(15)}</span><span>${I.refresh(15)}</span></div>
  </aside>`;
}

const NAV = [
  ["General", [["appearance", "Appearance", I.sun], ["permissions", "Permissions", I.shield], ["composer", "Composer", I.keyboard]]],
  ["Agents", [["presets", "Presets", I.bot], ["models", "Models", I.sliders], ["prompts", "Prompts", I.scroll]]],
  ["Runtimes", [["harnesses", "Harnesses", I.code]]],
  ["Data", [["work", "Work briefing", I.sparkles], ["import", "Import", I.download]]],
];
function rail(active) {
  return `<nav class="rail">
    <h1>Settings</h1>
    <div class="search">${I.search(12)}<span>Search settings</span></div>
    ${NAV.map(([g, items]) => `<div class="grp">${g}</div>${items.map(([id, label, icon]) => `<div class="it${id === active ? " on" : ""}">${icon(14)}<span>${label}</span></div>`).join("")}`).join("")}
    <div class="foot">${I.rotate(12)}Reset all settings</div>
  </nav>`;
}

function frame({ active, body, height = 900 }) {
  return `<!doctype html>
<html>
<head>
  <meta charset="utf-8">
  <script src="./support.js"></script>
</head>
<body>
<x-dc>
<helmet>
  <link rel="stylesheet" href="https://fonts.googleapis.com/css2?family=Geist:wght@400;500;600&family=Geist+Mono:wght@400;500&display=swap">
  <style>${CSS}</style>
</helmet>
<div class="app" style="height:${height}px">
  ${sidebar()}
  ${rail(active)}
  <main class="main"><div class="col" style="min-height:${height - 70}px">${body}</div></main>
</div>
</x-dc>
</body>
</html>`;
}

// ── Components ───────────────────────────────────────────────────────────
const sw = on => `<span class="sw${on ? " on" : ""}"></span>`;
const sel = (v, w, dim) => `<span class="sel${dim ? " dim" : ""}"${w ? ` style="width:${w}px"` : ""}>${v}${I.chevD(12).replace("<svg", "<svg class=\"cv\"")}</span>`;
const fld = (v, w, cls = "") => `<span class="fld ${cls}"${w ? ` style="width:${w}px"` : ""}>${v}</span>`;
const btn = (t, v = "gho", icon = "") => `<span class="btn ${v}">${icon}${t}</span>`;
const pill = (t, v = "", dot = false) => `<span class="pill ${v}">${dot ? "<i></i>" : ""}${t}</span>`;
const chev = () => `<span class="chev">${I.chevR(14)}</span>`;
const row = (t, d, c, opts = {}) => `<div class="row">${opts.lead ?? ""}<div class="l"><div class="t">${t}</div>${d ? `<div class="d${opts.mono ? " mono" : ""}">${d}</div>` : ""}</div><div class="c">${c}</div></div>`;
const group = (h, rows, right = "") => `<section class="g">${h ? `<div class="g-h"><h3>${h}</h3>${right ? `<span>${right}</span>` : ""}</div>` : ""}<div class="card">${rows.join("")}</div></section>`;
const header = (t, d, act = "") => `<div class="ph"><div><h2>${t}</h2>${d ? `<p>${d}</p>` : ""}</div>${act ? `<div class="act">${act}</div>` : ""}</div>`;
const crumbs = parts => `<div class="crumbs">${parts.map((p, i) => i === parts.length - 1 ? `<b>${p}</b>` : `<span>${p}</span><span class="sep">/</span>`).join("")}</div>`;
const editor = (name, lines, right = "") => `<div class="editor"><div class="bar mono"><span>${name}</span><span style="flex:1"></span>${right}</div><pre class="mono">${lines.map((l, i) => `<span class="ln">${i + 1}</span>${l}`).join("\n")}</pre></div>`;
const savebar = (t = "Unsaved changes") => `<div class="savebar"><span class="dot"></span><span>${t}</span><span class="sp"></span>${btn("Discard", "qui")}${btn("Save", "pri")}</div>`;
const icoBox = svg => `<span class="ico">${svg}</span>`;

// ── Pages ────────────────────────────────────────────────────────────────
const pages = {};

pages.Harnesses = frame({ active: "harnesses", body: `
  ${header("Harnesses", "The agent runtimes Bridge can run. Install from each vendor's official source, or point Bridge at a copy you already have.", "")}
  ${group("Installed", [
    row("Claude Code", "Bridge-managed · 0.3.209", pill("Ready", "ok", true) + chev(), { lead: icoBox(mark("claude", 16)) }),
    row("Codex", "Your own install on PATH · 0.147.0", pill("Working", "ok", true) + chev(), { lead: icoBox(mark("codex", 16)) }),
    row("Cursor", "Your own install on PATH", pill("Working", "ok", true) + chev(), { lead: icoBox(mark("cursor", 15)) }),
  ])}
  ${group("Available", [
    row("OpenCode", "Not installed", btn("Install", "gho") + chev(), { lead: icoBox(mark("opencode", 16)) }),
  ])}
  ${group("Defaults", [
    row("Bridge", "The built-in orchestrator. Routes each role to a runtime above.", pill("Always on", "solid") + chev(), { lead: icoBox(I.sliders(15)) }),
  ])}
` });

pages.HarnessDetail = frame({ active: "harnesses", height: 1180, body: `
  ${crumbs(["Harnesses", "OpenCode"])}
  ${header(`<span style="display:inline-flex;align-items:center;gap:10px">${mark("opencode", 18)}OpenCode</span>`, "Bridge-managed 1.8.3 · <span class=\"mono\" style=\"font-size:11px\">/usr/local/bin/opencode</span>", pill("Ready", "ok", true) + btn("Remove", "des"))}
  ${group("Sessions", [
    row("Enabled", "New sessions may start on OpenCode.", sw(true)),
    row("Default model", "Used when a role profile says Automatic.", sel("Kimi K2.5", 220)),
    row("Default effort", "", sel("Role default", 220)),
  ])}
  ${group("System prompt", [
    editor("opencode.md", ["Prefer small, reviewable diffs. Ask before touching migrations."], `<span style="display:flex;align-items:center;gap:6px"><span class="dot" style="width:6px;height:6px;border-radius:3px;background:${T.info};display:inline-block"></span>Unsaved</span>`),
  ], "Appended to every OpenCode session")}
  ${group("Providers", [
    row("OpenCode Go", "1 model · api", pill("Connected", "ok", true) + btn("Disconnect", "qui", I.unplug(12)), { lead: icoBox(mark("opencode", 14)) }),
    row("Anthropic", "Key goes straight to OpenCode's credential store. Bridge never keeps it.", fld("API key", 200, "ph") + btn("Connect", "gho", I.key(12)), { lead: icoBox(mark("claude", 14)) }),
    row("OpenAI", "Sign in with OpenCode's own flow, then refresh.", btn("Refresh", "qui", I.refresh(12)), { lead: icoBox(mark("codex", 14)) }),
  ], "Read from OpenCode")}
  ${group("Visible models", [
    row("Kimi K2.5", "opencode-go/kimi-k2.5", sw(true), { mono: true }),
    row("Kimi K2 Thinking", "opencode-go/kimi-k2-thinking", sw(false), { mono: true }),
  ], "All connected models when none is picked")}
  ${group("Advanced", [
    row("Executable", "Overrides the managed copy and PATH.", fld("/path/to/opencode", 260, "ph mono")),
  ])}
  ${savebar()}
` });

pages.Models = frame({ active: "models", height: 1040, body: `
  ${header("Models", "Which model each Bridge role runs on. Tracked roles follow the standard as it moves; pinned roles stay put.", pill("Version 1", "solid"))}
  ${group("Orchestration", [
    row("Standard orchestrator", `<span style="display:inline-flex;align-items:center;gap:6px">${mark("codex", 11)}Codex · GPT Terra · medium</span>`, pill("Tracks standard", "solid") + `<span class="chev">${I.chevD(14)}</span>`),
    `<div class="row" style="padding-top:0;border-top:0;align-items:stretch"><div style="flex:1;margin-left:0;padding:4px 0 8px;display:grid;grid-template-columns:repeat(2, minmax(0,1fr));gap:8px 24px">
      ${[["Selection", sel("Track standard", 200)], ["Provider and model", sel("Codex · GPT Terra", 200, true)], ["Reasoning effort", sel("medium", 200)], ["Fallback profile", sel("Catalog default", 200)], ["Budget", sel("Balanced", 200)], ["Latency", sel("Balanced", 200)], ["Allow learning", sw(true)]].map(([l, c]) => `<div style="display:flex;align-items:center;justify-content:space-between;gap:12px;min-height:36px"><span style="font-size:12.5px;color:${T.body}">${l}</span>${c}</div>`).join("")}
    </div></div>`,
    row("Premium orchestrator", `<span style="display:inline-flex;align-items:center;gap:6px">${mark("codex", 11)}Codex · GPT Sol · high</span>`, pill("Tracks standard", "solid") + chev()),
    row("Planner", `<span style="display:inline-flex;align-items:center;gap:6px">${mark("codex", 11)}Codex · GPT Sol · high</span>`, pill("Tracks standard", "solid") + chev()),
  ])}
  ${group("Workers", [
    row("Implementer", `<span style="display:inline-flex;align-items:center;gap:6px">${mark("claude", 11)}Claude Code · Opus · high</span>`, pill("Pinned", "info") + chev()),
    row("Researcher", `<span style="display:inline-flex;align-items:center;gap:6px">${mark("codex", 11)}Codex · GPT Terra · medium</span>`, pill("Tracks standard", "solid") + chev()),
    row("Documentation", `<span style="display:inline-flex;align-items:center;gap:6px">${mark("codex", 11)}Codex · GPT Luna · low</span>`, pill("Tracks standard", "solid") + chev()),
  ])}
  ${group("Verification", [
    row("Reviewer", `<span style="display:inline-flex;align-items:center;gap:6px">${mark("codex", 11)}Codex · GPT Sol · high</span>`, pill("Tracks standard", "solid") + chev()),
    row("Model evaluator", `<span style="display:inline-flex;align-items:center;gap:6px">${mark("codex", 11)}Codex · GPT Sol · high</span>`, pill("Tracks standard", "solid") + chev()),
  ], "Specialized rubric")}
  ${group("Catalog", [
    row("Codex catalog", "Using last-known-good models. Live discovery failed 2 hours ago.", pill("Stale", "warn", true) + btn("Retry", "qui", I.refresh(12))),
  ])}
` });

pages.Presets = frame({ active: "presets", body: `
  ${crumbs(["Presets", "Bridge orchestrator"])}
  ${header("Bridge orchestrator", "Built-in preset. Reset restores Bridge defaults.", pill("Default", "solid") + btn("Reset", "qui", I.rotate(12)))}
  ${group("Identity", [
    row("Enabled", "", sw(true)),
    row("Name", "", fld("Bridge orchestrator", 260)),
    row("Description", "", fld("Plans, routes, and owns the final answer.", 320)),
    row("Role", "Decides which runtimes and sandbox modes are eligible.", sel("Orchestrator", 200)),
  ])}
  ${group("Runtime", [
    row("Harness", "", sel("Bridge chooses", 200)),
    row("Model", "Follows the role profile until a model is picked here.", sel("Provider default", 200, true)),
    row("Effort", "", sel("medium", 200)),
  ])}
  ${group("System prompt", [
    editor("bridge-orchestrator.md", [`<span class="faint">Add role-specific behavior…</span>`]),
  ], "Appended after Bridge safety and routing policy")}
` });

pages.Prompts = frame({ active: "prompts", height: 1080, body: `
  ${crumbs(["Prompts", "Orchestrator", `<span class="mono">bridge_role</span>`])}
  ${header(`<span class="mono" style="font-weight:500;font-size:16px">bridge_role</span>`, "One section of the orchestrator's compiled prompt. Bridge lints the draft as you type.", pill("Modified", "warn") + `<span class="mono muted" style="font-size:11px">15 tok</span>` + btn("Reset section", "qui", I.rotate(12)))}
  ${group("", [
    editor("prompt.md", ["You are Bridge's starter orchestrator: a planner and router.", "", "Delegate with one fenced bridge-delegate block per worker and", "name the role and the reason in a short sentence first."], `<span style="display:flex;align-items:center;gap:6px"><span style="width:6px;height:6px;border-radius:3px;background:${T.info};display:inline-block"></span>Unsaved draft</span>`).replace('min-height: 96px', 'min-height: 220px'),
  ])}
  ${group("Sections", [
    row(`<span class="mono">bridge_role</span>`, "", `<span class="mono muted" style="font-size:11px">15 tok</span>` + pill("Modified", "warn") + chev()),
    row(`<span class="mono">delegation_protocol</span>`, "", `<span class="mono muted" style="font-size:11px">29 tok</span>` + chev()),
  ], "Orchestrator · 2 sections")}
  ${group("History", [
    row("Now", "Unsaved draft", `<span class="badge-dot" style="background:${T.info}"></span>`),
    row("Saved", "Today, 14:02", btn("Restore", "qui")),
    row("Reset to default", "Yesterday, 18:40", btn("Restore", "qui")),
  ])}
  ${group("Compiled preview", [
    row("Stable prefix", `<span class="mono">d99755baa49fed571… · 388 bytes · 83 tok</span>`, pill("Exact", "solid") + `<span class="chev">${I.chevD(14)}</span>`, { mono: true }),
    row("Provider layers", "Codex · Claude Code · OpenCode, estimated", `<span class="chev">${I.chevD(14)}</span>`),
  ], "Across all sections")}
  ${savebar("Unsaved draft in bridge_role")}
` });

pages.Permissions = frame({ active: "permissions", body: `
  ${header("Permissions", "How much Bridge asks before an agent acts.")}
  ${group("Provider prompts", [
    row("Auto-approve provider permissions", "Requests from Claude, Codex, OpenCode, and Cursor are accepted when the provider offers an allow option. Questions and macOS prompts still wait for you.", sw(false)),
  ])}
  ${group("Always asks", [
    row("Worker write scope", "A worker still needs your authorization for the paths it may write.", `<span class="muted">${I.lock(14)}</span>`),
    row("Browser outward effects", "Send, submit, purchase, publish, and credential steps ask every time.", `<span class="muted">${I.lock(14)}</span>`),
  ], "Not affected by the switch above")}
  ${group("Recent auto-approvals", [
    row(`<span class="mono" style="font-size:12px">Allowed Bash: bun run test</span>`, "Codex · Build session supervisor", `<span class="mono faint" style="font-size:11px">14:02</span>`),
    row(`<span class="mono" style="font-size:12px">Allowed Read: src/App.tsx</span>`, "Claude Code · Polish the Deck shell", `<span class="mono faint" style="font-size:11px">13:58</span>`),
  ], "Every automatic decision is recorded")}
` });

pages.Work = frame({ active: "work", body: `
  ${header("Work briefing", "A background read of your connected tools, summarised onto the Work board. Read-only, on your account.")}
  ${group("Suggested work", [
    row("Enabled", "Off is remembered as a choice. Facts stay on the board either way.", sw(true)),
    row("Harness", "", sel(`<span style="display:inline-flex;align-items:center;gap:7px">${mark("claude", 12)}Claude Code</span>`, 220)),
    row("Model", "The cheapest capable model is the default.", sel("Haiku", 220)),
    row("Effort", "", sel("Provider default", 220, true)),
  ])}
  ${group("What it reads", [
    row("Everything connected", "Including tools you connect later.", sw(false)),
    row("Slack", "", sw(true)),
    row("Linear", "", sw(true)),
    row("GitHub", "Needs sign-in in Claude Code", sw(false)),
  ], "Tools Claude Code has connected")}
  ${group("When it runs", [
    row("Cadence", "", sel("Every hour", 220)),
    row("Refresh when Bridge regains focus", "Unattended runs keep a 15-minute cooldown. A manual refresh always works.", sw(true)),
  ])}
` });

const preview = (bg, sb, card, fg) => `<div class="prev" style="background:${bg}"><div style="width:30%;background:${sb};border-right:1px solid ${card}"></div><div style="flex:1;padding:8px;display:flex;flex-direction:column;gap:5px"><div style="height:6px;width:40%;border-radius:3px;background:${fg};opacity:.7"></div><div style="flex:1;border-radius:5px;background:${card}"></div></div></div>`;
const tile = (on, prev, label, hint) => `<div class="tile${on ? " on" : ""}">${prev}<div class="cap"><span><b>${label}</b><span>${hint}</span></span><span class="radio${on ? " on" : ""}"></span></div></div>`;
pages.Appearance = frame({ active: "appearance", body: `
  ${header("Appearance", "Bridge follows macOS by default. Both modes share one palette.")}
  <section class="g"><div class="g-h"><h3>Mode</h3><span>Showing Graphite</span></div><div class="tiles">
    ${tile(true, `<div class="prev" style="display:flex"><div style="flex:1">${preview("#fafaf9", "#f3f3f1", "#e7e7e4", "#1f1f1d").replace('class="prev"', 'style="display:flex;height:100%"')}</div><div style="flex:1">${preview("#000", "#000", "#1e1e1e", "#e4e4e4").replace('class="prev"', 'style="display:flex;height:100%"')}</div></div>`, "Match macOS", "Follows system appearance")}
    ${tile(false, preview("#fafaf9", "#f3f3f1", "#e7e7e4", "#1f1f1d"), "Paper", "Light")}
    ${tile(false, preview("#000", "#000", "#1e1e1e", "#e4e4e4"), "Graphite", "Dark")}
  </div></section>
  <section class="g"><div class="g-h"><h3>Shell</h3></div><div class="tiles two">
    ${tile(true, preview("#000", "#000", "#1e1e1e", "#e4e4e4"), "Solid", "Opaque window")}
    ${tile(false, preview("#111", "#0d0d0d", "#242424", "#e4e4e4"), "Translucent", "Tints with your wallpaper")}
  </div></section>
` });

// Composer: the page Inline suggestions moves to.
pages.Composer = frame({ active: "composer", body: `
  ${header("Composer", "How the message box behaves while you type.")}
  ${group("Inline suggestions", [
    row("Enabled", "Ghost-text continuations of your draft, accepted with Tab. Your draft is sent to the model only while this is on.", sw(false)),
    row("Model", "", sel(`<span style="display:inline-flex;align-items:center;gap:7px">${mark("claude", 12)}Claude Code · Haiku</span>`, 240, true)),
  ])}
` });

// ── System sheet ─────────────────────────────────────────────────────────
pages.System = `<!doctype html>
<html><head><meta charset="utf-8"><script src="./support.js"></script></head><body><x-dc><helmet>
  <link rel="stylesheet" href="https://fonts.googleapis.com/css2?family=Geist:wght@400;500;600&family=Geist+Mono:wght@400;500&display=swap">
  <style>${CSS}</style></helmet>
<div class="sheet" style="height:1780px">
  <h2>Settings system</h2>
  <p class="lede">One layout for all nine pages. Every section is a column of groups; every group is a card of rows; every row is a label on the left and one control on the right. Nothing else is allowed on a settings page.</p>

  <div class="sec"><div><h4>Rail</h4><p>Nine pages in four groups. Search filters rows across pages. The old header, its stale subtitle, and the shield note are gone. Reset all moves to the rail footer.</p></div>
    <div class="demo"><div style="width:208px;border:1px solid ${T.borderCard};border-radius:12px;overflow:hidden;background:${T.bg}">${rail("harnesses").replace('class="rail"', 'class="rail" style="border-right:0;height:540px;overflow:hidden"')}</div>
    <div class="spec" style="max-width:300px;line-height:1.7">208px wide · items 30px · 13px Geist<br>Group labels 10.5px, tracked, uppercase<br>Active: fg at 8% behind, no chevron<br>Composer is new: Inline suggestions moves here from Role models</div></div></div>

  <div class="sec"><div><h4>Row</h4><p>44px minimum, 13px label, 11.5px description, one control. Rows separate with a hairline, never with margin. A row that opens a page ends in a chevron.</p></div>
    <div class="demo"><div style="width:560px" class="card">${row("Enabled", "Description at 11.5px, muted, line-height 1.4.", sw(true))}${row("Default model", "", sel("Kimi K2.5", 200))}${row("Executable", "", fld("/usr/local/bin/opencode", 240, "mono"))}${row("Claude Code", "Bridge-managed · 0.3.209", pill("Ready", "ok", true) + chev(), { lead: icoBox(mark("claude", 16)) })}</div>
    <div class="spec" style="line-height:1.7">Card ${T.card} on ${T.borderCard}, radius 12<br>Hairline ${T.border}<br>Lead icon box 28px on ${T.popover}</div></div></div>

  <div class="sec"><div><h4>Controls</h4><p>Five controls, one height. Native checkboxes become switches; native selects become 28px menus. Filled button only for Save and Connect.</p></div>
    <div class="demo">${sw(true)}${sw(false)}${sel("Track standard", 170)}${fld("Bridge orchestrator", 190)}${btn("Save", "pri")}${btn("Install", "gho")}${btn("Remove", "des")}${btn("Reset", "qui", I.rotate(12))}${pill("Ready", "ok", true)}${pill("Stale", "warn", true)}${pill("Pinned", "info")}${pill("Tracks standard", "solid")}<span class="spec">28px controls · 12px text · radius 8</span></div></div>

  <div class="sec"><div><h4>Saving</h4><p>Switches and menus save on change and confirm with a brief check in the row. Editors and text fields dirty the page; one bar at the bottom of the column saves or discards. That replaces the five save patterns settings has today.</p></div>
    <div class="demo"><div style="width:560px;position:relative;height:52px">${savebar()}</div>
    <div style="width:220px" class="card">${row("Enabled", "", `<span style="display:inline-flex;align-items:center;gap:8px;color:${T.success};font-size:11.5px">${I.check(12)}Saved</span>${sw(true)}`)}</div></div></div>

  <div class="sec"><div><h4>Type</h4><p>Four sizes replace the ten in use today.</p></div>
    <div class="demo" style="gap:28px"><span style="font-size:17px;font-weight:600;letter-spacing:-0.012em">17 Page title</span><span style="font-size:13px">13 Row label</span><span style="font-size:12px;color:${T.mutedFg}">12 Control, description at 11.5</span><span style="font-size:11px;font-weight:600;letter-spacing:.06em;text-transform:uppercase;color:${T.mutedFg}">11 Group</span><span class="mono" style="font-size:10.5px;color:${T.mutedFg}">10.5 mono meta</span></div></div>

  <div class="sec"><div><h4>Master and detail</h4><p>Presets, Harnesses, and Prompts stop drawing a third sidebar. A list page of rows opens a detail page with a breadcrumb, inside the same 720px column.</p></div>
    <div class="demo"><div style="width:560px">${crumbs(["Harnesses", "OpenCode"])}</div></div></div>

  <div class="sec"><div><h4>Harness marks</h4><p>The real brand marks, exact geometry. Claude tinted with its harness color; OpenAI stays white as the brand draws it; Cursor keeps its own facets; OpenCode's frame takes the harness tint with the inner block at 45%. The hand-built radial stand-ins in harnessMarks.tsx go, and so does the rotating startup animation: a mark breathes in opacity and never turns or deforms.</p></div>
    <div class="demo" style="gap:22px">${mark("claude", 20)}${mark("codex", 20)}${mark("opencode", 20)}${mark("cursor", 20)}${mark("unknown", 20)}<span class="spec">${T.claude} · fg · ${T.opencode} · fg · muted</span><span style="width:24px"></span>${mark("claude", 12)}${mark("codex", 12)}${mark("opencode", 12)}${mark("cursor", 12)}<span class="spec">12px</span></div></div>
</div></x-dc></body></html>`;

// ── Icon set comparison ──────────────────────────────────────────────────
const STRIP = ["search","bot","code","sliders","scroll","shield","download","sparkles","sun","keyboard","rotate","refresh","key","unplug","lock","history","folder","chevR","check","plus"];
const SET_LABEL = { phosphor: "Phosphor · Regular", "phosphor-light": "Phosphor · Light", iconoir: "Iconoir", tabler: "Tabler" };
const SET_NOTE = {
  phosphor: "Filled strokes on a 256 grid, so shapes stay crisp at 14px. 6 weights in one family. @phosphor-icons/react.",
  "phosphor-light": "Same family, one weight lighter. Quieter in the rail, thin at 12px.",
  iconoir: "1.5px strokes, rounder and softer. Closest to lucide in feel. iconoir-react.",
  tabler: "Bold 2px strokes, largest set (5k). Heaviest on a dark ground. @tabler/icons-react.",
};
function iconSheet() {
  const cols = SET_NAMES.map(name => {
    const J = makeIcons(name);
    return { name, J };
  });
  const railFor = J => `<nav class="rail" style="border-right:0;height:540px;overflow:hidden"><h1>Settings</h1><div class="search">${J.search(12)}<span>Search settings</span></div>${NAV.map(([g, items]) => `<div class="grp">${g}</div>${items.map(([id, label, key]) => `<div class="it${id === "harnesses" ? " on" : ""}">${J[ICON_KEY[id]](14)}<span>${label}</span></div>`).join("")}`).join("")}<div class="foot">${J.rotate(12)}Reset all settings</div></nav>`;
  return `<!doctype html>
<html><head><meta charset="utf-8"><script src="./support.js"></script></head><body><x-dc><helmet>
  <link rel="stylesheet" href="https://fonts.googleapis.com/css2?family=Geist:wght@400;500;600&family=Geist+Mono:wght@400;500&display=swap">
  <style>${CSS}</style></helmet>
<div class="sheet" style="height:1000px">
  <h2>Icon set</h2>
  <p class="lede">Lucide goes. Four candidates, each drawn from the library's real SVGs at the sizes the rail uses: 14px in the rail, 12px in controls, 15px in the app sidebar. The rest of the canvas uses ${SET_LABEL[SET]}.</p>
  <div style="display:grid;grid-template-columns:repeat(4, minmax(0,1fr));gap:24px">
    ${cols.map(({ name, J }) => `<div>
      <div style="display:flex;align-items:baseline;justify-content:space-between;margin-bottom:10px"><b style="font-size:13px;font-weight:600">${SET_LABEL[name]}</b>${name === SET ? pill("Used here", "info") : ""}</div>
      <p style="margin:0 0 14px;font-size:11.5px;color:${T.mutedFg};line-height:1.5;min-height:52px">${SET_NOTE[name]}</p>
      <div style="border:1px solid ${T.borderCard};border-radius:12px;overflow:hidden;background:${T.bg}">${railFor(J)}</div>
      <div style="display:grid;grid-template-columns:repeat(10, minmax(0,1fr));gap:10px 0;margin-top:18px;color:${T.body};justify-items:center">${STRIP.map(k => J[k](16)).join("")}</div>
      <div style="display:grid;grid-template-columns:repeat(10, minmax(0,1fr));gap:10px 0;margin-top:12px;color:${T.mutedFg};justify-items:center">${STRIP.map(k => J[k](12)).join("")}</div>
    </div>`).join("")}
  </div>
</div></x-dc></body></html>`;
}
const ICON_KEY = { appearance: "sun", permissions: "shield", composer: "keyboard", presets: "bot", models: "sliders", prompts: "scroll", harnesses: "code", work: "sparkles", import: "download" };

// ── Before artboards ─────────────────────────────────────────────────────
const before = (id, h) => `<!doctype html>
<html><head><meta charset="utf-8"><script src="./support.js"></script></head><body><x-dc><helmet><style>${CSS}</style></helmet>
<div class="before" style="width:1200px;height:${h}px"><img src="before-${id}.jpg" alt="Current ${id} section"></div></x-dc></body></html>`;

// ── Write ────────────────────────────────────────────────────────────────
const files = {
  "Main.dc.html": pages.Harnesses,
  "HarnessDetail.dc.html": pages.HarnessDetail,
  "Models.dc.html": pages.Models,
  "Presets.dc.html": pages.Presets,
  "Prompts.dc.html": pages.Prompts,
  "Permissions.dc.html": pages.Permissions,
  "Work.dc.html": pages.Work,
  "Appearance.dc.html": pages.Appearance,
  "Composer.dc.html": pages.Composer,
  "System.dc.html": pages.System,
  "Icons.dc.html": iconSheet(),
  "BeforeHarnesses.dc.html": before("harnesses", 3170),
  "BeforeRoleModels.dc.html": before("role-models", 1920),
  "BeforeAgents.dc.html": before("agents", 750),
  "BeforePromptStudio.dc.html": before("prompt-studio", 750),
  "BeforePermissions.dc.html": before("permissions", 750),
  "BeforeWork.dc.html": before("work", 750),
  "BeforeAppearance.dc.html": before("appearance", 750),
  "BeforeImport.dc.html": before("import", 750),
};
for (const [name, html] of Object.entries(files)) writeFileSync(`${OUT}/${name}`, html);

// Layout: column per page. Row 1 = before (real screenshots), row 2 = after.
const GAP = 120;
const cols = [
  ["BeforeHarnesses.dc.html", 1200, 3170, "Main.dc.html", "Before · Harnesses", "After · Harnesses"],
  ["BeforeHarnesses.dc.html", null, null, "HarnessDetail.dc.html", null, "After · Harness detail (OpenCode)"],
  ["BeforeRoleModels.dc.html", 1200, 1920, "Models.dc.html", "Before · Role models", "After · Models"],
  ["BeforeAgents.dc.html", 1200, 750, "Presets.dc.html", "Before · Agents", "After · Presets detail"],
  ["BeforePromptStudio.dc.html", 1200, 750, "Prompts.dc.html", "Before · Prompt Studio", "After · Prompts"],
  ["BeforePermissions.dc.html", 1200, 750, "Permissions.dc.html", "Before · Permissions", "After · Permissions"],
  ["BeforeWork.dc.html", 1200, 750, "Work.dc.html", "Before · Work", "After · Work briefing"],
  ["BeforeAppearance.dc.html", 1200, 750, "Appearance.dc.html", "Before · Appearance", "After · Appearance"],
  ["BeforeImport.dc.html", 1200, 750, "Composer.dc.html", "Before · Import (unchanged, restyled)", "After · Composer (new page)"],
];
const heights = { "Main.dc.html": 900, "HarnessDetail.dc.html": 1180, "Models.dc.html": 1040, "Presets.dc.html": 900, "Prompts.dc.html": 1080, "Permissions.dc.html": 900, "Work.dc.html": 900, "Appearance.dc.html": 900, "Composer.dc.html": 900 };
const artboards = [];
const beforeRowH = 1000; // befores taller than this scroll inside their frame
let x = 0;
for (const [bf, bw, bh, af, bt, at] of cols) {
  if (bw) artboards.push({ file: bf, x, y: 0, w: bw, h: Math.min(bh, beforeRowH), title: bt, page: "page-1", expand: "fit" });
  artboards.push({ file: af, x, y: beforeRowH + GAP + 40, w: 1440, h: heights[af], title: at, page: "page-1" });
  x += 1440 + GAP;
}
artboards.push({ file: "System.dc.html", x: 0, y: 0, w: 1440, h: 1780, title: "Settings system", page: "page-2" });
artboards.push({ file: "Icons.dc.html", x: 1560, y: 0, w: 1440, h: 1000, title: "Icon set", page: "page-2" });

const annotations = [
  { id: "why", x: 0, y: -260, w: 640, page: "page-1", text: "Settings redesign · issue 459\n\nTop row: the current build, captured from the running app.\nBottom row: the proposal. Same tokens, same fonts, same marks.\n\nOne rule fixes most of it: every page is a column of groups, every group is a card of rows, every row is one label and one control. See the System page for the rules." },
  { id: "ia", x: 700, y: -260, w: 560, page: "page-1", text: "Information architecture\n\nGeneral: Appearance, Permissions, Composer\nAgents: Presets, Models, Prompts\nRuntimes: Harnesses\nData: Work briefing, Import\n\nRenames: Agents to Presets, Role models to Models, Prompt Studio to Prompts.\nMoves: Inline suggestions leaves Role models for Composer. Agent runtimes and harness config merge into one Harnesses list with a detail page each." },
  { id: "save", x: 1320, y: -260, w: 520, page: "page-1", text: "Saving\n\nToday there are five save patterns: per-card Save, header Save, bottom Save, autosave on toggle, and a global Reset all that also deletes custom agents.\n\nAfter: switches and menus autosave with a check in the row. Editors dirty the page and one bar at the bottom of the column saves or discards. Reset all moves to the rail footer and gets a real confirmation." },
];

const canvas = {
  pages: [{ id: "page-1", name: "Before and after" }, { id: "page-2", name: "System" }],
  artboards,
  annotations,
  launch: { view: "canvas", page: "page-1" },
};
writeFileSync(`${OUT}/canvas.json`, JSON.stringify(canvas, null, 2));
console.log("wrote", Object.keys(files).length, "artboards");

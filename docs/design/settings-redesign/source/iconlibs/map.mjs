import { existsSync } from "node:fs";
const P = n => `node_modules/@phosphor-icons/core/assets/regular/${n}.svg`;
const N = n => `node_modules/iconoir/icons/regular/${n}.svg`;
const B = n => `node_modules/@tabler/icons/icons/outline/${n}.svg`;
export const MAP = {
  // key: [phosphor, iconoir, tabler]
  search: ["magnifying-glass", "search", "search"],
  chevR: ["caret-right", "nav-arrow-right", "chevron-right"],
  chevD: ["caret-down", "nav-arrow-down", "chevron-down"],
  chevL: ["caret-left", "nav-arrow-left", "chevron-left"],
  check: ["check", "check", "check"],
  plus: ["plus", "plus", "plus"],
  bot: ["robot", "sparks", "robot"],
  code: ["code", "code", "code"],
  sliders: ["sliders-horizontal", "control-slider", "adjustments-horizontal"],
  scroll: ["scroll", "page-edit", "file-text"],
  shield: ["shield-check", "shield", "shield"],
  download: ["download-simple", "download", "download"],
  sparkles: ["sparkle", "sparks", "sparkles"],
  sun: ["sun", "sun-light", "sun"],
  moon: ["moon", "half-moon", "moon"],
  monitor: ["monitor", "computer", "device-desktop"],
  keyboard: ["keyboard", "type", "keyboard"],
  rotate: ["arrow-counter-clockwise", "refresh-double", "rotate"],
  trash: ["trash", "trash", "trash"],
  key: ["key", "key", "key"],
  unplug: ["plugs", "plug-type-a", "plug-connected-x"],
  refresh: ["arrows-clockwise", "refresh", "refresh"],
  folder: ["folder", "folder", "folder"],
  panel: ["sidebar-simple", "sidebar-collapse", "layout-sidebar"],
  store: ["storefront", "shop", "building-store"],
  projects: ["kanban", "view-columns-3", "layout-kanban"],
  memory: ["brain", "brain", "brain"],
  edit: ["pencil-simple-line", "edit-pencil", "edit"],
  branch: ["git-branch", "git-branch", "git-branch"],
  chart: ["chart-bar", "stats-up-square", "chart-bar"],
  lock: ["lock-simple", "lock", "lock"],
  filter: ["funnel-simple", "filter", "filter"],
  history: ["clock-counter-clockwise", "clock-rotate-right", "history"],
  layers: ["stack", "book-stack", "stack-2"],
  square: ["square", "square", "square"],
  x: ["x", "xmark", "x"],
};
export const FILES = { phosphor: P, iconoir: N, tabler: B };
if (process.argv[2] === "check") {
  for (const [k, [p, n, b]] of Object.entries(MAP)) {
    const miss = [[P(p), "phosphor:" + p], [N(n), "iconoir:" + n], [B(b), "tabler:" + b]].filter(([f]) => !existsSync(f)).map(([, l]) => l);
    if (miss.length) console.log(k, "missing", miss.join(", "));
  }
  console.log("checked");
}

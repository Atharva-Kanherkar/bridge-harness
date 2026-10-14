import assert from "node:assert/strict";
import { readdirSync, readFileSync } from "node:fs";
import { test } from "node:test";
import { DEFAULT_OMARCHY_THEME, OMARCHY_THEMES } from "../app/content/omarchyThemes.ts";
import { prepaintScript, SKIN_STORAGE_KEY, THEME_STORAGE_KEY } from "../app/lib/omarchySkin.ts";

const fixtures = new URL("../../testing/fixtures/omarchy/themes/", import.meta.url);

test("the landing page uses the app's token table, byte for byte", () => {
  const app = readFileSync(new URL("../../src/lib/omarchyTheme.ts", import.meta.url), "utf8");
  const landing = readFileSync(new URL("../app/lib/omarchyTheme.ts", import.meta.url), "utf8");
  assert.equal(landing, app, "copy src/lib/omarchyTheme.ts to landing/app/lib/omarchyTheme.ts");
});

test("the theme list matches Omarchy's shipped themes", () => {
  const files = readdirSync(fixtures).filter(file => file.endsWith(".toml")).sort();
  assert.deepEqual(OMARCHY_THEMES.map(theme => `${theme.name}.toml`), files, "run scripts/generate-omarchy-themes.mjs");
  for (const theme of OMARCHY_THEMES) {
    const text = readFileSync(new URL(`${theme.name}.toml`, fixtures), "utf8");
    assert.equal(theme.mode, /^mode\s*=\s*"light"/m.test(text) ? "light" : "dark", theme.name);
    for (const [key, value] of Object.entries(theme.colors)) {
      assert.match(text, new RegExp(`^${key}\\s*=\\s*"${value}"`, "mi"), `${theme.name}.${key}`);
    }
  }
  assert.ok(OMARCHY_THEMES.some(theme => theme.name === DEFAULT_OMARCHY_THEME));
});

/** Run the pre-paint script against a fake document, storage and URL. */
function boot({ search = "", stored = {}, storageThrows = false, store: shared } = {}) {
  const store = shared ?? { ...stored };
  const location = { search, href: `https://bridge.example/${search}` };
  const history = {
    state: null,
    replaceState: (_state, _title, url) => {
      const next = new URL(url, location.href);
      location.href = next.href;
      location.search = next.search;
    },
  };
  const props = new Map();
  const root = {
    dataset: {},
    style: { setProperty: (k, v) => props.set(k, v), removeProperty: k => props.delete(k) },
  };
  const events = [];
  const window = { dispatchEvent: event => events.push(event.type) };
  const localStorage = {
    getItem: key => { if (storageThrows) throw new Error("blocked"); return store[key] ?? null; },
    setItem: (key, value) => { if (storageThrows) throw new Error("blocked"); store[key] = value; },
  };
  new Function("document", "window", "localStorage", "location", "history", "URLSearchParams", "URL", "Event", prepaintScript())(
    { documentElement: root }, window, localStorage, location, history, URLSearchParams, URL, class { constructor(type) { this.type = type; } },
  );
  return { root, props, store, events, location, controller: window.__bridgeOmarchy };
}

test("off by default: nothing is painted", () => {
  const { root, props, controller } = boot();
  assert.equal(root.dataset.skin, undefined);
  assert.equal(props.size, 0);
  assert.equal(controller.current(), null);
});

test("a stored choice is painted before first paint", () => {
  const { root, props } = boot({ stored: { [SKIN_STORAGE_KEY]: "omarchy", [THEME_STORAGE_KEY]: "rose-pine" } });
  assert.equal(root.dataset.skin, "omarchy");
  assert.equal(root.dataset.omarchyTheme, "rose-pine");
  assert.equal(props.get("--background"), "#faf4ed");
  assert.equal(props.get("color-scheme"), "light");
});

test("?omarchy deep-links in, falling back for unknown themes, and ?omarchy=off leaves", () => {
  const known = boot({ search: "?omarchy=gruvbox" });
  assert.equal(known.root.dataset.omarchyTheme, "gruvbox");
  assert.equal(known.store[SKIN_STORAGE_KEY], "omarchy");

  const unknown = boot({ search: "?omarchy=not-a-theme" });
  assert.equal(unknown.root.dataset.omarchyTheme, DEFAULT_OMARCHY_THEME);

  const off = boot({ search: "?omarchy=off", stored: { [SKIN_STORAGE_KEY]: "omarchy", [THEME_STORAGE_KEY]: "nord" } });
  assert.equal(off.root.dataset.skin, undefined);
  assert.equal(off.store[SKIN_STORAGE_KEY], "graphite");
});

test("the controller switches themes and turns the mode off cleanly", () => {
  const { root, props, events, controller } = boot();
  controller.apply("nord");
  assert.equal(controller.current(), "nord");
  assert.ok(props.has("--primary"));
  controller.apply(null);
  assert.equal(controller.current(), null);
  assert.equal(root.dataset.skin, undefined);
  assert.equal(props.size, 0);
  assert.deepEqual(events, ["bridge-omarchy-change", "bridge-omarchy-change"]);
});

test("blocked storage still lets the mode apply for the visit", () => {
  const { root, controller } = boot({ storageThrows: true, search: "?omarchy=everforest" });
  assert.equal(root.dataset.omarchyTheme, "everforest");
  controller.apply(null);
  assert.equal(root.dataset.skin, undefined);
});

test("the copy claims only what ships", () => {
  const page = readFileSync(new URL("../app/page.tsx", import.meta.url), "utf8");
  const banner = page.slice(page.indexOf("Made for Omarchy"), page.indexOf("Made for Omarchy") + 120);
  assert.match(banner, /colors and font from your theme/);
  assert.doesNotMatch(page, /notification|quickshell|\bAUR\b|waybar/i);
  const promo = readFileSync(new URL("../app/components/omarchy/OmarchyPromo.tsx", import.meta.url), "utf8");
  assert.match(promo, /colors and font from your theme/);
  assert.doesNotMatch(promo, /notification|quickshell|\bAUR\b|waybar/i);
});

test("a choice made after a deep link survives reload", () => {
  const first = boot({ search: "?omarchy=gruvbox" });
  assert.equal(first.root.dataset.omarchyTheme, "gruvbox");
  assert.equal(first.location.search, "?omarchy=gruvbox", "boot leaves an incoming link alone");
  first.controller.apply("nord");
  assert.equal(first.location.search, "");
  const reload = boot({ search: first.location.search, store: first.store });
  assert.equal(reload.root.dataset.omarchyTheme, "nord");

  const offFromLink = boot({ search: "?omarchy=gruvbox" });
  offFromLink.controller.apply(null);
  const reloadOff = boot({ search: offFromLink.location.search, store: offFromLink.store });
  assert.equal(reloadOff.root.dataset.skin, undefined);
});

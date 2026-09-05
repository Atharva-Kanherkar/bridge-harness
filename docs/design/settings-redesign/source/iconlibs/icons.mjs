import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import { MAP } from "./map.mjs";
const here = dirname(fileURLToPath(import.meta.url));
const inner = svg => svg.replace(/^[\s\S]*?<svg[^>]*>/, "").replace(/<\/svg>\s*$/, "").replace(/<path stroke="none" d="M0 0h24v24H0z" fill="none" \/>/, "").trim();
const SETS = {
  phosphor: { dir: "node_modules/@phosphor-icons/core/assets/regular", idx: 0, wrap: (c, s) => `<svg width="${s}" height="${s}" viewBox="0 0 256 256" fill="currentColor" aria-hidden="true">${c}</svg>` },
  "phosphor-light": { dir: "node_modules/@phosphor-icons/core/assets/light", idx: 0, suffix: "-light", wrap: (c, s) => `<svg width="${s}" height="${s}" viewBox="0 0 256 256" fill="currentColor" aria-hidden="true">${c}</svg>` },
  iconoir: { dir: "node_modules/iconoir/icons/regular", idx: 1, wrap: (c, s) => `<svg width="${s}" height="${s}" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${c}</svg>` },
  tabler: { dir: "node_modules/@tabler/icons/icons/outline", idx: 2, wrap: (c, s) => `<svg width="${s}" height="${s}" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${c}</svg>` },
};
export const SET_NAMES = Object.keys(SETS);
export function makeIcons(set) {
  const def = SETS[set];
  if (!def) throw new Error("unknown icon set " + set);
  const cache = {};
  const I = {};
  for (const [key, names] of Object.entries(MAP)) {
    const file = join(here, def.dir, names[def.idx] + (def.suffix ?? "") + ".svg");
    I[key] = size => {
      cache[key] ??= inner(readFileSync(file, "utf8"));
      return def.wrap(cache[key], size);
    };
  }
  return I;
}

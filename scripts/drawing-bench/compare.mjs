#!/usr/bin/env node
// Side-by-side sheets of the drawing bench, one per case: the same request
// before and after, as the page shows them, with each side's defects.
//
//   node scripts/drawing-bench/compare.mjs <before dir> <after dir> <out dir> [run]
//
// Run score.mjs on both directories first.

import fs from "node:fs";
import path from "node:path";
import { openRenderer } from "./score.mjs";

const [before, after, out, runArg] = process.argv.slice(2);
const run = Number(runArg ?? 0);
const load = dir => JSON.parse(fs.readFileSync(path.join(dir, "scores.json"), "utf8")).rows;
const [a, b] = [load(before), load(after)];
fs.mkdirSync(out, { recursive: true });
const esc = text => String(text).replace(/[&<>]/g, c => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;" })[c]);
const side = (dir, row, title) => {
  const image = row?.png ? `<img src="data:image/png;base64,${fs.readFileSync(path.join(dir, row.png)).toString("base64")}" style="width:640px;display:block;border:1px solid #ddd;border-radius:8px">` : `<div style="width:640px;height:120px;border:1px dashed #bbb;display:flex;align-items:center;justify-content:center;color:#888">no drawing</div>`;
  const s = row?.score;
  const defects = (s?.defects ?? []).filter(d => d.kind !== "dangling" && d.kind !== "crossing").map(d => `<li>${esc(d.kind)}: ${esc(d.detail)}</li>`).join("");
  const missing = (s?.coverage.checks ?? []).filter(c => !c.ok).map(c => `<li>missing ${esc(c.kind)} ${esc(c.detail)}</li>`).join("");
  return `<div style="width:640px"><div style="font:600 15px system-ui;margin:0 0 6px">${title}: ${s ? `${s.severe} severe, coverage ${s.coverage.passed}/${s.coverage.total}, text ${s.readability.minPx?.toFixed(1)} px` : ""}</div>${image}<ul style="font:12px system-ui;color:#b42318;margin:6px 0 0;padding-left:18px">${defects}${missing}</ul></div>`;
};
const renderer = await openRenderer();
try {
  for (const row of b.filter(r => r.run === run)) {
    const was = a.find(r => r.id === row.id && r.run === run) ?? a.find(r => r.id === row.id);
    const ask = JSON.parse(fs.readFileSync(path.join(after, `${row.id}-${row.run}.json`), "utf8")).ask;
    const html = `<div style="padding:20px;background:#fff;font:14px system-ui"><div style="font:600 17px system-ui">${esc(row.id)}${row.holdout ? " (held out)" : ""}</div><div style="color:#555;margin:4px 0 14px;max-width:1300px">${esc(ask)}</div><div style="display:flex;gap:24px;align-items:flex-start">${side(before, was, "Before")}${side(after, row, "After")}</div></div>`;
    await renderer.shoot(html, 1344, path.join(out, `${row.id}.png`));
    console.log(path.join(out, `${row.id}.png`));
  }
} finally {
  await renderer.close();
}

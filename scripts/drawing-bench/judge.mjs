#!/usr/bin/env node
// A blind second opinion on the drawing bench: a model looks at two renders of
// the same request, in random order and without knowing which is which, and
// scores both. Run score.mjs on both directories first (it writes the PNGs).
//
//   node scripts/drawing-bench/judge.mjs <dir A> <dir B> [--model claude-opus-5-5]
//
// Uses the `claude` CLI. Results go to <dir B>/judge.json and are printed.

import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";

const args = process.argv.slice(2);
const modelAt = args.indexOf("--model");
const model = modelAt >= 0 ? args.splice(modelAt, 2)[1] : "claude-opus-5-5";
const [dirA, dirB] = args.map(dir => path.resolve(dir));
if (!dirA || !dirB) {
  console.error("usage: judge.mjs <dir A> <dir B> [--model id]");
  process.exit(2);
}
const rowsOf = dir => JSON.parse(fs.readFileSync(path.join(dir, "scores.json"), "utf8")).rows.filter(row => row.png);
const [rowsA, rowsB] = [rowsOf(dirA), rowsOf(dirB)];
const ask = (dir, row) => JSON.parse(fs.readFileSync(path.join(dir, `${row.id}-${row.run}.json`), "utf8")).ask;

const RUBRIC = `Score each image from 1 to 10 on:
- faithful: every part and connection the request asks for is there, joined the right way, nothing invented
- readable: every label is whole and inside its shape, nothing sits on top of anything, arrows and their labels are clear, text is large enough to read at this size
- layout: the flow reads in one direction, shapes are aligned and evenly spaced, the drawing uses the space well
- overall: how glad a person would be to find it in their notes
Reply with only this JSON: {"first": {"faithful": n, "readable": n, "layout": n, "overall": n, "issues": "one short line"}, "second": {same}, "better": "first" | "second" | "tie"}`;

const results = [];
for (const b of rowsB) {
  const a = rowsA.find(row => row.id === b.id && row.run === b.run) ?? rowsA.find(row => row.id === b.id);
  if (!a) continue;
  const flip = Math.random() < 0.5;
  const [first, second] = flip ? [[dirB, b], [dirA, a]] : [[dirA, a], [dirB, b]];
  const prompt = `Two people drew a diagram in their notes app for this request:\n\n"${ask(dirB, b)}"\n\nThe first drawing is the image at ${path.join(first[0], first[1].png)} and the second is the image at ${path.join(second[0], second[1].png)}. Read both images. Each is shown exactly as the notes page shows it, fitted to a 640 px wide column.\n\n${RUBRIC}`;
  let verdict = null;
  for (let attempt = 0; attempt < 2 && !verdict; attempt++) {
    try {
      const out = execFileSync("claude", ["-p", prompt, "--model", model, "--allowedTools", "Read", "--output-format", "text"], { encoding: "utf8", timeout: 240000, cwd: path.dirname(dirA) });
      verdict = JSON.parse(out.slice(out.indexOf("{"), out.lastIndexOf("}") + 1));
    } catch (error) {
      console.error(`judge failed on ${b.id}: ${String(error.message).slice(0, 200)}`);
    }
  }
  if (!verdict) continue;
  const scoreA = flip ? verdict.second : verdict.first;
  const scoreB = flip ? verdict.first : verdict.second;
  const better = verdict.better === "tie" ? "tie" : (verdict.better === "first") !== flip ? "A" : "B";
  results.push({ id: b.id, run: b.run, holdout: b.holdout, A: scoreA, B: scoreB, better });
  console.log(`${b.id.padEnd(18)} A ${scoreA.overall}  B ${scoreB.overall}  better ${better}   A: ${scoreA.issues}  |  B: ${scoreB.issues}`);
}
const mean = (list, side, key) => list.length ? list.reduce((total, r) => total + r[side][key], 0) / list.length : 0;
const summary = Object.fromEntries(["all", "dev", "holdout"].map(group => {
  const list = results.filter(r => group === "all" || (group === "holdout") === Boolean(r.holdout));
  const stats = { n: list.length, Bwins: list.filter(r => r.better === "B").length, Awins: list.filter(r => r.better === "A").length, ties: list.filter(r => r.better === "tie").length };
  for (const key of ["faithful", "readable", "layout", "overall"]) stats[key] = { A: +mean(list, "A", key).toFixed(2), B: +mean(list, "B", key).toFixed(2) };
  return [group, stats];
}));
fs.writeFileSync(path.join(dirB, "judge.json"), JSON.stringify({ model, a: dirA, b: dirB, summary, results }, null, 2));
for (const [group, s] of Object.entries(summary)) {
  console.log(`${group}: n ${s.n}, B better ${s.Bwins}, A better ${s.Awins}, ties ${s.ties}; overall A ${s.overall.A} vs B ${s.overall.B}; readable ${s.readable.A} vs ${s.readable.B}; layout ${s.layout.A} vs ${s.layout.B}; faithful ${s.faithful.A} vs ${s.faithful.B}`);
}

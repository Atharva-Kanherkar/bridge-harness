#!/usr/bin/env node
// Import fidelity eval for Orca: does each chat in Bridge say what Orca itself
// shows for it?
//
//   ORACLE=<folder of `orca agent read <id> --json` files and titles.json, captured from a running Orca> \
//   BRIDGE_DUMP=<json from: ORCA_EVAL_OUT=<file> cargo test -p bridge-core --lib -- --ignored dump_the_orca_import_for_the_eval> \
//     node scripts/e2e/import-orca-fidelity.mjs
//
// The oracle is Orca's own CLI (`orca agent read`, and the titles `orca agent
// list` showed), not its databases, so the reader under test is not checking
// itself. For every chat Orca read out, the Bridge chat must exist with
// Orca's title, and every user and assistant text Orca shows must appear in
// that chat, same role, in order. Prints one line per chat and a score; exits
// non-zero below 100%.

import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";

const { ORACLE, BRIDGE_DUMP } = process.env;
if (!ORACLE || !BRIDGE_DUMP) throw new Error("Set ORACLE and BRIDGE_DUMP");
const norm = text => String(text ?? "").replace(/\s+/g, " ").trim();
const chats = JSON.parse(readFileSync(BRIDGE_DUMP, "utf8"));
const titles = JSON.parse(readFileSync(join(ORACLE, "titles.json"), "utf8"));

let passed = 0;
let total = 0;
for (const file of readdirSync(ORACLE).filter(name => name.endsWith(".json") && name !== "titles.json" && name !== "list.json").sort()) {
  const read = JSON.parse(readFileSync(join(ORACLE, file), "utf8"));
  if (!read.ok) continue;
  const id = read.result.orcaSessionId;
  total += 1;
  const shown = read.result.entries
    .filter(entry => entry.kind === "message" && (entry.role === "user" || entry.role === "assistant") && norm(entry.text))
    .map(entry => ({ role: entry.role, text: norm(entry.text) }));
  const chat = chats.find(item => item.orcaId === id);
  const problems = [];
  if (!chat) problems.push("no Bridge chat");
  else {
    if (norm(chat.title) !== norm(titles[id])) problems.push(`title ${JSON.stringify(chat.title)} is not Orca's ${JSON.stringify(titles[id])}`);
    let at = 0;
    for (const want of shown) {
      const found = chat.entries.findIndex((entry, index) => index >= at && entry.role === want.role && norm(entry.text).includes(want.text));
      if (found < 0) { problems.push(`missing ${want.role}: ${want.text.slice(0, 60)}`); break; }
      at = found + 1;
    }
  }
  const ok = problems.length === 0;
  if (ok) passed += 1;
  console.log(`${ok ? "PASS" : "FAIL"} ${id} ${JSON.stringify(titles[id] ?? "")} (${shown.length} texts, ${chat?.source ?? "-"})${ok ? "" : `: ${problems.join("; ")}`}`);
}
const score = total ? Math.round((passed / total) * 100) : 0;
console.log(`SCORE ${passed}/${total} (${score}%)`);
process.exit(passed === total && total > 0 ? 0 : 1);

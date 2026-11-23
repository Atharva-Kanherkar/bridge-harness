#!/usr/bin/env node
// Import fidelity eval for Superset: does each chat in Bridge say what
// Superset itself shows for it?
//
//   ORACLE=<folder of `superset sessions read` text files, one per chat, and
//           `opencode session export` JSON files for terminal sessions, captured from a running host> \
//   BRIDGE_DUMP=<json from: SUPERSET_EVAL_OUT=<file> cargo test -p bridge-core --lib -- --ignored dump_the_superset_import_for_the_eval> \
//     node scripts/e2e/import-superset-fidelity.mjs
//
// The oracle is each tool's own CLI, not its database, so the reader under
// test is not checking itself: Superset's `sessions read` for its chats,
// and OpenCode's `session export` for a terminal session OpenCode ran. Every
// user and agent text the oracle shows must appear in that chat in Bridge,
// same role, in order. Superset's renderer prints every journaled version
// of an item (a reply first empty, then whole), so its distinct non-empty
// texts are compared, first-seen first. Prints one line per chat and a
// score; exits non-zero below 100%.

import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";

const { ORACLE, BRIDGE_DUMP } = process.env;
if (!ORACLE || !BRIDGE_DUMP) throw new Error("Set ORACLE and BRIDGE_DUMP");
const norm = text => String(text ?? "").replace(/\s+/g, " ").trim();
const chats = JSON.parse(readFileSync(BRIDGE_DUMP, "utf8"));

/** `[user] text` / `[agent] text` lines of `superset sessions read`, distinct, first seen first. */
function supersetTexts(raw) {
  const seen = new Set();
  const out = [];
  for (const line of raw.split("\n")) {
    const match = /^\[(user|agent)\] (.*)$/.exec(line);
    if (!match || !norm(match[2])) continue;
    const role = match[1] === "agent" ? "assistant" : "user";
    const key = `${role}\u0000${norm(match[2])}`;
    if (seen.has(key)) continue;
    seen.add(key);
    out.push({ role, text: norm(match[2]) });
  }
  return out;
}

/** User and assistant text parts of an `opencode session export`. */
function openCodeTexts(raw) {
  const exported = JSON.parse(raw);
  return (exported.messages ?? []).flatMap(message => {
    if (message.type === "user") return norm(message.text) ? [{ role: "user", text: norm(message.text) }] : [];
    if (message.type !== "assistant") return [];
    return (message.content ?? []).filter(part => part.type === "text" && norm(part.text)).map(part => ({ role: "assistant", text: norm(part.text) }));
  });
}

let passed = 0;
let total = 0;
for (const file of readdirSync(ORACLE).sort()) {
  let shown;
  let chat;
  let label;
  if (file.endsWith(".txt")) {
    const id = file.slice(0, -4);
    shown = supersetTexts(readFileSync(join(ORACLE, file), "utf8"));
    chat = chats.find(item => item.supersetId === id);
    label = `chat ${id}`;
  } else if (file.startsWith("opencode-") && file.endsWith(".json")) {
    const session = file.slice("opencode-".length, -5);
    shown = openCodeTexts(readFileSync(join(ORACLE, file), "utf8"));
    chat = chats.find(item => item.nativeKey === `opencode:${session}`);
    label = `terminal ${session}`;
  } else continue;
  total += 1;
  const problems = [];
  if (!chat) problems.push("no Bridge chat");
  else {
    let at = 0;
    for (const want of shown) {
      const found = chat.entries.findIndex((entry, index) => index >= at && entry.role === want.role && norm(entry.text).includes(want.text));
      if (found < 0) { problems.push(`missing ${want.role}: ${want.text.slice(0, 60)}`); break; }
      // Superset prints a reply's earlier versions too; they are inside the final one.
      at = found;
    }
  }
  const ok = problems.length === 0;
  if (ok) passed += 1;
  console.log(`${ok ? "PASS" : "FAIL"} ${label} (${shown.length} texts, ${chat?.source ?? "-"})${ok ? "" : `: ${problems.join("; ")}`}`);
}
const score = total ? Math.round((passed / total) * 100) : 0;
console.log(`SCORE ${passed}/${total} (${score}%)`);
process.exit(passed === total && total > 0 ? 0 : 1);

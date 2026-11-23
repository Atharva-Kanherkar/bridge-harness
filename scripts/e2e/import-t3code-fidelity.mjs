#!/usr/bin/env node
// Import fidelity eval for T3 Code: does each thread's chat in Bridge say what
// T3 Code itself shows for that thread?
//
//   T3=<t3 binary> T3_BASE=<T3 Code base dir with a running `t3 serve`> \
//   BRIDGE_DUMP=<json from: T3_EVAL_OUT=<file> cargo test -p bridge-core --lib -- --ignored dump_the_t3code_import_for_the_eval> \
//     node scripts/e2e/import-t3code-fidelity.mjs
//
// The oracle is T3 Code's own CLI (`t3 thread show --json`), not its database,
// so the reader under test is not checking itself. For every thread T3 Code
// lists, the Bridge chat with its title must exist and every user and
// assistant text T3 Code shows must appear in that chat, same role, in order.
// Prints one line per thread and a score; exits non-zero below 100%.

import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";

const { T3, T3_BASE, BRIDGE_DUMP } = process.env;
if (!T3 || !T3_BASE || !BRIDGE_DUMP) throw new Error("Set T3, T3_BASE and BRIDGE_DUMP");
const t3 = (...args) => JSON.parse(execFileSync(T3, [...args, "--base-dir", T3_BASE, "--json"], { encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] }));
const norm = text => String(text ?? "").replace(/\s+/g, " ").trim();
const chats = JSON.parse(readFileSync(BRIDGE_DUMP, "utf8"));

const projects = [...new Set(chats.map(chat => chat.project).filter(Boolean))];
const threads = [];
for (const project of projects) {
  for (const thread of t3("thread", "list", "--project", project)) threads.push(thread);
}
const seen = new Set();
let passed = 0;
let total = 0;
for (const listed of threads) {
  const id = listed.threadId ?? listed.id;
  if (!id || seen.has(id)) continue;
  seen.add(id);
  total += 1;
  const shown = t3("thread", "show", id, "--messages", "500");
  const title = norm(shown.thread?.title ?? listed.title);
  // Threads can share a title: try each unused chat with it, keep the first that matches.
  const check = chat => {
    const problems = [];
    const byRole = role => norm(chat.entries.filter(entry => entry.role === role).map(entry => entry.text).join(" "));
    const haystack = { user: byRole("user"), assistant: byRole("assistant") };
    const at = { user: 0, assistant: 0 };
    for (const message of shown.messages ?? []) {
      const text = norm(message.text);
      if (!text) continue;
      const found = haystack[message.role]?.indexOf(text, at[message.role]) ?? -1;
      if (found < 0) problems.push(`${message.role} text missing or out of order: ${JSON.stringify(text.slice(0, 60))}`);
      else at[message.role] = found + text.length;
    }
    return problems;
  };
  const candidates = chats.filter(item => !item.used && norm(item.title) === title);
  let chat = null;
  let problems = candidates.length === 0 ? ["no Bridge chat with this title"] : null;
  for (const candidate of candidates) {
    const found = check(candidate);
    if (found.length === 0 || problems === null) { chat = candidate; problems = found; }
    if (found.length === 0) break;
  }
  if (chat && problems.length === 0) chat.used = true;
  if (problems.length === 0) passed += 1;
  console.log(`${problems.length === 0 ? "PASS" : "FAIL"} ${title} (${chat?.source ?? "missing"}, ${(shown.messages ?? []).length} messages)${problems.length ? `: ${problems.join("; ")}` : ""}`);
}
const score = total === 0 ? 0 : (passed / total) * 100;
console.log(`SCORE ${passed}/${total} = ${score.toFixed(1)}%`);
process.exit(score === 100 && total > 0 ? 0 : 1);

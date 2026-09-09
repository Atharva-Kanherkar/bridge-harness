#!/usr/bin/env node
// Measure what a session-forest snapshot costs to send, per chat, against a
// real bridge.db.
//
// `sessions/get_session_forest` crosses to the UI as one newline-delimited
// JSON frame, and bridge-client caps that frame at 64 MB. This reports the
// bytes each chat's snapshot would carry before and after the display window
// in `store::session_entry_window`, so the claim "opening an old chat is slow
// or impossible" can be checked rather than asserted.
//
//   node scripts/measure-forest-snapshot.mjs [path/to/bridge.db]
//
// Defaults to the macOS app's database. It opens read-only while still letting
// SQLite read the live WAL, so the measurement sees a consistent snapshot.

import { DatabaseSync } from "node:sqlite";
import { homedir } from "node:os";
import { join } from "node:path";

// Kept in step with the Rust constants they mirror; the point of this script
// is to model the shipped behaviour, so a drift here makes it lie.
const ENTRY_WINDOW = 1500; // store::SNAPSHOT_ENTRY_WINDOW
const STRING_CAP = 4 * 1024; // store::SNAPSHOT_STRING_BYTES
const MAX_FRAME = 64 * 1024 * 1024; // bridge_client::MAX_SERVER_FRAME_BYTES

const path = process.argv[2]
  ?? join(homedir(), "Library/Application Support/dev.bridge.deck/bridge.db");

/** The Rust trimmer's shape: shorten oversized strings, keep the structure. */
function trim(value, key) {
  if (typeof value === "string") {
    if (key === "dataUri" && value.startsWith("data:image/")) return value;
    return value.length > STRING_CAP
      ? `${value.slice(0, STRING_CAP)}\n… ${value.length - STRING_CAP} more bytes not shown`
      : value;
  }
  if (Array.isArray(value)) return value.map(item => trim(item));
  if (value && typeof value === "object") {
    return Object.fromEntries(Object.entries(value).map(([name, item]) => [name, trim(item, name)]));
  }
  return value;
}

const db = new DatabaseSync(path, { readOnly: true });
const sessionIds = db.prepare(
  "SELECT session_id FROM session_heads WHERE active_entry_id IS NOT NULL",
).all();
const branchStats = db.prepare(`
  WITH RECURSIVE active_branch(id,parent_entry_id,sequence) AS (
    SELECT id,parent_entry_id,sequence FROM session_entries
    WHERE session_id=? AND id=?
    UNION
    SELECT parent.id,parent.parent_entry_id,parent.sequence
    FROM session_entries parent
    JOIN active_branch child ON parent.id=child.parent_entry_id
    WHERE parent.session_id=?
  )
  SELECT count(*) AS entries, sum(length(entry.payload)) AS bytes
  FROM active_branch branch
  JOIN session_entries entry ON entry.id=branch.id
`);
const window_ = db.prepare(`
  WITH RECURSIVE active_branch(id,parent_entry_id,sequence) AS (
    SELECT id,parent_entry_id,sequence FROM session_entries
    WHERE session_id=? AND id=?
    UNION
    SELECT parent.id,parent.parent_entry_id,parent.sequence
    FROM session_entries parent
    JOIN active_branch child ON parent.id=child.parent_entry_id
    WHERE parent.session_id=?
  )
  SELECT entry.payload
  FROM active_branch branch
  JOIN session_entries entry ON entry.id=branch.id
  ORDER BY branch.sequence DESC
  LIMIT ?
`);
const head = db.prepare("SELECT active_entry_id FROM session_heads WHERE session_id=?");

const rows = [];
let unopenableBefore = 0;
let unopenableAfter = 0;
for (const { session_id } of sessionIds) {
  const activeHead = head.get(session_id).active_entry_id;
  const session = { session_id, ...branchStats.get(session_id, activeHead, session_id) };
  let after = 0;
  for (const { payload } of window_.all(session_id, activeHead, session_id, ENTRY_WINDOW)) {
    try {
      after += JSON.stringify(trim(JSON.parse(payload))).length;
    } catch {
      after += payload.length; // unparseable payloads travel as stored
    }
  }
  if (session.bytes > MAX_FRAME) unopenableBefore += 1;
  if (after > MAX_FRAME) unopenableAfter += 1;
  rows.push({ ...session, after });
}
rows.sort((left, right) => Number(right.bytes - left.bytes));

const mb = value => `${(value / 1048576).toFixed(1)} MB`;
console.log(`${path}\n${rows.length} sessions with durable active history\n`);
console.log("worst by payload sent:");
for (const row of rows.slice(0, 10)) {
  console.log(
    `  ${row.session_id.slice(0, 8)}  ${String(row.entries).padStart(6)} entries  `
    + `${mb(row.bytes).padStart(9)} -> ${mb(row.after).padStart(8)}`,
  );
}
const total = rows.reduce((sum, row) => sum + row.bytes, 0);
const trimmed = rows.reduce((sum, row) => sum + row.after, 0);
console.log(`\ntotal across all chats: ${mb(total)} -> ${mb(trimmed)}`);
console.log(`worst single chat:      ${mb(Math.max(0, ...rows.map(row => row.bytes)))} -> ${mb(Math.max(0, ...rows.map(row => row.after)))}`);
console.log(`over the ${mb(MAX_FRAME)} frame limit (cannot open at all): ${unopenableBefore} -> ${unopenableAfter}`);

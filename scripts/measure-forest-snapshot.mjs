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
// Defaults to the macOS app's database. Read-only: opens an immutable URI, so
// it is safe to run against a live database while the app is using it.

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
function trim(value) {
  if (typeof value === "string") {
    return value.length > STRING_CAP
      ? `${value.slice(0, STRING_CAP)}\n… ${value.length - STRING_CAP} more bytes not shown`
      : value;
  }
  if (Array.isArray(value)) return value.map(trim);
  if (value && typeof value === "object") {
    return Object.fromEntries(Object.entries(value).map(([key, item]) => [key, trim(item)]));
  }
  return value;
}

const db = new DatabaseSync(`file:${path}?immutable=1`, { readOnly: true });
const sessions = db.prepare(
  `SELECT session_id, count(*) AS entries, sum(length(payload)) AS bytes
     FROM session_entries GROUP BY session_id ORDER BY bytes DESC`,
).all();
const window_ = db.prepare(
  "SELECT payload FROM session_entries WHERE session_id=? ORDER BY sequence DESC LIMIT ?",
);

const rows = [];
let unopenableBefore = 0;
let unopenableAfter = 0;
for (const session of sessions) {
  let after = 0;
  for (const { payload } of window_.all(session.session_id, ENTRY_WINDOW)) {
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

const mb = value => `${(value / 1048576).toFixed(1)} MB`;
console.log(`${path}\n${sessions.length} sessions with durable history\n`);
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

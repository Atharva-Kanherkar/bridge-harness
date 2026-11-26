import { test } from "node:test";
import assert from "node:assert/strict";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { checkIssueFormat } from "../issue-format.mjs";
import { isConventionalPullRequestTitle } from "../check-pull-request-title.mjs";
import {
  LABEL,
  MARKER,
  WATCHED,
  compare,
  drizzleMigrations,
  findIssue,
  github,
  issueBody,
  issueTitle,
  observe,
  parseArgs,
  parseTables,
  parseTsConstant,
  parseVersions,
  readChecked,
  run,
  syncIssue,
  t3Migrations,
  tableChanges,
} from "../import-schema-drift.mjs";

const root = fileURLToPath(new URL("../../", import.meta.url));
const SHA = "d5d5c17ec0000000000000000000000000000000";
const watch = (id) => WATCHED.find((candidate) => candidate.id === id);

// The store.rs shapes, as the Rust side writes them.
const T3_STORE = `
/// The migration ids Bridge reads.
pub const MIGRATIONS: Versions = Versions { oldest: 5, newest_checked: 119 };
/// The first migration of the v2 era.
pub const FIRST_V2_MIGRATION: i64 = 55;

/// The only tables Bridge reads from T3 Code's database.
pub const ALLOWED_TABLES: &[&str] = &[
    "effect_sql_migrations",
    "projection_projects",
    "orchestration_v2_projection_threads",
];
pub const DENIED_TABLES: &[&str] = &["auth_sessions"];
`;
const SUPERSET_STORE = `
pub const HOST_MIGRATIONS: Versions = Versions { oldest: 1_773_188_543_980, newest_checked: 1_792_467_855_578 };
pub const CHAT_MIGRATIONS: Versions = Versions {
    oldest: 1_785_962_456_515,
    newest_checked: 1_794_194_559_956,
};
pub const HOST_TABLES: &[&str] = &[
    "__drizzle_migrations",
    "projects",
    "workspaces",
];
pub const CHAT_TABLES: &[&str] = &["__drizzle_migrations", "chat_sessions_local", "chat_journal"];
`;
const ORCA_STORE = `
pub const CHAT_VERSIONS: Versions = Versions { oldest: 1, newest_checked: 2 };
pub const PROFILE_VERSIONS: Versions = Versions { oldest: 1, newest_checked: 3 };
pub const CHAT_TABLES: &[&str] = &["agent_session_records", "agent_session_store_meta", "journal_chat_state", "journal_items"];
pub const PROFILE_TABLES: &[&str] = &["profile_state_documents", "profile_state_document_parts", "profile_state_document_parts_meta"];
`;

// Real migrations, trimmed: pingdotgg/t3code apps/server/src/persistence/Migrations.
const T3_104 = `import * as Effect from "effect/Effect";
import * as SqlClient from "effect/sql/SqlClient";

export default Effect.gen(function* () {
  const sql = yield* SqlClient.SqlClient;
  // Earlier builds of this change shipped the column under another migration id.
  const columns = yield* sql<{ readonly name: string }>\`
    PRAGMA table_info(projection_projects)
  \`;
  if (columns.some((column) => column.name === "tracked_folders_json")) return;
  yield* sql\`
    ALTER TABLE projection_projects
    ADD COLUMN tracked_folders_json TEXT NOT NULL DEFAULT '[]'
  \`;
});
`;
const T3_110 = `export default Effect.gen(function* () {
  const sql = yield* SqlClient.SqlClient;
  yield* sql\`
    CREATE TABLE workflow_runs (
      run_id TEXT PRIMARY KEY,
      workflow TEXT NOT NULL,
      -- What a run was started with, for the cleanup of a cancelled run whose body is gone.
      input_json TEXT
    )
  \`;
  yield* sql\`CREATE INDEX workflow_runs_status ON workflow_runs (status, updated_at)\`;
  // A key names one run of a workflow whatever its version: the row that takes it is that run.
  yield* sql\`CREATE UNIQUE INDEX workflow_runs_key ON workflow_runs (workflow, idempotency_key)\`;
  yield* sql\`CREATE INDEX workflow_waits_key ON workflow_waits (workflow, key)\`;
});
`;
const T3_106 = `export default Effect.gen(function* () {
  const sql = yield* SqlClient.SqlClient;

  // Keep this expression identical to ProjectionStore's loose worktree filter.
  yield* sql\`
    CREATE INDEX IF NOT EXISTS orchestration_v2_projection_threads_worktree_path_key_idx
    ON orchestration_v2_projection_threads(
      CASE WHEN json_valid(payload_json)
        THEN lower(rtrim(replace(trim(json_extract(payload_json, '$.worktreePath')), '\\\\', '/'), '/'))
      END
    )
  \`;
});
`;
const T3_102 = `  yield* sql\`
    INSERT INTO orchestration_v2_projection_thread_item_counts (thread_id, item_count)
    SELECT counts.thread_id, SUM(counts.item_count)
    FROM orchestration_v2_projection_turn_item_counts counts
    GROUP BY counts.thread_id
  \`;
  yield* sql\`
    CREATE TRIGGER orchestration_v2_thread_item_counts_count_update
    AFTER UPDATE ON orchestration_v2_projection_turn_item_counts
    BEGIN
      UPDATE orchestration_v2_projection_thread_item_counts
      SET item_count = item_count - old.item_count
      WHERE thread_id = old.thread_id;
      INSERT INTO orchestration_v2_projection_thread_item_counts (thread_id, item_count)
      SELECT new.thread_id, new.item_count
      ON CONFLICT (thread_id) DO UPDATE SET item_count = item_count + excluded.item_count;
      DELETE FROM orchestration_v2_projection_thread_item_counts
      WHERE thread_id = old.thread_id AND item_count <= 0;
    END
  \`;
  yield* sql\`DROP TABLE IF EXISTS auth_pairing_links\`;
`;

// Real drizzle migrations, trimmed: superset-sh/superset packages/host-service/drizzle.
const DRIZZLE_0019 = `PRAGMA foreign_keys=OFF;--> statement-breakpoint
CREATE TABLE \`__new_workspaces\` (
	\`id\` text PRIMARY KEY NOT NULL,
	\`project_id\` text,
	\`worktree_path\` text NOT NULL,
	FOREIGN KEY (\`project_id\`) REFERENCES \`projects\`(\`id\`) ON UPDATE no action ON DELETE cascade
);
--> statement-breakpoint
INSERT INTO \`__new_workspaces\`("id", "project_id", "worktree_path") SELECT "id", "project_id", "worktree_path" FROM \`workspaces\`;--> statement-breakpoint
DROP TABLE \`workspaces\`;--> statement-breakpoint
ALTER TABLE \`__new_workspaces\` RENAME TO \`workspaces\`;--> statement-breakpoint
PRAGMA foreign_keys=ON;--> statement-breakpoint
CREATE INDEX \`workspaces_project_id_idx\` ON \`workspaces\` (\`project_id\`);`;
const DRIZZLE_0037 = "ALTER TABLE `projects` ADD `deleted_at` integer;--> statement-breakpoint\nALTER TABLE `projects` ADD `deleted_by_user_id` text;";
const DRIZZLE_0038 = `CREATE TABLE \`mobile_last_devices\` (
	\`workspace_id\` text PRIMARY KEY NOT NULL,
	FOREIGN KEY (\`workspace_id\`) REFERENCES \`workspaces\`(\`id\`) ON UPDATE no action ON DELETE cascade
);
--> statement-breakpoint
UPDATE \`workspaces\` SET \`type\` = 'local' WHERE \`type\` = 'main';`;

function tempRoot(stores = {}) {
  const dir = mkdtempSync(join(tmpdir(), "import-schema-drift-"));
  // `null` leaves a store file out.
  const pick = (key, fallback) => (key in stores ? stores[key] : fallback);
  const files = {
    "src-tauri/bridge-core/src/t3code_import/store.rs": pick("t3", T3_STORE),
    "src-tauri/bridge-core/src/superset_import/store.rs": pick("superset", SUPERSET_STORE),
    "src-tauri/bridge-core/src/orca_import/store.rs": pick("orca", ORCA_STORE),
  };
  for (const [file, text] of Object.entries(files)) {
    if (text === null) continue;
    mkdirSync(dirname(join(dir, file)), { recursive: true });
    writeFileSync(join(dir, file), text);
  }
  return dir;
}

/** A fetch that answers from `routes` (`"GET /path?query"`) and records every call. */
function fakeFetch(routes) {
  const calls = [];
  const fetch = async (url, init = {}) => {
    const { pathname, search } = new URL(url);
    const method = init.method ?? "GET";
    const call = { method, path: `${pathname}${search}`, headers: init.headers, body: init.body ? JSON.parse(init.body) : undefined };
    calls.push(call);
    const route = routes[`${method} ${pathname}${search}`] ?? routes[`${method} ${pathname}`];
    if (route === undefined) return new Response(JSON.stringify({ message: "Not Found" }), { status: 404 });
    const answer = typeof route === "function" ? route(call) : route;
    const { status = 200, body = "" } = answer;
    return new Response(typeof body === "string" ? body : JSON.stringify(body), { status });
  };
  return { fetch, calls };
}

const listing = (names) =>
  names.map((name) => ({
    name,
    type: name.includes(".") ? "file" : "dir",
    path: `apps/server/src/persistence/Migrations/${name}`,
  }));

function upstreamRoutes({ t3 = ["119_WorkflowRunSubject.ts", "120_Accounts.ts"], host, chat, orcaChat = 2, orcaProfile = 3 } = {}) {
  const contents = (repo, path) => `GET /repos/${repo}/contents/${path}`;
  const hostEntries = host ?? [
    { idx: 38, when: 1_792_467_855_578, tag: "0038_mobile_devices" },
  ];
  const chatEntries = chat ?? [{ idx: 5, when: 1_794_194_559_956, tag: "0005_chat_sync_state" }];
  return {
    "GET /repos/pingdotgg/t3code/commits/HEAD": { body: SHA },
    "GET /repos/superset-sh/superset/commits/HEAD": { body: SHA },
    "GET /repos/stablyai/orca/commits/HEAD": { body: SHA },
    [contents("pingdotgg/t3code", "apps/server/src/persistence/Migrations")]: { body: listing(t3) },
    [contents("pingdotgg/t3code", "apps/server/src/persistence/Migrations/120_Accounts.ts")]: {
      body: "yield* sql`ALTER TABLE projection_projects ADD COLUMN x TEXT`;\nyield* sql`CREATE TABLE accounts (token TEXT)`;",
    },
    [contents("superset-sh/superset", "packages/host-service/drizzle/meta/_journal.json")]: { body: { entries: hostEntries } },
    [contents("superset-sh/superset", "packages/host-service/drizzle/0039_tags.sql")]: { body: "CREATE TABLE `tags` (`id` text);" },
    [contents("superset-sh/superset", "packages/chat-runtime/src/db/drizzle/meta/_journal.json")]: { body: { entries: chatEntries } },
    [contents("stablyai/orca", "src/main/native-chat/agent-session-journal/database/journal-domain-migrations.ts")]: {
      body: `export const JOURNAL_DOMAIN_SCHEMA_VERSION = ${orcaChat}\n`,
    },
    [contents("stablyai/orca", "src/main/persistence/profile-state/database/profile-state-database-schema.ts")]: {
      body: `export const PROFILE_STATE_DATABASE_SCHEMA_VERSION = ${orcaProfile}\n`,
    },
  };
}

// --- What Bridge checked ----------------------------------------------------

test("parses the Versions constants, with digit separators and across lines", () => {
  assert.deepEqual(parseVersions(T3_STORE, "MIGRATIONS"), { oldest: 5, newest_checked: 119 });
  assert.deepEqual(parseVersions(SUPERSET_STORE, "HOST_MIGRATIONS"), {
    oldest: 1_773_188_543_980,
    newest_checked: 1_792_467_855_578,
  });
  assert.deepEqual(parseVersions(SUPERSET_STORE, "CHAT_MIGRATIONS"), {
    oldest: 1_785_962_456_515,
    newest_checked: 1_794_194_559_956,
  });
  assert.deepEqual(parseVersions(ORCA_STORE, "PROFILE_VERSIONS"), { oldest: 1, newest_checked: 3 });
});

test("does not mistake a commented-out constant for the real one", () => {
  const source = "// pub const MIGRATIONS: Versions = Versions { oldest: 1, newest_checked: 2 };\n" + T3_STORE;
  assert.deepEqual(parseVersions(source, "MIGRATIONS"), { oldest: 5, newest_checked: 119 });
});

test("a missing or reshaped Versions constant throws naming the file and the constant", () => {
  const file = "src-tauri/bridge-core/src/t3code_import/store.rs";
  assert.throws(() => parseVersions("pub const NEWEST_MIGRATION: i64 = 102;", "MIGRATIONS", file), (error) => {
    assert.match(error.message, /t3code_import\/store\.rs/);
    assert.match(error.message, /MIGRATIONS/);
    return true;
  });
  assert.throws(
    () => parseVersions("pub const CHAT_VERSIONS: RangeInclusive<i64> = 1..=2;", "CHAT_VERSIONS", "orca/store.rs"),
    /orca\/store\.rs: no `pub const CHAT_VERSIONS: Versions/,
  );
  assert.throws(
    () => parseVersions("pub const MIGRATIONS: Versions = Versions { oldest: 5 };", "MIGRATIONS", "x.rs"),
    /x\.rs: `MIGRATIONS` has no `newest_checked`/,
  );
});

test("parses the table allowlists and throws when one is missing", () => {
  assert.deepEqual(parseTables(T3_STORE, "ALLOWED_TABLES"), [
    "effect_sql_migrations",
    "projection_projects",
    "orchestration_v2_projection_threads",
  ]);
  assert.deepEqual(parseTables(SUPERSET_STORE, "CHAT_TABLES"), ["__drizzle_migrations", "chat_sessions_local", "chat_journal"]);
  assert.throws(() => parseTables(T3_STORE, "HOST_TABLES", "t3/store.rs"), /t3\/store\.rs: no `pub const HOST_TABLES/);
});

test("reads every watched database from the store files", () => {
  const dir = tempRoot();
  try {
    const checked = readChecked(dir);
    assert.deepEqual(Object.keys(checked).sort(), WATCHED.map((w) => w.id).sort());
    assert.equal(checked.t3code.versions.newest_checked, 119);
    assert.equal(checked["superset-host"].versions.newest_checked, 1_792_467_855_578);
    assert.deepEqual(checked["orca-chat"].reads, ["agent_session_records", "agent_session_store_meta", "journal_chat_state", "journal_items"]);
    assert.deepEqual(checked["orca-profile"].versions, { oldest: 1, newest_checked: 3 });
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("a missing store file is a clear error", () => {
  const dir = tempRoot({ orca: null });
  try {
    assert.throws(() => readChecked(dir), /orca_import\/store\.rs: cannot read it/);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

// Expected to pass once the Rust side carries the `Versions` constants.
test("parses the real store files in this repository", () => {
  const checked = readChecked(root);
  for (const watched of WATCHED) {
    const { versions, reads } = checked[watched.id];
    assert.ok(Number.isSafeInteger(versions.oldest) && versions.oldest > 0, watched.id);
    assert.ok(versions.oldest <= versions.newest_checked, watched.id);
    assert.ok(reads.length > 0, watched.id);
  }
});

// --- Upstream ----------------------------------------------------------------

test("finds T3 Code's head in a directory listing, ignoring tests, folders and helpers", () => {
  const migrations = t3Migrations(
    listing([
      "001_OrchestrationEvents.ts",
      "055_OrchestrationV2.application-event-source.test.ts",
      "055_OrchestrationV2.ts",
      "102_ThreadItemCounts.test.ts",
      "102_ThreadItemCounts.ts",
      "119_WorkflowRunSubject.ts",
      "OrchestrationV2",
      "legacyThreadPullRequestKey.ts",
    ]),
  );
  assert.deepEqual(
    migrations.map((m) => m.label),
    ["001_OrchestrationEvents", "055_OrchestrationV2", "102_ThreadItemCounts", "119_WorkflowRunSubject"],
  );
  assert.equal(migrations.at(-1).version, 119);
  assert.equal(migrations.at(-1).path, "apps/server/src/persistence/Migrations/119_WorkflowRunSubject.ts");
  assert.throws(() => t3Migrations(listing(["OrchestrationV2", "README.md"])), /no NNN_Name\.ts migrations/);
  assert.throws(() => t3Migrations({ message: "Not Found" }), /not a list/);
});

test("reads drizzle journals by `when` and Orca's version constant", () => {
  const journal = {
    entries: [
      { idx: 1, when: 20, tag: "0001_b" },
      { idx: 0, when: 10, tag: "0000_a" },
    ],
  };
  assert.deepEqual(drizzleMigrations(journal), [
    { version: 10, label: "0000_a", idx: 0 },
    { version: 20, label: "0001_b", idx: 1 },
  ]);
  assert.throws(() => drizzleMigrations({ entries: [{ tag: "0000_a" }] }, "j.json"), /j\.json: an entry has no numeric `when`/);
  assert.equal(parseTsConstant("export const PROFILE_STATE_DATABASE_SCHEMA_VERSION = 3\n", "PROFILE_STATE_DATABASE_SCHEMA_VERSION", "f.ts"), 3);
  assert.throws(() => parseTsConstant("const X = 3", "PROFILE_STATE_DATABASE_SCHEMA_VERSION", "f.ts"), /f\.ts: no `export const/);
});

test("extracts the tables a T3 Code migration changes", () => {
  assert.deepEqual(tableChanges(T3_104), [{ table: "projection_projects", ops: ["alter"] }]);
  assert.deepEqual(tableChanges(T3_110), [
    { table: "workflow_runs", ops: ["create", "index"] },
    { table: "workflow_waits", ops: ["index"] },
  ]);
  assert.deepEqual(tableChanges(T3_106), [{ table: "orchestration_v2_projection_threads", ops: ["index"] }]);
  assert.deepEqual(tableChanges(T3_102), [
    { table: "auth_pairing_links", ops: ["drop"] },
    { table: "orchestration_v2_projection_thread_item_counts", ops: ["insert", "update", "delete"] },
    { table: "orchestration_v2_projection_turn_item_counts", ops: ["trigger"] },
  ]);
});

test("extracts the tables a drizzle migration changes, folding __new_ rebuilds", () => {
  assert.deepEqual(tableChanges(DRIZZLE_0019), [{ table: "workspaces", ops: ["create", "rename", "alter", "drop", "index", "insert"] }]);
  assert.deepEqual(tableChanges(DRIZZLE_0037), [{ table: "projects", ops: ["alter"] }]);
  assert.deepEqual(tableChanges(DRIZZLE_0038), [
    { table: "mobile_last_devices", ops: ["create"] },
    { table: "workspaces", ops: ["update"] },
  ]);
});

test("ignores tables named only in comments", () => {
  const source = "// ALTER TABLE ghosts ADD x\n/* DROP TABLE spirits */\nsql`-- UPDATE phantoms SET x = 1\nSELECT 1`";
  assert.deepEqual(tableChanges(source), []);
});

test("observes T3 Code at a pinned commit and fetches only the new migrations", async () => {
  const { fetch, calls } = fakeFetch(upstreamRoutes());
  const client = github({ token: "t", fetch });
  const observation = await observe(watch("t3code"), { versions: { oldest: 5, newest_checked: 119 }, reads: ["projection_projects"] }, client);
  assert.equal(observation.head, 120);
  assert.equal(observation.sha, SHA);
  assert.deepEqual(observation.migrations.map((m) => m.label), ["120_Accounts"]);
  assert.deepEqual(observation.migrations[0].changes, [
    { table: "accounts", ops: ["create"] },
    { table: "projection_projects", ops: ["alter"] },
  ]);
  assert.ok(calls.every((call) => call.headers.Authorization === "Bearer t"));
  const contents = calls.filter((call) => call.path.includes("/contents/"));
  assert.ok(contents.every((call) => call.path.endsWith(`?ref=${SHA}`)), "every read is pinned to one commit");
  assert.equal(calls.some((call) => call.path.includes("119_WorkflowRunSubject")), false);
});

// --- Compare and the issue -------------------------------------------------

function observations(options) {
  const dir = tempRoot();
  const { fetch } = fakeFetch(upstreamRoutes(options));
  const client = github({ fetch });
  try {
    const checked = readChecked(dir);
    return Promise.all(WATCHED.map((w) => observe(w, checked[w.id], client)));
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
}

test("no drift when every upstream head equals what Bridge checked", async () => {
  const { drifts, notes } = compare(await observations({ t3: ["119_WorkflowRunSubject.ts"] }));
  assert.deepEqual(drifts, []);
  assert.deepEqual(notes, []);
});

test("an upstream older than what Bridge checked is a note, not drift", async () => {
  const { drifts, notes } = compare(await observations({ t3: ["118_WorkflowPools.ts"], orcaProfile: 2 }));
  assert.deepEqual(drifts, []);
  assert.equal(notes.length, 2);
  assert.match(notes[0], /T3 Code: upstream is at migration 118, older than the 119 Bridge checked/);
  assert.match(notes[1], /Orca profile-state\.db: upstream is at version 2/);
});

test("drift names the new migrations and flags tables Bridge reads", async () => {
  const host = [
    { idx: 38, when: 1_792_467_855_578, tag: "0038_mobile_devices" },
    { idx: 39, when: 1_793_000_000_000, tag: "0039_tags" },
  ];
  const { drifts } = compare(await observations({ host, orcaChat: 3 }));
  assert.deepEqual(drifts.map((d) => d.id), ["t3code", "superset-host", "orca-chat"]);

  const [t3, superset, orca] = drifts;
  assert.equal(t3.checked, 119);
  assert.equal(t3.head, 120);
  assert.equal(t3.touchesReads, true);
  assert.deepEqual(t3.readsTouched, ["projection_projects"]);
  assert.deepEqual(t3.newTables, ["accounts"]);
  assert.deepEqual(t3.migrations[0].tables, [
    { table: "accounts", ops: ["create"], read: false },
    { table: "projection_projects", ops: ["alter"], read: true },
  ]);

  assert.equal(superset.checkedLabel, "0038_mobile_devices");
  assert.equal(superset.headLabel, "0039_tags");
  assert.equal(superset.touchesReads, false);
  assert.equal(superset.migrations[0].idx, 39);

  assert.equal(orca.head, 3);
  assert.equal(orca.migrations, null);
  assert.equal(orca.touchesReads, null);
});

test("the issue passes the issue-format gate and bolds the tables Bridge reads", async () => {
  const host = [
    { idx: 38, when: 1_792_467_855_578, tag: "0038_mobile_devices" },
    { idx: 39, when: 1_793_000_000_000, tag: "0039_tags" },
  ];
  const { drifts } = compare(await observations({ host, orcaChat: 3 }));
  const body = issueBody(drifts);
  const result = checkIssueFormat(body);
  assert.equal(result.ok, true, result.problems.join("\n"));
  assert.equal(result.exempt, false);
  assert.ok(body.startsWith(MARKER));
  assert.ok(body.indexOf("## For humans") < body.indexOf("## For agents"));
  assert.match(body, /\*\*`projection_projects`\*\* \(alter\)/);
  assert.match(body, /`accounts` \(create\)/);
  assert.doesNotMatch(body, /\*\*`accounts`\*\*/);
  assert.match(body, /`MIGRATIONS\.newest_checked` to `120`/);
  assert.match(body, /schema-120\.sql/);
  assert.match(body, /DENIED_TABLES/);
  assert.match(body, /host-39\.sql/);
  assert.match(body, /`CHAT_VERSIONS\.newest_checked` to `3`/);
  assert.match(body, /Conductor is not watched/);
  assert.match(body, /still read when every table and column Bridge reads is present/);
  assert.doesNotMatch(body, /```mermaid/);

  const spec = JSON.parse(/```diagram\n(.+)\n```/.exec(body)[1]);
  assert.ok(spec.nodes.length > 0 && spec.edges.length > 0 && spec.caption && spec.ariaLabel);
  const ids = new Set(spec.nodes.map((node) => node.id));
  for (const node of spec.nodes) {
    assert.equal(typeof node.row, "number");
    assert.equal(typeof node.col, "number");
    assert.ok(node.label.length <= 18, node.label);
  }
  for (const edge of spec.edges) assert.ok(ids.has(edge.from) && ids.has(edge.to));
});

test("the title is a conventional commit title", async () => {
  const { drifts } = compare(await observations());
  const title = issueTitle(drifts);
  assert.equal(title, "chore(import): T3 Code moved to migration 120, Bridge checked 119");
  assert.ok(isConventionalPullRequestTitle(title));

  const several = compare(await observations({ orcaProfile: 4 })).drifts;
  const combined = issueTitle(several);
  assert.equal(
    combined,
    "chore(import): T3 Code moved to migration 120 (Bridge checked 119), Orca profile-state.db to version 4 (checked 3)",
  );
  assert.ok(isConventionalPullRequestTitle(combined));
});

// --- The issue on GitHub ---------------------------------------------------

const ISSUES = `GET /repos/o/r/issues?state=open&labels=${LABEL}&per_page=100`;

test("updates the open drift issue instead of opening another", async () => {
  const { fetch, calls } = fakeFetch({
    [ISSUES]: {
      body: [
        { number: 3, title: "labeled by hand", body: "not ours" },
        { number: 7, title: "old title", body: `${MARKER}\nold body` },
        { number: 9, title: "a pull request", body: MARKER, pull_request: {} },
      ],
    },
    "PATCH /repos/o/r/issues/7": { body: { number: 7 } },
  });
  const result = await syncIssue(github({ token: "t", fetch }), "o/r", { title: "new title", body: `${MARKER}\nnew body` });
  assert.deepEqual(result, { action: "updated", number: 7 });
  const writes = calls.filter((call) => call.method !== "GET");
  assert.deepEqual(writes.map((call) => `${call.method} ${call.path}`), ["PATCH /repos/o/r/issues/7"]);
  assert.deepEqual(writes[0].body, { title: "new title", body: `${MARKER}\nnew body` });
});

test("leaves an up-to-date issue alone", async () => {
  const { fetch, calls } = fakeFetch({ [ISSUES]: { body: [{ number: 7, title: "t", body: `${MARKER}\nb` }] } });
  const result = await syncIssue(github({ token: "t", fetch }), "o/r", { title: "t", body: `${MARKER}\nb` });
  assert.deepEqual(result, { action: "unchanged", number: 7 });
  assert.equal(calls.filter((call) => call.method !== "GET").length, 0);
});

test("opens a labeled issue when none is open, tolerating an existing label", async () => {
  const { fetch, calls } = fakeFetch({
    [ISSUES]: { body: [{ number: 3, title: "labeled by hand", body: "not ours" }] },
    "POST /repos/o/r/labels": { status: 422, body: { message: "Validation Failed" } },
    "POST /repos/o/r/issues": { status: 201, body: { number: 12 } },
  });
  const result = await syncIssue(github({ token: "t", fetch }), "o/r", { title: "t", body: `${MARKER}\nb` });
  assert.deepEqual(result, { action: "created", number: 12 });
  const writes = calls.filter((call) => call.method !== "GET");
  assert.deepEqual(writes.map((call) => `${call.method} ${call.path}`), ["POST /repos/o/r/labels", "POST /repos/o/r/issues"]);
  assert.equal(writes[0].body.name, LABEL);
  assert.deepEqual(writes[1].body, { title: "t", body: `${MARKER}\nb`, labels: [LABEL] });
});

test("an API failure is an error naming the call and the status", async () => {
  const { fetch } = fakeFetch({ [ISSUES]: { status: 403, body: { message: "Resource not accessible by integration" } } });
  await assert.rejects(findIssue(github({ fetch }), "o/r"), /GitHub GET \/repos\/o\/r\/issues\?.* failed: 403 Resource not accessible/);
  const offline = github({ fetch: async () => { throw new Error("getaddrinfo ENOTFOUND api.github.com"); } });
  await assert.rejects(offline.headSha("o/r"), /GitHub GET \/repos\/o\/r\/commits\/HEAD failed: getaddrinfo ENOTFOUND/);
});

test("a dry run prints the issue and writes nothing", async () => {
  const dir = tempRoot();
  const { fetch, calls } = fakeFetch({ ...upstreamRoutes(), [ISSUES]: { body: [{ number: 7, title: "x", body: MARKER }] } });
  const lines = [];
  try {
    const result = await run({ root: dir, client: github({ fetch }), repo: "o/r", dryRun: true, log: (line) => lines.push(line) });
    assert.equal(result.action, "dry-run");
    assert.equal(result.existing.number, 7);
    assert.equal(calls.filter((call) => call.method !== "GET").length, 0);
    const printed = lines.join("\n");
    assert.match(printed, /would update #7/);
    assert.match(printed, /chore\(import\): T3 Code moved to migration 120, Bridge checked 119/);
    assert.match(printed, /## For agents/);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("no drift prints so and writes nothing", async () => {
  const dir = tempRoot();
  const { fetch, calls } = fakeFetch(upstreamRoutes({ t3: ["119_WorkflowRunSubject.ts"] }));
  const lines = [];
  try {
    const result = await run({ root: dir, client: github({ token: "t", fetch }), repo: "o/r", log: (line) => lines.push(line) });
    assert.equal(result.action, "none");
    assert.equal(lines.at(-1), "no drift");
    assert.equal(calls.filter((call) => call.method !== "GET").length, 0);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("a real run needs a repository and a token", async () => {
  await assert.rejects(run({ client: github({ token: "t" }), repo: undefined }), /GITHUB_REPOSITORY is not set/);
  await assert.rejects(run({ client: github({}), repo: "o/r" }), /GITHUB_TOKEN is not set/);
});

test("parses the command line", () => {
  assert.equal(parseArgs([], {}).dryRun, false);
  assert.equal(parseArgs(["--dry-run"], {}).dryRun, true);
  assert.equal(parseArgs([], { DRY_RUN: "true" }).dryRun, true);
  assert.equal(parseArgs([], { DRY_RUN: "false" }).dryRun, false);
  assert.equal(parseArgs(["--root", "/tmp/x"], {}).root, "/tmp/x");
  assert.throws(() => parseArgs(["--root"], {}), /--root needs a directory/);
  assert.throws(() => parseArgs(["--force"], {}), /unknown argument --force/);
});

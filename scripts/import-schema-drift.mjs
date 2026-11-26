#!/usr/bin/env node
// Watches the SQLite schemas Bridge imports from other coding tools.
//
// Bridge reads T3 Code's, Superset's and Orca's databases and gates each on
// `Versions { oldest, newest_checked }` in that tool's store.rs: the newest
// schema version Bridge was checked against with a real fixture. A newer
// database is still read while everything Bridge reads is there, so upstream
// moving on is not an outage, but someone has to read the new migrations and
// move `newest_checked`. This script compares each tool's newest upstream
// version with the one Bridge checked and keeps one GitHub issue open while
// they differ (.github/workflows/import-schema-drift.yml, weekly).
//
// Conductor is not watched: its source is not public.
//
//   node scripts/import-schema-drift.mjs              open or update the issue
//   node scripts/import-schema-drift.mjs --dry-run    print it, write nothing
//   node scripts/import-schema-drift.mjs --root dir   read store.rs from another checkout
//
// GITHUB_TOKEN (optional for reads, required for writes) and
// GITHUB_REPOSITORY come from the environment. DRY_RUN=true is --dry-run.

import { readFileSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

/** Hidden marker that identifies the issue this script owns. */
export const MARKER = '<!-- import-schema-drift -->';

/** Label on that issue; the open-issue lookup filters on it. */
export const LABEL = 'import-schema-drift';

const DEFAULT_ROOT = fileURLToPath(new URL('../', import.meta.url));
const WORKFLOW = '.github/workflows/import-schema-drift.yml';
const SCRIPT = 'scripts/import-schema-drift.mjs';

const T3_STORE = 'src-tauri/bridge-core/src/t3code_import/store.rs';
const SUPERSET_STORE = 'src-tauri/bridge-core/src/superset_import/store.rs';
const ORCA_STORE = 'src-tauri/bridge-core/src/orca_import/store.rs';

/**
 * Every database Bridge imports that has public upstream source.
 *
 * `versions` and `tables` name the Rust constants in `store`; the upstream
 * block says where the tool records its newest schema version.
 */
export const WATCHED = [
  {
    id: 't3code',
    tool: 'T3 Code',
    database: null,
    short: 'T3 Code',
    store: T3_STORE,
    versions: 'MIGRATIONS',
    tables: 'ALLOWED_TABLES',
    upstream: { kind: 't3', repo: 'pingdotgg/t3code', dir: 'apps/server/src/persistence/Migrations' },
  },
  {
    id: 'superset-host',
    tool: 'Superset',
    database: 'host.db',
    short: 'Superset host',
    store: SUPERSET_STORE,
    versions: 'HOST_MIGRATIONS',
    tables: 'HOST_TABLES',
    fixture: 'host',
    upstream: { kind: 'drizzle', repo: 'superset-sh/superset', dir: 'packages/host-service/drizzle' },
  },
  {
    id: 'superset-chat',
    tool: 'Superset',
    database: 'chat.db',
    short: 'Superset chat',
    store: SUPERSET_STORE,
    versions: 'CHAT_MIGRATIONS',
    tables: 'CHAT_TABLES',
    fixture: 'chat',
    upstream: { kind: 'drizzle', repo: 'superset-sh/superset', dir: 'packages/chat-runtime/src/db/drizzle' },
  },
  {
    id: 'orca-chat',
    tool: 'Orca',
    database: 'agent-session-domain.db',
    short: 'Orca chat',
    store: ORCA_STORE,
    versions: 'CHAT_VERSIONS',
    tables: 'CHAT_TABLES',
    upstream: {
      kind: 'constant',
      repo: 'stablyai/orca',
      path: 'src/main/native-chat/agent-session-journal/database/journal-domain-migrations.ts',
      constant: 'JOURNAL_DOMAIN_SCHEMA_VERSION',
    },
  },
  {
    id: 'orca-profile',
    tool: 'Orca',
    database: 'profile-state.db',
    short: 'Orca profile',
    store: ORCA_STORE,
    versions: 'PROFILE_VERSIONS',
    tables: 'PROFILE_TABLES',
    upstream: {
      kind: 'constant',
      repo: 'stablyai/orca',
      path: 'src/main/persistence/profile-state/database/profile-state-database-schema.ts',
      constant: 'PROFILE_STATE_DATABASE_SCHEMA_VERSION',
    },
  },
];

/** "T3 Code", "Superset host.db". */
export function displayName(watch) {
  return watch.database ? `${watch.tool} ${watch.database}` : watch.tool;
}

// ---------------------------------------------------------------------------
// What Bridge checked: the constants in store.rs.

function stripRustComments(source) {
  return String(source).replace(/\/\*[\s\S]*?\*\//g, ' ').replace(/\/\/[^\n]*/g, '');
}

const PUB_CONST = String.raw`\bpub(?:\([^)]*\))?\s+const\s+`;

function rustInt(digits, where) {
  const value = Number(digits.replace(/_/g, ''));
  if (!Number.isSafeInteger(value)) throw new Error(`${where}: ${digits} is not a safe integer`);
  return value;
}

/**
 * `pub const NAME: Versions = Versions { oldest: 5, newest_checked: 119 };`
 * → `{ oldest, newest_checked }`. Throws, naming the file and constant, when
 * the constant is missing or has another shape, so a rename fails loudly.
 */
export function parseVersions(source, constant, file = 'store.rs') {
  const pattern = new RegExp(
    `${PUB_CONST}${constant}\\s*:\\s*(?:[\\w:]+::)?Versions\\s*=\\s*(?:[\\w:]+::)?Versions\\s*\\{([^}]*)\\}`,
  );
  const match = pattern.exec(stripRustComments(source));
  if (!match) {
    throw new Error(
      `${file}: no \`pub const ${constant}: Versions = Versions { oldest: N, newest_checked: N }\`. ` +
        `If it was renamed or reshaped, update ${SCRIPT} to match.`,
    );
  }
  const field = (name) => {
    const found = new RegExp(`\\b${name}\\s*:\\s*(\\d[\\d_]*)`).exec(match[1]);
    if (!found) throw new Error(`${file}: \`${constant}\` has no \`${name}\` number`);
    return rustInt(found[1], `${file}: ${constant}.${name}`);
  };
  const versions = { oldest: field('oldest'), newest_checked: field('newest_checked') };
  if (versions.oldest > versions.newest_checked) {
    throw new Error(`${file}: \`${constant}\` has oldest ${versions.oldest} above newest_checked ${versions.newest_checked}`);
  }
  return versions;
}

/** `pub const NAME: &[&str] = &["a", "b"];` → `["a", "b"]`. Throws like parseVersions. */
export function parseTables(source, constant, file = 'store.rs') {
  const pattern = new RegExp(
    `${PUB_CONST}${constant}\\s*:\\s*&(?:'static\\s+)?\\[\\s*&(?:'static\\s+)?str\\s*\\]\\s*=\\s*&\\[([^\\]]*)\\]`,
  );
  const match = pattern.exec(stripRustComments(source));
  if (!match) throw new Error(`${file}: no \`pub const ${constant}: &[&str] = &[...]\`. If it was renamed, update ${SCRIPT} to match.`);
  const tables = [...match[1].matchAll(/"([^"\\]*)"/g)].map((found) => found[1]);
  if (!tables.length) throw new Error(`${file}: \`${constant}\` lists no tables`);
  return tables;
}

/** Read every watched database's checked versions and allowed tables. */
export function readChecked(root = DEFAULT_ROOT, watched = WATCHED) {
  const sources = new Map();
  const source = (file) => {
    if (!sources.has(file)) {
      try {
        sources.set(file, readFileSync(join(root, file), 'utf8'));
      } catch (error) {
        throw new Error(`${file}: cannot read it (${error.code ?? error.message})`);
      }
    }
    return sources.get(file);
  };
  const checked = {};
  for (const watch of watched) {
    checked[watch.id] = {
      versions: parseVersions(source(watch.store), watch.versions, watch.store),
      reads: parseTables(source(watch.store), watch.tables, watch.store),
    };
  }
  return checked;
}

// ---------------------------------------------------------------------------
// Which tables a migration touches.

const QUOTED = String.raw`[\`"\[]?[A-Za-z_][\w$]*[\`"\]]?`;
const TABLE = String.raw`(?:${QUOTED}\.)?[\`"\[]?([A-Za-z_][\w$]*)[\`"\]]?`;
const OPS = ['create', 'rename', 'alter', 'drop', 'index', 'trigger', 'insert', 'update', 'delete'];
const STATEMENTS = [
  ['create', String.raw`\bCREATE\s+(?:(?:TEMP|TEMPORARY|VIRTUAL)\s+)?TABLE\s+(?:IF\s+NOT\s+EXISTS\s+)?${TABLE}`],
  ['alter', String.raw`\bALTER\s+TABLE\s+${TABLE}`],
  ['rename', String.raw`\bRENAME\s+TO\s+${TABLE}`],
  ['drop', String.raw`\bDROP\s+TABLE\s+(?:IF\s+EXISTS\s+)?${TABLE}`],
  ['index', String.raw`\bCREATE\s+(?:UNIQUE\s+)?INDEX\s+(?:IF\s+NOT\s+EXISTS\s+)?${QUOTED}\s+ON\s+${TABLE}`],
  [
    'trigger',
    String.raw`\bCREATE\s+(?:(?:TEMP|TEMPORARY)\s+)?TRIGGER\s+(?:IF\s+NOT\s+EXISTS\s+)?${QUOTED}\s+` +
      String.raw`(?:(?:BEFORE|AFTER|INSTEAD\s+OF)\s+)?(?:INSERT|DELETE|UPDATE(?:\s+OF\s+[^;]*?)?)\s+ON\s+${TABLE}`,
  ],
  ['insert', String.raw`\b(?:INSERT(?:\s+OR\s+[A-Z]+)?|REPLACE)\s+INTO\s+${TABLE}`],
  ['update', String.raw`\bUPDATE(?:\s+OR\s+[A-Z]+)?\s+${TABLE}\s+SET\b`],
  ['delete', String.raw`\bDELETE\s+FROM\s+${TABLE}`],
].map(([op, source]) => [op, new RegExp(source, 'gi')]);
const NOT_TABLES = new Set(['if', 'not', 'exists', 'select', 'set', 'values', 'on', 'from', 'where']);

// Drops SQL and JavaScript comments (line and block), which name tables in
// prose. Drizzle's `--> statement-breakpoint` goes with them.
export function stripComments(source) {
  return String(source)
    .replace(/\/\*[\s\S]*?\*\//g, ' ')
    .replace(/(^|\s)\/\/[^\n]*/g, '$1')
    .replace(/--[^\n]*/g, ' ');
}

/**
 * The tables a migration's SQL changes, from a T3 Code migration (Effect
 * `sql` templates in TypeScript) or a drizzle `.sql` file. Drizzle rebuilds a
 * table as `__new_<name>`, so that prefix folds into the table it replaces.
 * Reads (`SELECT ... FROM`, `PRAGMA table_info`) are not changes.
 *
 * @returns {{ table: string, ops: string[] }[]} sorted by table
 */
export function tableChanges(source) {
  const text = stripComments(source);
  const found = new Map();
  for (const [op, pattern] of STATEMENTS) {
    for (const match of text.matchAll(pattern)) {
      const table = match[1].toLowerCase().replace(/^__new_/, '');
      if (NOT_TABLES.has(table)) continue;
      if (!found.has(table)) found.set(table, new Set());
      found.get(table).add(op);
    }
  }
  return [...found.keys()].sort().map((table) => ({ table, ops: OPS.filter((op) => found.get(table).has(op)) }));
}

// ---------------------------------------------------------------------------
// Upstream heads.

/**
 * T3 Code's migrations from a GitHub directory listing: `NNN_Name.ts` files,
 * not tests, not folders. Sorted by number.
 */
export function t3Migrations(listing) {
  if (!Array.isArray(listing)) throw new Error('T3 Code: the migrations directory listing is not a list');
  const migrations = [];
  for (const entry of listing) {
    const name = String(entry?.name ?? '');
    if (entry?.type !== 'file' || /\.(?:test|spec|d)\.ts$/.test(name)) continue;
    if (!/^\d+_/.test(name)) continue;
    // The name goes into an issue agents act on: only the shape T3 Code uses.
    const match = /^(\d+)_[A-Za-z0-9_]+\.ts$/.exec(name);
    if (!match) throw new Error(`T3 Code: migration file ${JSON.stringify(name)} is not named NNN_Name.ts`);
    migrations.push({ version: Number(match[1]), label: name.slice(0, -'.ts'.length), path: entry.path });
  }
  if (!migrations.length) throw new Error('T3 Code: no NNN_Name.ts migrations in the directory listing');
  return migrations.sort((a, b) => a.version - b.version);
}

/**
 * A drizzle journal's migrations. `when` is the version: drizzle writes it to
 * `__drizzle_migrations.created_at`, which Bridge compares.
 */
export function drizzleMigrations(journal, where = '_journal.json') {
  const entries = journal?.entries;
  if (!Array.isArray(entries) || !entries.length) throw new Error(`${where}: no \`entries\``);
  return entries
    .map((entry) => {
      if (!Number.isSafeInteger(entry?.when) || typeof entry?.tag !== 'string') {
        throw new Error(`${where}: an entry has no numeric \`when\` or string \`tag\``);
      }
      // The tag goes into a fetch path and an issue agents act on: only the shape drizzle writes.
      if (!/^\d{4}_[a-z0-9_]+$/.test(entry.tag)) {
        throw new Error(`${where}: migration tag ${JSON.stringify(entry.tag)} is not NNNN_name`);
      }
      return { version: entry.when, label: entry.tag, idx: entry.idx };
    })
    .sort((a, b) => a.version - b.version);
}

/** `export const NAME = 3` in TypeScript → 3. Throws when it is missing. */
export function parseTsConstant(source, constant, where) {
  const match = new RegExp(`\\bexport\\s+const\\s+${constant}\\s*(?::\\s*[\\w.]+\\s*)?=\\s*(\\d[\\d_]*)\\b`).exec(
    String(source),
  );
  if (!match) throw new Error(`${where}: no \`export const ${constant} = N\`; Orca moved or renamed it`);
  return rustInt(match[1], `${where}: ${constant}`);
}

/**
 * Look one database up upstream, at the default branch's current commit.
 * New migrations (newer than `checked`) are fetched so their tables are known.
 */
export async function observe(watch, checked, client) {
  const { repo } = watch.upstream;
  const newest = checked.versions.newest_checked;
  const sha = await client.headSha(repo);
  const blob = (path) => `https://github.com/${repo}/blob/${sha}/${path}`;
  const base = { watch, checked: checked.versions, reads: checked.reads, repo, sha };

  if (watch.upstream.kind === 't3') {
    const { dir } = watch.upstream;
    const all = t3Migrations(await client.list(repo, dir, sha));
    const migrations = [];
    for (const migration of all.filter((m) => m.version > newest)) {
      const changes = tableChanges(await client.raw(repo, migration.path, sha));
      migrations.push({ version: migration.version, label: migration.label, url: blob(migration.path), changes });
    }
    const head = all.at(-1).version;
    return {
      ...base,
      head,
      headLabel: `migration ${head}`,
      headPlain: String(head),
      headShort: String(head),
      checkedLabel: String(newest),
      checkedShort: String(newest),
      source: { path: dir, url: `https://github.com/${repo}/tree/${sha}/${dir}` },
      migrations,
    };
  }

  if (watch.upstream.kind === 'drizzle') {
    const { dir } = watch.upstream;
    const journalPath = `${dir}/meta/_journal.json`;
    let journal;
    try {
      journal = JSON.parse(await client.raw(repo, journalPath, sha));
    } catch (error) {
      if (error instanceof SyntaxError) throw new Error(`${repo}/${journalPath}: not JSON (${error.message})`);
      throw error;
    }
    const all = drizzleMigrations(journal, `${repo}/${journalPath}`);
    const migrations = [];
    for (const migration of all.filter((m) => m.version > newest)) {
      const path = `${dir}/${migration.label}.sql`;
      const changes = tableChanges(await client.raw(repo, path, sha));
      migrations.push({ version: migration.version, label: migration.label, idx: migration.idx, url: blob(path), changes });
    }
    const head = all.at(-1);
    const checkedEntry = all.find((m) => m.version === newest);
    const tagNumber = (label) => label.split('_')[0];
    return {
      ...base,
      head: head.version,
      headLabel: head.label,
      headPlain: head.label,
      headShort: tagNumber(head.label),
      checkedLabel: checkedEntry?.label ?? String(newest),
      checkedShort: checkedEntry ? tagNumber(checkedEntry.label) : String(newest),
      source: { path: dir, url: `https://github.com/${repo}/tree/${sha}/${dir}` },
      migrations,
    };
  }

  if (watch.upstream.kind === 'constant') {
    const { path, constant } = watch.upstream;
    const head = parseTsConstant(await client.raw(repo, path, sha), constant, `${repo}/${path}`);
    return {
      ...base,
      head,
      headLabel: `version ${head}`,
      headPlain: String(head),
      headShort: `v${head}`,
      checkedLabel: String(newest),
      checkedShort: `v${newest}`,
      source: { path, url: blob(path), constant },
      // Orca keeps no per-migration list, only the newest version.
      migrations: null,
    };
  }

  throw new Error(`${watch.id}: unknown upstream kind ${watch.upstream.kind}`);
}

// ---------------------------------------------------------------------------
// Drift.

const unique = (values) => [...new Set(values)];

/**
 * Which databases moved past what Bridge checked. An upstream that is older
 * than what Bridge checked (a revert, or Bridge checked a branch) is not
 * drift; it comes back as a note.
 *
 * @returns {{ drifts: object[], notes: string[] }}
 */
export function compare(observations) {
  const drifts = [];
  const notes = [];
  for (const observation of observations) {
    const { watch } = observation;
    const checked = observation.checked.newest_checked;
    if (observation.head === checked) continue;
    if (observation.head < checked) {
      notes.push(
        `${displayName(watch)}: upstream is at ${observation.headLabel}, older than the ${observation.checkedLabel} ` +
          'Bridge checked. Not drift.',
      );
      continue;
    }
    const reads = new Set(observation.reads.map((table) => table.toLowerCase()));
    const migrations =
      observation.migrations === null
        ? null
        : observation.migrations
            .filter((migration) => migration.version > checked)
            .map(({ changes, ...migration }) => ({
              ...migration,
              tables: changes.map((change) => ({ ...change, read: reads.has(change.table) })),
            }));
    const tables = migrations?.flatMap((migration) => migration.tables) ?? [];
    const readsTouched = unique(tables.filter((t) => t.read).map((t) => t.table)).sort();
    const newTables = unique(
      tables.filter((t) => !t.read && t.ops.includes('create') && !t.ops.includes('drop')).map((t) => t.table),
    ).sort();
    drifts.push({
      id: watch.id,
      watch,
      tool: watch.tool,
      database: watch.database,
      name: displayName(watch),
      repo: observation.repo,
      sha: observation.sha,
      checked,
      checkedLabel: observation.checkedLabel,
      checkedShort: observation.checkedShort,
      head: observation.head,
      headLabel: observation.headLabel,
      headPlain: observation.headPlain,
      headShort: observation.headShort,
      source: observation.source,
      migrations,
      readsTouched,
      newTables,
      // null when upstream has no per-migration list to read.
      touchesReads: migrations === null ? null : readsTouched.length > 0,
    });
  }
  return { drifts, notes };
}

// ---------------------------------------------------------------------------
// The issue.

const TITLE_MAX = 250;
const code = (text) => `\`${text}\``;
const shortSha = (sha) => String(sha).slice(0, 9);
/** 1792467855578 → `1_792_467_855_578`, as store.rs writes it. */
const rustNumber = (value) => (value >= 10_000 ? String(value).replace(/\B(?=(\d{3})+$)/g, '_') : String(value));

/** `chore(import): T3 Code moved to migration 120, Bridge checked 119`. */
export function issueTitle(drifts) {
  if (!drifts.length) throw new Error('issueTitle: no drift');
  let title;
  if (drifts.length === 1) {
    const [d] = drifts;
    title = `chore(import): ${d.name} moved to ${d.headLabel}, Bridge checked ${d.checkedLabel}`;
  } else {
    const [first, ...rest] = drifts;
    const parts = [
      `${first.name} moved to ${first.headLabel} (Bridge checked ${first.checkedLabel})`,
      ...rest.map((d) => `${d.name} to ${d.headLabel} (checked ${d.checkedLabel})`),
    ];
    title = `chore(import): ${parts.join(', ')}`;
  }
  return title.length > TITLE_MAX ? `${title.slice(0, TITLE_MAX - 1)}…` : title;
}

function diagram(drifts) {
  const nodes = [];
  const edges = [];
  drifts.forEach((d, col) => {
    const emphasis = d.touchesReads ? 'active' : undefined;
    nodes.push({ id: `up${col}`, row: 0, col, label: `${d.watch.short} ${d.headShort}`, ...(emphasis && { emphasis }) });
    nodes.push({ id: `ck${col}`, row: 1, col, label: `checked ${d.checkedShort}` });
    edges.push({ from: `up${col}`, to: `ck${col}`, ...(emphasis && { emphasis }) });
    edges.push({ from: `ck${col}`, to: 'fix' });
  });
  nodes.push({ id: 'fix', row: 2, col: 0, label: 'Re-check, bump' });
  const said = drifts.map((d) => `${d.name} is at ${d.headLabel} and Bridge checked ${d.checkedLabel}.`).join(' ');
  return {
    nodes,
    edges,
    caption: 'Upstream moved past the schema version Bridge was checked against.',
    ariaLabel: `${said} Each needs its new migrations read and newest_checked moved.`,
  };
}

function migrationRows(drift) {
  const rows = ['| Migration | Tables it touches |', '|---|---|'];
  for (const migration of drift.migrations) {
    const tables = migration.tables.length
      ? migration.tables
          .map((t) => `${t.read ? `**${code(t.table)}**` : code(t.table)} (${t.ops.join(', ')})`)
          .join(', ')
      : '_no SQL found in the file; read it_';
    rows.push(`| [${code(migration.label)}](${migration.url}) | ${tables} |`);
  }
  return rows;
}

function filesToUpdate(drift) {
  const { watch } = drift;
  const commit = `${drift.repo}@${shortSha(drift.sha)}`;
  const constant = `${watch.versions}.newest_checked`;
  if (watch.upstream.kind === 't3') {
    return [
      `- ${code(watch.store)}: ${code(constant)} to ${code(drift.head)}, and the commit in its doc comment to ${code(commit)}.`,
      `- ${code(`testing/fixtures/import/t3code/schema-${drift.head}.sql`)} and ${code(`migrations-${drift.head}.txt`)}: ` +
        `add ${code(drift.head)} to ${code('HEADS')} in ${code('testing/fixtures/import/t3code/schemaDump.test.ts')} ` +
        'and rebuild them from a T3 Code checkout at that commit (the file says how).',
      `- ${code('DENIED_TABLES')} in the same ${code('store.rs')}: add any new table that holds credentials, tokens, ` +
        'webhooks, links to other machines or raw event logs.',
      `- ${code(watch.tables)}: only if Bridge must read a new table.`,
    ];
  }
  if (watch.upstream.kind === 'drizzle') {
    const head = drift.migrations?.at(-1);
    const idx = head?.idx ?? '<idx>';
    return [
      `- ${code(watch.store)}: ${code(constant)} to ${code(rustNumber(drift.head))} (the journal ${code('when')} of ` +
        `${code(drift.headLabel)}), and the migration and commit (${code(commit)}) in its doc comment.`,
      `- ${code(`testing/fixtures/import/superset/${watch.fixture}-${idx}.sql`)} and ${code(`${watch.fixture}-${idx}.txt`)}: ` +
        `add ${code(idx)} to the ${code(watch.fixture)} heads in ${code('testing/fixtures/import/superset/rebuild.py')} ` +
        'and run it against a Superset checkout at that commit.',
      `- ${code(watch.tables)}: only if Bridge must read a new table.`,
    ];
  }
  return [
    `- ${code(watch.store)}: ${code(constant)} to ${code(drift.head)}, and the Orca release and commit ` +
      `(${code(commit)}) in its doc comment.`,
    `- A schema fixture for the Orca release that ships it, beside ${code('testing/fixtures/import/orca/1.4.271/')}.`,
    `- ${code(watch.tables)}: only if Bridge must read a new table.`,
  ];
}

function driftSection(drift) {
  const lines = [`### ${drift.name}: ${drift.checkedLabel} → ${drift.headPlain}`, ''];
  const commit = `${drift.repo}@${shortSha(drift.sha)}`;
  if (drift.migrations === null) {
    lines.push(
      `Upstream: ${code(drift.source.constant)} is ${code(drift.head)} in [${code(drift.source.path)}](${drift.source.url}) ` +
        `at ${code(commit)}. Orca keeps no per-migration list, so read the migration code there for what changed ` +
        `between ${drift.checkedLabel} and ${drift.head}, and check it against ${code(drift.watch.tables)}.`,
    );
  } else {
    lines.push(`Upstream: [${code(drift.source.path)}](${drift.source.url}) at ${code(commit)}.`, '');
    lines.push(...migrationRows(drift));
    if (drift.readsTouched.length) {
      lines.push(
        '',
        `Tables Bridge reads (${code(drift.watch.tables)}) that these migrations change: ` +
          `${drift.readsTouched.map(code).join(', ')}. Check every column Bridge reads from them first.`,
      );
    }
    if (drift.newTables.length) {
      lines.push('', `New tables: ${drift.newTables.map(code).join(', ')}. Bridge never reads them unless they join ${code(drift.watch.tables)}.`);
    }
  }
  lines.push('', 'Files to update:', '', ...filesToUpdate(drift));
  return lines;
}

function tldr(drifts) {
  const moved =
    drifts.length === 1
      ? `${drifts[0].name} moved to ${drifts[0].headLabel}; Bridge was checked up to ${drifts[0].checkedLabel}.`
      : `${drifts.map((d) => `${d.name} moved to ${d.headLabel} (checked ${d.checkedLabel})`).join('; ')}.`;
  const touching = drifts.filter((d) => d.touchesReads);
  const unknown = drifts.filter((d) => d.touchesReads === null);
  let impact;
  if (touching.length) {
    const named = touching.map((d) => `${d.name}: ${d.readsTouched.map(code).join(', ')}`).join('; ');
    impact = `A new migration changes a table Bridge reads (${named}), so check that first: an import fails if a column Bridge reads is gone.`;
  } else {
    impact = 'Bridge still reads a newer database while every column it reads is there, so this is a re-check, not an outage.';
  }
  if (unknown.length) {
    impact += ` ${unknown.map((d) => d.name).join(' and ')} keep${unknown.length === 1 ? 's' : ''} no per-migration list, so read ${unknown.length === 1 ? 'its' : 'their'} change by hand.`;
  }
  return `**TL;DR:** ${moved} ${impact}`;
}

function summaryRows(drifts) {
  const rows = ['| Database | Bridge checked | Upstream now | New migrations | Changes a table Bridge reads |', '|---|---|---|---|---|'];
  for (const d of drifts) {
    const count = d.migrations === null ? 'not listed' : String(d.migrations.length);
    const reads =
      d.touchesReads === null ? 'unknown' : d.touchesReads ? `**yes**: ${d.readsTouched.map(code).join(', ')}` : 'no';
    rows.push(`| ${d.name} | ${code(d.checkedLabel)} | ${code(d.headPlain)} | ${count} | ${reads} |`);
  }
  return rows;
}

/** The issue body: the repository's two-audience format (docs/issue-format.md). */
export function issueBody(drifts) {
  if (!drifts.length) throw new Error('issueBody: no drift');
  const allowlists = unique(drifts.map((d) => code(d.watch.tables)));
  return [
    MARKER,
    '## For humans',
    '',
    tldr(drifts),
    '',
    '```diagram',
    JSON.stringify(diagram(drifts)),
    '```',
    '',
    ...summaryRows(drifts),
    '',
    'Conductor is not watched: its source is not public, so its schema is checked by hand.',
    '',
    '## For agents',
    '',
    'Bridge gates each import on `Versions { oldest, newest_checked }` in the tool\'s `store.rs` ' +
      '(`src-tauri/bridge-core/src/import_sqlite.rs`). A database newer than `newest_checked` is still read when ' +
      'every table and column Bridge reads is present, so nothing breaks unless a migration below changes a table ' +
      'Bridge reads (**bold**). The work: read each new migration, decide whether Bridge\'s reads change, add a schema ' +
      'fixture at the new head, and move `newest_checked`.',
    '',
    ...drifts.flatMap((drift) => [...driftSection(drift), '']),
    '### Acceptance criteria',
    '',
    '- [ ] Every new migration above is read, and the PR says for each bold table whether Bridge\'s reads change.',
    '- [ ] `newest_checked` equals the upstream head for every database above, with a schema fixture at that version.',
    `- [ ] No new table that holds secrets is readable: T3 Code's go in \`DENIED_TABLES\`, and no allowlist (${allowlists.join(', ')}) gains one.`,
    '- [ ] `bun run test` passes.',
    `- [ ] The next run of \`${WORKFLOW}\` reports no drift for these databases.`,
    '',
    `Written by \`${SCRIPT}\` (weekly, \`${WORKFLOW}\`) and rewritten while the drift remains, so edits here are ` +
      'overwritten. Close it from the PR that moves the constants.',
    '',
  ].join('\n');
}

// ---------------------------------------------------------------------------
// GitHub.

/**
 * A small GitHub REST client. `fetch` is injectable so tests run offline.
 * Any non-2xx answer (other than the ones a call says it tolerates) throws
 * with the method, path and status.
 */
export function github({ token, fetch: fetchImpl = globalThis.fetch, api = 'https://api.github.com' } = {}) {
  async function request(method, path, { body, accept = 'application/vnd.github+json', tolerate = [] } = {}) {
    const headers = {
      Accept: accept,
      'User-Agent': 'bridge-import-schema-drift',
      'X-GitHub-Api-Version': '2022-11-28',
    };
    if (token) headers.Authorization = `Bearer ${token}`;
    if (body !== undefined) headers['Content-Type'] = 'application/json';
    let response;
    try {
      response = await fetchImpl(`${api}${path}`, {
        method,
        headers,
        body: body === undefined ? undefined : JSON.stringify(body),
      });
    } catch (error) {
      throw new Error(`GitHub ${method} ${path} failed: ${error.message}`);
    }
    const text = await response.text();
    if (!response.ok && !tolerate.includes(response.status)) {
      let message = text.slice(0, 200);
      try {
        message = JSON.parse(text).message ?? message;
      } catch {
        // Not JSON: keep the raw text.
      }
      throw new Error(`GitHub ${method} ${path} failed: ${response.status} ${message}`.trim());
    }
    return { status: response.status, text };
  }
  const json = async (method, path, options) => {
    const { text } = await request(method, path, options);
    return text ? JSON.parse(text) : null;
  };
  const contents = (repo, path, ref) =>
    `/repos/${repo}/contents/${path.split('/').map(encodeURIComponent).join('/')}${ref ? `?ref=${encodeURIComponent(ref)}` : ''}`;
  return {
    authenticated: Boolean(token),
    request,
    async headSha(repo) {
      const { text } = await request('GET', `/repos/${repo}/commits/HEAD`, { accept: 'application/vnd.github.sha' });
      const sha = text.trim();
      if (!/^[0-9a-f]{40}$/.test(sha)) throw new Error(`GitHub GET /repos/${repo}/commits/HEAD: not a commit sha`);
      return sha;
    },
    async raw(repo, path, ref) {
      return (await request('GET', contents(repo, path, ref), { accept: 'application/vnd.github.raw' })).text;
    },
    async list(repo, path, ref) {
      const listing = await json('GET', contents(repo, path, ref));
      if (!Array.isArray(listing)) throw new Error(`GitHub ${repo}/${path} is not a directory`);
      return listing;
    },
    openIssues(repo, label) {
      return json('GET', `/repos/${repo}/issues?state=open&labels=${encodeURIComponent(label)}&per_page=100`);
    },
    createLabel(repo, label) {
      // 422: it already exists.
      return request('POST', `/repos/${repo}/labels`, { body: label, tolerate: [422] });
    },
    createIssue(repo, issue) {
      return json('POST', `/repos/${repo}/issues`, { body: issue });
    },
    updateIssue(repo, number, changes) {
      return json('PATCH', `/repos/${repo}/issues/${number}`, { body: changes });
    },
  };
}

/** The open issue this script owns, if any: labeled and carrying the marker. */
export async function findIssue(client, repo) {
  const issues = await client.openIssues(repo, LABEL);
  return (
    (issues ?? [])
      .filter((issue) => !issue.pull_request && String(issue.body ?? '').includes(MARKER))
      .sort((a, b) => a.number - b.number)[0] ?? null
  );
}

/** Update the open drift issue, or open one. Returns what it did. */
export async function syncIssue(client, repo, { title, body }) {
  const existing = await findIssue(client, repo);
  if (existing) {
    if (existing.title === title && existing.body === body) return { action: 'unchanged', number: existing.number };
    await client.updateIssue(repo, existing.number, { title, body });
    return { action: 'updated', number: existing.number };
  }
  await client.createLabel(repo, {
    name: LABEL,
    color: 'ededed',
    description: 'An imported tool moved past the schema version Bridge checked',
  });
  const created = await client.createIssue(repo, { title, body, labels: [LABEL] });
  return { action: 'created', number: created?.number };
}

// ---------------------------------------------------------------------------
// CLI.

export function parseArgs(argv, env = {}) {
  const args = { dryRun: /^(?:true|1)$/i.test(env.DRY_RUN ?? ''), root: DEFAULT_ROOT };
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];
    if (arg === '--dry-run') args.dryRun = true;
    else if (arg === '--root') {
      const dir = argv[(index += 1)];
      if (!dir) throw new Error('--root needs a directory');
      args.root = resolve(dir);
    } else throw new Error(`unknown argument ${arg}`);
  }
  return args;
}

/** Check every watched database and sync the issue. */
export async function run({ root = DEFAULT_ROOT, client, repo, dryRun = false, log = console.log, watched = WATCHED }) {
  if (!dryRun && !repo) throw new Error('GITHUB_REPOSITORY is not set; pass --dry-run to only print the issue');
  if (!dryRun && !client.authenticated) throw new Error('GITHUB_TOKEN is not set; it is needed to open or update the issue');
  const checked = readChecked(root, watched);
  const observations = [];
  for (const watch of watched) observations.push(await observe(watch, checked[watch.id], client));
  for (const o of observations) {
    log(`${displayName(o.watch)}: upstream ${o.headLabel} (${o.repo}@${shortSha(o.sha)}), Bridge checked ${o.checkedLabel}`);
  }
  const { drifts, notes } = compare(observations);
  for (const note of notes) log(`note: ${note}`);
  if (!drifts.length) {
    log('no drift');
    return { action: 'none', drifts };
  }
  const title = issueTitle(drifts);
  const body = issueBody(drifts);
  if (dryRun) {
    const existing = repo ? await findIssue(client, repo) : null;
    log(`dry run: would ${existing ? `update #${existing.number}` : 'open a new issue'}, and writes nothing`);
    log('');
    log(title);
    log('');
    log(body);
    return { action: 'dry-run', drifts, title, body, existing };
  }
  const result = await syncIssue(client, repo, { title, body });
  log(`${result.action} #${result.number}: ${title}`);
  return { ...result, drifts, title, body };
}

export async function main(argv = process.argv.slice(2), env = process.env) {
  const args = parseArgs(argv, env);
  const client = github({ token: env.GITHUB_TOKEN || undefined });
  return run({ root: args.root, client, repo: env.GITHUB_REPOSITORY || undefined, dryRun: args.dryRun });
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main().catch((error) => {
    console.error(`import-schema-drift: ${error.message}`);
    process.exitCode = 1;
  });
}

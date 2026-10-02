import { closeSync, existsSync, fsyncSync, mkdirSync, openSync, readFileSync,
  renameSync, rmSync, writeFileSync } from 'node:fs';
import { dirname } from 'node:path';
import initSqlJs from 'sql.js';

const RETENTION_MS = 24 * 60 * 60 * 1000;

// One process owns this disk. SQLite is exported atomically after each mutation;
// sql.js avoids native addons and works with both Node and bun workspace installs.
export async function openStore(path) {
  mkdirSync(dirname(path), { recursive: true, mode: 0o700 });
  const lockPath = `${path}.lock`;
  if (existsSync(lockPath)) {
    const pid = Number(readFileSync(lockPath, 'utf8'));
    if (!Number.isInteger(pid) || pid < 1) throw new Error('Invalid ticketer lock; inspect it before removal');
    let alive = true;
    try { process.kill(pid, 0); } catch (error) {
      if (error.code === 'ESRCH') alive = false;
      else throw error;
    }
    if (alive) throw new Error('Another ticketer process owns this database');
    rmSync(lockPath);
  }
  const lock = openSync(lockPath, 'wx', 0o600);
  writeFileSync(lock, String(process.pid));
  closeSync(lock);
  let db;
  try {
    const SQL = await initSqlJs();
    db = new SQL.Database(existsSync(path) ? readFileSync(path) : undefined);
    db.run(`CREATE TABLE IF NOT EXISTS tickets (
      id TEXT PRIMARY KEY, state TEXT NOT NULL, issue TEXT
    ); CREATE TABLE IF NOT EXISTS messages (
      id TEXT PRIMARY KEY, timestamp INTEGER NOT NULL, source TEXT NOT NULL
    );`);
  } catch (error) { rmSync(lockPath, { force: true }); throw error; }

  function persist() {
    const temporary = `${path}.tmp`;
    const fd = openSync(temporary, 'w', 0o600);
    try { writeFileSync(fd, db.export()); fsyncSync(fd); } finally { closeSync(fd); }
    renameSync(temporary, path);
    const parent = openSync(dirname(path), 'r');
    try { fsyncSync(parent); } finally { closeSync(parent); }
  }
  function rows(sql, params = []) {
    const stmt = db.prepare(sql);
    try {
      stmt.bind(params);
      const out = [];
      while (stmt.step()) out.push(stmt.getAsObject());
      return out;
    } finally { stmt.free(); }
  }
  persist();
  return {
    get(id) {
      const row = rows('SELECT state, issue FROM tickets WHERE id = ?', [id])[0];
      return row ? { state: row.state, issue: row.issue ? JSON.parse(row.issue) : null } : null;
    },
    set(id, state, issue = null) {
      db.run('INSERT OR REPLACE INTO tickets VALUES (?, ?, ?)', [id, state, issue ? JSON.stringify(issue) : null]);
      persist();
    },
    remember(source, now = Date.now()) {
      db.run('INSERT OR IGNORE INTO messages VALUES (?, ?, ?)', [source.id, source.timestamp, JSON.stringify(source)]);
      db.run('DELETE FROM messages WHERE timestamp < ?', [now - RETENTION_MS]);
      db.run('DELETE FROM messages WHERE id NOT IN (SELECT id FROM messages ORDER BY timestamp DESC LIMIT 1000)');
      persist();
    },
    source(id, now = Date.now()) {
      const row = rows('SELECT source FROM messages WHERE id = ? AND timestamp >= ?', [id, now - RETENTION_MS])[0];
      return row ? JSON.parse(row.source) : null;
    },
    prior(source) {
      return rows('SELECT source FROM messages WHERE timestamp >= ? AND timestamp <= ? AND id != ? ORDER BY timestamp DESC LIMIT 10',
        [source.timestamp - 15 * 60 * 1000, source.timestamp, source.id])
        .reverse().map((row) => JSON.parse(row.source));
    },
    close() { db.close(); rmSync(lockPath, { force: true }); },
  };
}

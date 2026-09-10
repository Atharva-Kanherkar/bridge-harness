// Adapted from Orca's headless-emulator and terminal-history-session-writer.
// Copyright (c) 2026 Lovecast Inc. MIT; see THIRD_PARTY_NOTICES.md.
import headless from '@xterm/headless';
import serialize from '@xterm/addon-serialize';
import unicode from '@xterm/addon-unicode11';
import { StringDecoder } from 'node:string_decoder';
import { createHash } from 'node:crypto';
import { mkdir, readFile, writeFile, rename, appendFile, readdir, stat } from 'node:fs/promises';
import { join } from 'node:path';
import { hostname } from 'node:os';
import { advancePartialEscapeTail } from './orca/terminal-partial-escape-tail.mjs';
import { TerminalMouseModeMirror } from './orca/terminal-mouse-mode-mirror.mjs';
import { serializeWithAbsoluteCursor, readSavedCursorRegister } from './orca/terminal-serialize-absolute-cursor.mjs';

export const LOG_LIMIT = 512 * 1024;
export const SNAPSHOT_LIMIT = 4 * 1024 * 1024;
export const HISTORY_BUDGET = 128 * 1024 * 1024;
const hash = key => createHash('sha256').update(key).digest('hex');
const readJson = async path => JSON.parse(await readFile(path, 'utf8'));
async function atomic(path, value) {
  const temp = `${path}.tmp`;
  await writeFile(temp, JSON.stringify(value), { mode: 0o600 });
  await rename(temp, path);
}

export class TerminalStateStore {
  constructor(root) { this.root = root; this.states = new Map(); }
  directory(key) { return join(this.root, hash(key)); }
  async ensure(key) {
    if (this.states.has(key)) return this.states.get(key);
    const dir = this.directory(key);
    const saved = await readJson(join(dir, 'checkpoint.json'));
    const state = this.makeState(saved.record);
    await this.parse(state, saved.ansi ?? '');
    state.sequence = saved.sequence ?? 0;
    if (saved.pendingUtf8) state.decoder.write(Buffer.from(saved.pendingUtf8, 'base64'));
    state.partial = saved.partial ?? '';
    if (state.partial) await this.parse(state, state.partial);
    // A checkpoint is installed before the journal is reset. Sequence numbers
    // make a crash between those two writes safe to replay.
    const log = await readFile(join(dir, 'output.log'), 'utf8').catch(() => '');
    for (const line of log.split('\n')) {
      if (!line) continue;
      let event;
      try { event = JSON.parse(line); } catch { break; } // torn final append
      if (event.sequence <= state.sequence) continue;
      if (event.kind === 'resize') {
        state.terminal.resize(event.cols, event.rows);
        Object.assign(state.record, { cols: event.cols, rows: event.rows });
      } else {
        const data = event.bytes === undefined ? event.data : state.decoder.write(Buffer.from(event.bytes, 'base64'));
        await this.parse(state, data);
        state.partial = advancePartialEscapeTail(state.partial, data);
      }
      state.sequence = event.sequence;
    }
    state.logBytes = Buffer.byteLength(log);
    this.states.set(key, state);
    return state;
  }
  makeState(record) {
    const terminal = new headless.Terminal({ cols: record.cols, rows: record.rows, scrollback: 5000, allowProposedApi: true, logLevel: 'off', vtExtensions: { kittyKeyboard: true } });
    const serializer = new serialize.SerializeAddon();
    terminal.loadAddon(serializer);
    terminal.loadAddon(new unicode.Unicode11Addon());
    terminal.unicode.activeVersion = '11';
    const state = { terminal, serializer, mouseModes: new TerminalMouseModeMirror(), record: { ...record }, sequence: 0, partial: '', decoder: new StringDecoder('utf8'), logBytes: 0 };
    // Host observes these values; terminal-controlled text never becomes a path
    // to execute without the Rust launch boundary validating it.
    terminal.parser.registerOscHandler(7, data => {
      try { const url = new URL(data); if (url.protocol === 'file:' && (!url.hostname || ['localhost', hostname(), process.env.HOSTNAME].some(host => host?.toLowerCase() === url.hostname.toLowerCase()))) state.record.cwd = decodeURIComponent(url.pathname); } catch {}
      return false;
    });
    return state;
  }
  parse(state, data) {
    state.mouseModes.scan(data);
    return new Promise(resolve => state.terminal.write(data, resolve));
  }
  serialize(state) {
    let scrollback = 5000;
    let ansi;
    do {
      ansi = serializeWithAbsoluteCursor(state.serializer, state.terminal, { scrollback }, readSavedCursorRegister(state.terminal));
      scrollback = Math.floor(scrollback / 2);
    } while (Buffer.byteLength(ansi) > SNAPSHOT_LIMIT && scrollback > 0);
    if (Buffer.byteLength(ansi) > SNAPSHOT_LIMIT) throw new Error('Terminal snapshot exceeds its storage budget');
    // SerializeAddon omits mouse encoding and Kitty keyboard negotiation.
    // Follow Orca's mode mirror and private-core flags reader so a restored
    // interactive CLI gets the same input protocol as its live terminal.
    if (state.mouseModes.sgrMouseMode) ansi += '\x1b[?1006h';
    if (state.mouseModes.sgrMousePixelsMode) ansi += '\x1b[?1016h';
    const flags = state.terminal._core?.coreService?.kittyKeyboard?.flags;
    if (typeof flags === 'number' && flags > 0) ansi += `\x1b[=${flags}u`;
    return ansi;
  }
  async checkpoint(key, state) {
    const dir = this.directory(key);
    await atomic(join(dir, 'checkpoint.json'), { record: state.record, sequence: state.sequence, ansi: this.serialize(state), partial: state.partial, pendingUtf8: state.decoder.lastNeed ? state.decoder.lastChar.subarray(0, state.decoder.lastTotal - state.decoder.lastNeed).toString('base64') : '' });
    await writeFile(join(dir, 'output.log'), '', { mode: 0o600 });
    state.logBytes = 0;
  }
  async create(key, record) {
    const old = this.states.get(key);
    old?.terminal.dispose();
    const state = this.makeState(record);
    await mkdir(this.directory(key), { recursive: true, mode: 0o700 });
    this.states.set(key, state);
    await this.checkpoint(key, state);
    return record;
  }
  async increment(key, event) {
    const state = await this.ensure(key);
    if (event.generation !== state.record.generation) return null;
    const sequence = ++state.sequence;
    let data = '';
    if (event.kind === 'resize') {
      state.terminal.resize(event.cols, event.rows);
      Object.assign(state.record, { cols: event.cols, rows: event.rows });
    } else {
      data = state.decoder.write(Buffer.from(event.bytes, 'base64'));
      await this.parse(state, data);
      state.partial = advancePartialEscapeTail(state.partial, data);
    }
    const record = event.kind === 'resize' ? { kind: 'resize', cols: event.cols, rows: event.rows, sequence } : { kind: 'data', bytes: event.bytes, sequence };
    const line = JSON.stringify(record) + '\n';
    await appendFile(join(this.directory(key), 'output.log'), line, { mode: 0o600 });
    state.logBytes += Buffer.byteLength(line);
    if (state.logBytes >= LOG_LIMIT) await this.checkpoint(key, state);
    return { generation: state.record.generation, sequence, data };
  }
  async describe(key) {
    const state = await this.ensure(key);
    const result = { record: state.record, sequence: state.sequence };
    this.releaseEnded(key, state);
    return result;
  }
  async snapshot(key) {
    const state = await this.ensure(key);
    const result = { record: state.record, sequence: state.sequence, ansi: this.serialize(state) + state.partial };
    this.releaseEnded(key, state);
    return result;
  }
  releaseEnded(key, state) {
    if (state.record.status !== 'running') {
      this.states.delete(key);
      state.terminal.dispose();
    }
  }
  async update(key, changes) {
    const state = await this.ensure(key);
    if (changes.generation && changes.generation !== state.record.generation) {
      this.releaseEnded(key, state);
      return state.record;
    }
    const closed = state.record.status === 'closed';
    Object.assign(state.record, changes);
    if (changes.status) state.sequence++;
    if (closed) state.record.status = 'closed';
    await this.checkpoint(key, state);
    if (state.record.status !== 'running') {
      this.releaseEnded(key, state);
      await this.prune();
    }
    return state.record;
  }
  async inventory(workspaceId) {
    await mkdir(this.root, { recursive: true, mode: 0o700 });
    const records = [];
    for (const entry of await readdir(this.root, { withFileTypes: true })) {
      if (!entry.isDirectory() || !/^[0-9a-f]{64}$/.test(entry.name)) continue;
      try {
        const saved = await readJson(join(this.root, entry.name, 'checkpoint.json'));
        const live = [...this.states.values()].find(s => s.record.terminalId === saved.record.terminalId && s.record.workspaceId === saved.record.workspaceId);
        if (live) saved.record = live.record;
        if (saved.record.workspaceId === workspaceId && saved.record.status !== 'closed') records.push(saved.record);
      } catch {} // a damaged terminal does not hide the remaining workspace
    }
    return records;
  }
  async prune() {
    const files = [];
    for (const entry of await readdir(this.root, { withFileTypes: true })) {
      if (!entry.isDirectory() || !/^[0-9a-f]{64}$/.test(entry.name)) continue;
      const path = join(this.root, entry.name);
      try {
        const saved = await readJson(join(path, 'checkpoint.json'));
        const info = await stat(join(path, 'checkpoint.json'));
        const log = await stat(join(path, 'output.log')).catch(() => ({ size: 0 }));
        files.push({ path, saved, bytes: info.size + log.size, age: info.mtimeMs });
      } catch {}
    }
    let total = files.reduce((n, f) => n + f.bytes, 0);
    for (const f of files.sort((a, b) => a.age - b.age)) {
      if (total <= HISTORY_BUDGET) break;
      if (f.saved.record.status === 'running') continue;
      await atomic(join(f.path, 'checkpoint.json'), { ...f.saved, ansi: '', partial: '', record: { ...f.saved.record, historyTruncated: true } });
      await writeFile(join(f.path, 'output.log'), '');
      total -= f.bytes;
    }
  }
  async layout(workspaceId, value) {
    await mkdir(this.root, { recursive: true, mode: 0o700 });
    const path = join(this.root, `layout-${hash(workspaceId)}.json`);
    if (value !== undefined) {
      if (Buffer.byteLength(JSON.stringify(value)) > 256 * 1024) throw new Error('Terminal layout is too large');
      await atomic(path, value);
      return value;
    }
    return readJson(path).catch(() => null);
  }
  dispose() { for (const s of this.states.values()) s.terminal.dispose(); this.states.clear(); }
}

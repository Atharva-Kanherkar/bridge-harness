import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, rm, appendFile, readFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import headless from '@xterm/headless';
import { TerminalStateStore } from './state.mjs';

const record = { workspaceId: 'w', terminalId: 't', generation: 'first', title: 'Shell', cwd: '/tmp', agentId: null, rows: 12, cols: 40, status: 'running', createdAt: '2026-09-10T00:00:00Z' };
async function fixture(fn) {
  const root = await mkdtemp(join(tmpdir(), 'bridge-terminal-state-'));
  const store = new TerminalStateStore(root);
  try { await store.create('w:t', record); await fn(store, root); }
  finally { store.dispose(); await rm(root, { recursive: true, force: true }); }
}
const write = (store, text) => store.increment('w:t', { generation: 'first', bytes: Buffer.from(text).toString('base64') });
const visible = terminal => Array.from({ length: terminal.rows }, (_, row) => terminal.buffer.active.getLine(terminal.buffer.active.baseY + row)?.translateToString(true) ?? '');
async function replay(snapshot, continuation = '') {
  const terminal = new headless.Terminal({ rows: snapshot.record.rows, cols: snapshot.record.cols, allowProposedApi: true, vtExtensions: { kittyKeyboard: true } });
  await new Promise(resolve => terminal.write(snapshot.ansi + continuation, resolve));
  return terminal;
}

test('journal restores output produced without a UI and ignores a torn final append', () => fixture(async (store, root) => {
  await write(store, 'first\r\nsecond');
  await appendFile(join(store.directory('w:t'), 'output.log'), '{"unfinished":');
  const reopened = new TerminalStateStore(root);
  try {
    const snapshot = await reopened.snapshot('w:t');
    assert.equal(snapshot.sequence, 1);
    const terminal = await replay(snapshot);
    assert.deepEqual(visible(terminal).slice(0, 2), ['first', 'second']);
    terminal.dispose();
  } finally { reopened.dispose(); }
}));

test('UTF-8 split across PTY reads is emitted intact', () => fixture(async store => {
  const bytes = Buffer.from('हैलो 🌍');
  let output = '';
  for (const byte of bytes) output += (await store.increment('w:t', { generation: 'first', bytes: Buffer.from([byte]).toString('base64') })).data;
  assert.equal(output, 'हैलो 🌍');
}));

test('Orca partial-escape helper restores a sequence split at snapshot time', () => fixture(async store => {
  await write(store, 'hello\x1b[3');
  const snapshot = await store.snapshot('w:t');
  assert.ok(snapshot.ansi.endsWith('\x1b[3'));
  const terminal = await replay(snapshot, '1m red');
  assert.equal(visible(terminal)[0], 'hello red');
  terminal.dispose();
}));

test('alternate screen, cursor and continued output match uninterrupted rendering', () => fixture(async store => {
  await write(store, 'normal\x1b[?1049h\x1b[2J\x1b[3;4Hagent\x1b[5;2H');
  const snapshot = await store.snapshot('w:t');
  const restored = await replay(snapshot, 'next');
  await write(store, 'next');
  const live = (await store.ensure('w:t')).terminal;
  assert.equal(restored.buffer.active.type, 'alternate');
  assert.deepEqual(visible(restored), visible(live));
  assert.equal(restored.buffer.active.cursorX, live.buffer.active.cursorX);
  restored.dispose();
}));

test('resize records retain dimensions and ordering across a restart', () => fixture(async (store, root) => {
  await write(store, 'before');
  await store.increment('w:t', { kind: 'resize', generation: 'first', rows: 16, cols: 60 });
  await write(store, '\r\nafter');
  const reopened = new TerminalStateStore(root);
  try {
    const snapshot = await reopened.snapshot('w:t');
    assert.equal(snapshot.sequence, 3);
    assert.equal(snapshot.record.cols, 60);
    assert.equal(snapshot.record.rows, 16);
  } finally { reopened.dispose(); }
}));

test('stale generations cannot write to or settle a replacement process', () => fixture(async store => {
  await store.create('w:t', { ...record, generation: 'second' });
  assert.equal(await write(store, 'stale'), null);
  await store.update('w:t', { generation: 'first', status: 'exited' });
  assert.equal((await store.snapshot('w:t')).record.status, 'running');
}));

test('close remains closed when its process reports exit later', () => fixture(async store => {
  await store.update('w:t', { status: 'closed' });
  await store.update('w:t', { generation: 'first', status: 'exited' });
  assert.deepEqual(await store.inventory('w'), []);
}));

test('reading ended history does not retain a hidden terminal emulator', () => fixture(async store => {
  await write(store, 'saved output');
  await store.update('w:t', { status: 'exited' });
  assert.equal(store.states.size, 0);
  assert.ok((await store.snapshot('w:t')).ansi.includes('saved output'));
  assert.equal(store.states.size, 0);
  assert.equal((await store.describe('w:t')).record.status, 'exited');
  assert.equal(store.states.size, 0);
}));

test('restored CLIs retain mouse encoding and negotiated keyboard modes', () => fixture(async (store, root) => {
  await write(store, '\x1b[?1002;100');
  await write(store, '6h\x1b[>3u');
  await store.checkpoint('w:t', await store.ensure('w:t'));
  const reopened = new TerminalStateStore(root);
  try {
    const terminal = await replay(await reopened.snapshot('w:t'));
    assert.equal(terminal.modes.mouseTrackingMode, 'drag');
    assert.equal(terminal._core.mouseStateService.activeEncoding, 'SGR');
    assert.equal(terminal._core.coreService.kittyKeyboard.flags, 3);
    terminal.dispose();
    await write(reopened, '\x1bc');
    const reset = await replay(await reopened.snapshot('w:t'));
    assert.equal(reset.modes.mouseTrackingMode, 'none');
    assert.equal(reset._core.coreService.kittyKeyboard.flags, 0);
    reset.dispose();
  } finally { reopened.dispose(); }
}));

test('checkpoint replay does not duplicate an older retained journal', () => fixture(async (store, root) => {
  await write(store, 'one');
  const log = await readFile(join(store.directory('w:t'), 'output.log'));
  await store.checkpoint('w:t', await store.ensure('w:t'));
  await appendFile(join(store.directory('w:t'), 'output.log'), log);
  const reopened = new TerminalStateStore(root);
  try {
    const terminal = await replay(await reopened.snapshot('w:t'));
    assert.equal(visible(terminal)[0], 'one');
    terminal.dispose();
  } finally { reopened.dispose(); }
}));

test('workspace layouts persist independently with bounded payloads', () => fixture(async (store, root) => {
  const layout = { version: 1, activeLeafId: 't', tabs: [] };
  await store.layout('w', layout);
  const reopened = new TerminalStateStore(root);
  try {
    assert.deepEqual(await reopened.layout('w'), layout);
    assert.equal(await reopened.layout('other'), null);
    await assert.rejects(store.layout('w', 'x'.repeat(300_000)), /too large/);
  } finally { reopened.dispose(); }
}));


test('UTF-8 pending at a checkpoint survives a history runtime restart', () => fixture(async (store, root) => {
  const bytes = Buffer.from('🌍');
  await store.increment('w:t', { generation: 'first', bytes: bytes.subarray(0, 2).toString('base64') });
  await store.checkpoint('w:t', await store.ensure('w:t'));
  const reopened = new TerminalStateStore(root);
  try {
    const frame = await reopened.increment('w:t', { generation: 'first', bytes: bytes.subarray(2).toString('base64') });
    assert.equal(frame.data, '🌍');
    const terminal = await replay(await reopened.snapshot('w:t'));
    assert.equal(visible(terminal)[0], '🌍');
    terminal.dispose();
  } finally { reopened.dispose(); }
}));

import assert from 'node:assert/strict';
import { test } from 'node:test';

import {
  collectCounts,
  compareCounts,
  platformKey,
  ratchetedLints,
  renderedErrors,
  totals,
} from '../clippy-ratchet.mjs';

const TRACKED = ['clippy::unwrap_used', 'clippy::panic'];

function diagnostic(lint, file, line, { level = 'warning', column = 1 } = {}) {
  return JSON.stringify({
    reason: 'compiler-message',
    message: {
      level,
      code: lint ? { code: lint } : null,
      spans: [
        { is_primary: false, file_name: 'other.rs', line_start: 1, column_start: 1, line_end: 1, column_end: 2 },
        { is_primary: true, file_name: file, line_start: line, column_start: column, line_end: line, column_end: column + 4 },
      ],
    },
  });
}

test('counts warnings per lint and file from the primary span', () => {
  const output = [
    diagnostic('clippy::unwrap_used', 'bridge-core/src/a.rs', 3),
    diagnostic('clippy::unwrap_used', 'bridge-core/src/a.rs', 9),
    diagnostic('clippy::panic', 'bridge-core/src/b.rs', 1),
  ].join('\n');

  assert.deepEqual(collectCounts(output, TRACKED), {
    'clippy::panic': { 'bridge-core/src/b.rs': 1 },
    'clippy::unwrap_used': { 'bridge-core/src/a.rs': 2 },
  });
});

test('counts a diagnostic once when several targets report it', () => {
  const repeated = diagnostic('clippy::unwrap_used', 'bridge-core/src/a.rs', 3);
  const counts = collectCounts([repeated, repeated, repeated].join('\n'), TRACKED);

  assert.equal(counts['clippy::unwrap_used']['bridge-core/src/a.rs'], 1);
});

test('ignores errors, uncoded messages, other records and non-JSON lines', () => {
  const output = [
    '   Compiling bridge-core v0.9.0',
    '{ not json',
    JSON.stringify({ reason: 'compiler-artifact' }),
    diagnostic(null, 'bridge-core/src/a.rs', 1),
    diagnostic('clippy::unwrap_used', 'bridge-core/src/a.rs', 2, { level: 'error' }),
  ].join('\n');

  assert.deepEqual(collectCounts(output, TRACKED), {});
});

test('ignores lints the manifest does not ratchet', () => {
  const output = diagnostic('clippy::needless_return', 'bridge-core/src/a.rs', 1);

  assert.deepEqual(collectCounts(output, TRACKED), {});
});

test('reads the warn tier from the workspace lint tables', () => {
  const manifest = [
    '[workspace.lints.rust]',
    'unused_must_use = "deny"',
    'dead-code = "warn"',
    '',
    '[workspace.lints.clippy]',
    'todo = "deny"',
    'unwrap_used = "warn"',
    'panic = "warn" # trailing',
    '',
    '[package]',
    'name = "warn"',
  ].join('\n');

  assert.deepEqual(ratchetedLints(manifest), ['clippy::panic', 'clippy::unwrap_used', 'dead_code']);
});

test('reports a rise in an existing file, a new file and a new lint', () => {
  const baseline = { 'clippy::unwrap_used': { 'a.rs': 2 } };
  const current = {
    'clippy::unwrap_used': { 'a.rs': 3, 'b.rs': 1 },
    'clippy::panic': { 'a.rs': 1 },
  };

  assert.deepEqual(compareCounts(current, baseline).rises, [
    { lint: 'clippy::panic', file: 'a.rs', baseline: 0, current: 1 },
    { lint: 'clippy::unwrap_used', file: 'a.rs', baseline: 2, current: 3 },
    { lint: 'clippy::unwrap_used', file: 'b.rs', baseline: 0, current: 1 },
  ]);
});

test('a violation moved between files is a rise in the destination', () => {
  const baseline = { 'clippy::unwrap_used': { 'a.rs': 2 } };
  const current = { 'clippy::unwrap_used': { 'a.rs': 1, 'b.rs': 1 } };
  const { rises, drops } = compareCounts(current, baseline);

  assert.deepEqual(rises, [{ lint: 'clippy::unwrap_used', file: 'b.rs', baseline: 0, current: 1 }]);
  assert.deepEqual(drops, [{ lint: 'clippy::unwrap_used', file: 'a.rs', baseline: 2, current: 1 }]);
});

test('equal counts produce neither rises nor drops', () => {
  const counts = { 'clippy::unwrap_used': { 'a.rs': 2 } };

  assert.deepEqual(compareCounts(counts, counts), { rises: [], drops: [] });
});

test('totals sums each lint across files', () => {
  assert.deepEqual(totals({ 'clippy::unwrap_used': { 'a.rs': 2, 'b.rs': 5 } }), {
    'clippy::unwrap_used': 7,
  });
});

test('darwin is recorded as macos and linux keeps its name', () => {
  assert.equal(platformKey('darwin'), 'macos');
  assert.equal(platformKey('linux'), 'linux');
});

test('renders each error once and skips warnings', () => {
  const record = (level, rendered) =>
    JSON.stringify({ reason: 'compiler-message', message: { level, rendered } });
  const output = [
    record('error', 'error: boom'),
    record('error', 'error: boom'),
    record('warning', 'warning: meh'),
  ].join('\n');

  assert.deepEqual(renderedErrors(output), ['error: boom']);
});

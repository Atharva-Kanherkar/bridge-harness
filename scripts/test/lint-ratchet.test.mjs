import assert from 'node:assert/strict';
import { test } from 'node:test';

import { compareCounts } from '../clippy-ratchet.mjs';
import {
  collectCounts,
  formatDiagnostic,
  parseDiagnostics,
  risenDiagnostics,
  ruleName,
} from '../lint-ratchet.mjs';

function diagnostic(code, filename, line = 1) {
  return { code, filename, message: `message for ${code}`, labels: [{ span: { line, column: 4 } }] };
}

test('names a rule the way the config does', () => {
  assert.equal(ruleName('shadcn(no-restyle)'), 'shadcn/no-restyle');
  assert.equal(ruleName('bridge(no-raw-controls)'), 'bridge/no-raw-controls');
  assert.equal(ruleName(undefined), 'unknown');
});

test('reads the report even when a plugin prints before it', () => {
  const report = JSON.stringify({ diagnostics: [diagnostic('eslint(no-unused-vars)', 'src/a.ts')] });
  assert.equal(parseDiagnostics(`warming up\n${report}`).length, 1);
  assert.throws(() => parseDiagnostics('no report here'));
});

test('counts violations per rule and file, sorted', () => {
  const counts = collectCounts([
    diagnostic('shadcn(no-restyle)', 'src/b.tsx'),
    diagnostic('shadcn(no-restyle)', 'src/a.tsx'),
    diagnostic('shadcn(no-restyle)', 'src/a.tsx', 9),
    diagnostic('bridge(no-raw-controls)', 'src/a.tsx'),
  ]);
  assert.deepEqual(counts, {
    'bridge/no-raw-controls': { 'src/a.tsx': 1 },
    'shadcn/no-restyle': { 'src/a.tsx': 2, 'src/b.tsx': 1 },
  });
  assert.deepEqual(Object.keys(counts['shadcn/no-restyle']), ['src/a.tsx', 'src/b.tsx']);
});

test('a new file starts at zero and an old file may only go down', () => {
  const baseline = { 'bridge/no-raw-controls': { 'src/old.tsx': 3 } };
  const current = collectCounts([
    diagnostic('bridge(no-raw-controls)', 'src/old.tsx'),
    diagnostic('bridge(no-raw-controls)', 'src/new.tsx'),
  ]);
  const { rises, drops } = compareCounts(current, baseline);
  assert.deepEqual(rises, [
    { lint: 'bridge/no-raw-controls', file: 'src/new.tsx', baseline: 0, current: 1 },
  ]);
  assert.deepEqual(drops, [
    { lint: 'bridge/no-raw-controls', file: 'src/old.tsx', baseline: 3, current: 1 },
  ]);
});

test('prints only the violations in files that rose', () => {
  const all = [
    diagnostic('bridge(no-raw-controls)', 'src/new.tsx', 7),
    diagnostic('shadcn(no-restyle)', 'src/new.tsx'),
    diagnostic('bridge(no-raw-controls)', 'src/old.tsx'),
  ];
  const risen = risenDiagnostics(all, [{ lint: 'bridge/no-raw-controls', file: 'src/new.tsx' }]);
  assert.equal(risen.length, 1);
  assert.match(formatDiagnostic(risen[0]), /src\/new\.tsx:7:4 {2}bridge\/no-raw-controls/);
});

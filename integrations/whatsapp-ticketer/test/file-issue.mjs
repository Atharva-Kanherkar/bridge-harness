import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile, access } from 'node:fs/promises';
import { createFiler, prepareDraft, receiptMarker, validateDraft } from '../src/file-issue.mjs';

const body = '## For humans\nThe sidebar flickers whenever a user changes the active chat. This should stay stable.\n\n## For agents\nReproduce by switching chats repeatedly. Inspect sidebar state and add a regression test.';
const draft = { title: 'Sidebar flickers', kind: 'bug', labels: ['bug', 'invented-label'], body };

test('format failure receives exactly one repair turn with validator errors', async () => {
  const calls = [];
  const result = await prepareDraft(async (context, options) => {
    calls.push(options); return calls.length === 1 ? { ...draft, body: 'bad' } : draft;
  }, {});
  assert.equal(result.body, body);
  assert.equal(calls.length, 2);
  assert.ok(calls[1].repair.problems.some((problem) => problem.includes('For humans')));
  let failures = 0;
  await assert.rejects(prepareDraft(async () => { failures++; return { ...draft, body: 'bad' }; }, {}), /one repair/);
  assert.equal(failures, 2);
});

test('model cannot bypass the issue gate with an exemption', () => {
  for (const output of [{ ...draft, body: '<!-- issue-format: exempt -->' }, { ...draft, labels: ['format-exempt'] }]) {
    assert.ok(validateDraft(output).problems.includes('Format exemptions are not permitted'));
  }
});

test('gh gets argv and a private body file; unknown labels are dropped and temp files removed', async () => {
  let path;
  const filer = createFiler({ run: async (command, args) => {
    assert.equal(command, 'gh');
    assert.deepEqual(args.slice(0, 4), ['issue', 'create', '--repo', 'Atharva-Kanherkar/bridge-harness']);
    assert.equal(args[args.indexOf('--title') + 1], '$(touch /tmp/never) sidebar');
    assert.equal(args.includes('--body'), false);
    assert.equal(args.includes('invented-label'), false);
    assert.equal(args.includes('bug'), true);
    assert.equal(args.includes('from-whatsapp'), true);
    path = args[args.indexOf('--body-file') + 1];
    const written = await readFile(path, 'utf8');
    assert.ok(written.includes(receiptMarker('source')));
    assert.doesNotMatch(written, /9876543210/);
    return { stdout: 'https://github.com/Atharva-Kanherkar/bridge-harness/issues/900\n' };
  } });
  const issue = await filer.create({ ...draft, title: '$(touch /tmp/never) sidebar', body: `${body}\nContact 9876543210` }, 'source', ['from-whatsapp', 'bug']);
  assert.equal(issue.number, 900);
  await assert.rejects(access(path));
});

test('invalid draft never reaches gh; missing required label fails setup', async () => {
  let called = 0;
  const filer = createFiler({ run: async () => { called++; return { stdout: '[]' }; } });
  await assert.rejects(filer.create({ ...draft, body: 'bad' }, 'source', ['from-whatsapp']));
  assert.equal(called, 0);
  await assert.rejects(filer.labels(), /from-whatsapp/);
});

test('reconciliation matches the exact hashed receipt marker', async () => {
  const filer = createFiler({ run: async (_command, args) => {
    assert.equal(args[1], 'list');
    assert.equal(args[args.indexOf('--state') + 1], 'all');
    return { stdout: JSON.stringify([{ number: 1, body: 'unrelated' }, { number: 2, body: `<!-- ${receiptMarker('source')} -->` }]) };
  } });
  assert.equal((await filer.find('source')).number, 2);
});

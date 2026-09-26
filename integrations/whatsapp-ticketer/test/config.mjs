import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, mkdir, rm, symlink, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { loadConfig } from '../src/config.mjs';

test('required config fails closed, defaults model and normalizes sender JIDs', async (t) => {
  const dir = await mkdtemp(join(tmpdir(), 'ticketer-config-'));
  t.after(() => rm(dir, { recursive: true, force: true }));
  await mkdir(join(dir, 'repo'));
  const env = { WA_GROUP_JID: '123@g.us', WA_ALLOWED_SENDERS: '111:2@s.whatsapp.net, 222@lid',
    WA_AUTH_DIR: join(dir, 'auth'), TICKETER_DB: join(dir, 'tickets.sqlite'), REPO_CLONE_DIR: join(dir, 'repo') };
  const config = loadConfig(env);
  assert.deepEqual([...config.senders], ['111@s.whatsapp.net', '222@lid']);
  assert.equal(config.model, 'claude-sonnet-5');
  for (const key of Object.keys(env)) assert.throws(() => loadConfig({ ...env, [key]: '' }), /Missing/);
  assert.throws(() => loadConfig({ ...env, WA_GROUP_JID: '111@s.whatsapp.net' }), /group JID/);
  assert.throws(() => loadConfig({ ...env, WA_ALLOWED_SENDERS: '*' }), /individual/);
  assert.throws(() => loadConfig({ ...env, WA_AUTH_DIR: join(dir, 'repo/auth') }), /outside/);
  await symlink(join(dir, 'repo'), join(dir, 'link'));
  assert.throws(() => loadConfig({ ...env, TICKETER_DB: join(dir, 'link/state.sqlite') }), /outside/);
  await writeFile(join(dir, 'repo/secret'), 'private');
  await symlink(join(dir, 'repo/secret'), join(dir, 'db-link'));
  assert.throws(() => loadConfig({ ...env, TICKETER_DB: join(dir, 'db-link') }), /regular file/);
});

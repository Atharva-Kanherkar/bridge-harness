import { existsSync, lstatSync, mkdirSync, realpathSync } from 'node:fs';
import { dirname, isAbsolute, relative, resolve, sep } from 'node:path';
import { fileURLToPath } from 'node:url';
import { normalizeJid } from './privacy.mjs';

export const REPOSITORY = 'Atharva-Kanherkar/bridge-harness';
export const SERVICE_ROOT = fileURLToPath(new URL('../../../', import.meta.url));

export function isWithin(root, path) {
  const rel = relative(root, path);
  return rel === '' || (!isAbsolute(rel) && rel !== '..' && !rel.startsWith(`..${sep}`));
}

export function loadConfig(env = process.env) {
  const required = (key) => {
    if (!env[key]?.trim()) throw new Error(`Missing ${key}`);
    return env[key].trim();
  };
  const group = required('WA_GROUP_JID');
  const senders = new Set(required('WA_ALLOWED_SENDERS').split(',').map((s) => normalizeJid(s.trim())));
  if (!/^\d[\d-]*@g\.us$/.test(group)) throw new Error('WA_GROUP_JID must be a group JID');
  if ([...senders].some((s) => !/^\d+@(?:s\.whatsapp\.net|lid)$/.test(s))) {
    throw new Error('WA_ALLOWED_SENDERS must contain individual phone or LID JIDs');
  }
  const repoDir = realpathSync(resolve(required('REPO_CLONE_DIR')));
  const authDir = resolve(required('WA_AUTH_DIR'));
  const dbPath = resolve(required('TICKETER_DB'));
  // Resolve parent directories as well, so a symlink cannot put credentials
  // inside either the service checkout or the model's source checkout.
  mkdirSync(authDir, { recursive: true, mode: 0o700 });
  mkdirSync(dirname(dbPath), { recursive: true, mode: 0o700 });
  const realAuth = realpathSync(authDir);
  const realDb = resolve(realpathSync(dirname(dbPath)), dbPath.split(sep).at(-1));
  if (existsSync(realDb) && (!lstatSync(realDb).isFile() || lstatSync(realDb).isSymbolicLink())) {
    throw new Error('TICKETER_DB must be a regular file, not a symlink or directory');
  }
  for (const root of [realpathSync(SERVICE_ROOT), repoDir]) {
    if (isWithin(root, realAuth) || isWithin(root, realDb)) {
      throw new Error('WA_AUTH_DIR and TICKETER_DB must be outside both repository checkouts');
    }
  }
  return { group, senders, repoDir, authDir: realAuth, dbPath: realDb,
    model: env.TICKETER_MODEL?.trim() || 'claude-sonnet-5' };
}

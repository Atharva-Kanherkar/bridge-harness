import { execFile as execFileCallback } from 'node:child_process';
import { createHash } from 'node:crypto';
import { mkdtemp, rm, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { promisify } from 'node:util';
import { checkIssueFormat } from '../../../scripts/issue-format.mjs';
import { parseDraft } from './agent.mjs';
import { REPOSITORY } from './config.mjs';

const execFile = promisify(execFileCallback);
export const receiptMarker = (id) => `whatsapp-ticket:${createHash('sha256').update(id).digest('hex')}`;

export function validateDraft(output) {
  const draft = parseDraft(output);
  const result = checkIssueFormat(draft.body);
  const problems = [...result.problems];
  if (result.exempt || draft.labels.some((label) => label.toLowerCase() === 'format-exempt')) {
    problems.push('Format exemptions are not permitted');
  }
  return { draft, problems };
}

export async function prepareDraft(agent, context, signal) {
  let previous;
  let problems;
  for (let attempt = 0; attempt < 2; attempt++) {
    signal?.throwIfAborted();
    const output = await agent(context, { signal, ...(attempt ? { repair: { previous, problems } } : {}) });
    previous = output;
    const checked = validateDraft(output);
    if (checked.problems.length === 0) return checked.draft;
    problems = checked.problems;
  }
  throw new Error('Issue format is still invalid after one repair');
}

// This service is the sole GitHub writer. No shell, model-generated command,
// repository selector, or action selector ever reaches the CLI.
export function createFiler({ run = execFile, env = process.env } = {}) {
  const gh = (args, signal) => run('gh', args, { env: { ...env, GH_PROMPT_DISABLED: '1' },
    timeout: 15000, maxBuffer: 4 * 1024 * 1024, signal });
  return {
    async labels(signal) {
      const { stdout } = await gh(['label', 'list', '--repo', REPOSITORY, '--limit', '1000', '--json', 'name'], signal);
      const labels = JSON.parse(stdout).map((label) => label.name);
      if (!labels.includes('from-whatsapp')) throw new Error('Create the from-whatsapp label before starting');
      return labels;
    },
    async find(id, signal) {
      const marker = receiptMarker(id);
      const { stdout } = await gh(['issue', 'list', '--repo', REPOSITORY, '--state', 'all',
        '--search', `"${marker}" in:body`, '--limit', '100', '--json', 'number,title,url,body'], signal);
      return JSON.parse(stdout).find((issue) => issue.body?.includes(`<!-- ${marker} -->`)) ?? null;
    },
    async create(draft, id, existingLabels, signal) {
      const checked = validateDraft(draft);
      if (checked.problems.length) throw new Error('Refusing to file an invalid issue');
      const canonical = new Map(existingLabels.map((name) => [name.toLowerCase(), name]));
      if (!canonical.has('from-whatsapp')) throw new Error('Missing from-whatsapp label');
      const labels = [...new Set(['from-whatsapp', ...checked.draft.labels.map((name) => canonical.get(name.toLowerCase()))])]
        .filter((name) => name && name.toLowerCase() !== 'format-exempt');
      const temporary = await mkdtemp(join(tmpdir(), 'bridge-ticketer-issue-'));
      const bodyFile = join(temporary, 'body.md');
      try {
        await writeFile(bodyFile, `${checked.draft.body}\n\n<!-- ${receiptMarker(id)} -->\n`, { mode: 0o600 });
        const { stdout } = await gh(['issue', 'create', '--repo', REPOSITORY, '--title', checked.draft.title,
          '--body-file', bodyFile, ...labels.flatMap((label) => ['--label', label])], signal);
        const url = stdout.trim();
        const match = url.match(/^https:\/\/github\.com\/Atharva-Kanherkar\/bridge-harness\/issues\/(\d+)$/);
        if (!match) throw new Error('GitHub did not return an issue URL');
        return { number: Number(match[1]), title: checked.draft.title, url };
      } finally { await rm(temporary, { recursive: true, force: true }); }
    },
  };
}

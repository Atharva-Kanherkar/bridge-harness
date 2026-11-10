#!/usr/bin/env node
// Validates the PR notes every pull request in this repository carries.
//
// A pull request body answers five questions, each under its own heading:
// what I did, which code it touches, which users it reaches, the tests I ran
// (pasted unedited), and the tradeoffs and what could go wrong. The plan note
// that came before the code (what, why, how) is linked from the body but lives
// outside GitHub, so this check covers only the PR side.
//
// Like scripts/issue-format.mjs, the rule is mechanical so a contributor can
// reproduce it locally: `node scripts/pr-format.mjs body.md`, or pipe a body
// in on stdin. The "PR notes" job in .github/workflows/pr-title.yml runs it.

import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { parseHeadings, visibleText } from './issue-format.mjs';

/** Minimum non-whitespace characters a section needs to count as written. */
export const MIN_SECTION_CHARS = 20;

/** Marker a body can carry to opt out of the gate. */
export const EXEMPT_MARKER = 'pr-format: exempt';

/** Label that exempts a pull request, shared with the issue gate. */
export const EXEMPT_LABEL = 'format-exempt';

export const SECTIONS = [
  {
    key: 'did',
    heading: 'What I did',
    pattern: /what\s+(?:i|we)\s+did|what\s+this\s+pr\s+does|what\s+changed/i,
  },
  { key: 'codebase', heading: 'Codebase affected', pattern: /codebase|code\s+affected/i },
  { key: 'users', heading: 'Users affected', pattern: /users?\s+affected|user\s+impact|who\s+is\s+affected/i },
  { key: 'tests', heading: 'Tests I ran', pattern: /tests?\s+(?:i|we)\s+ran|tests?\s+run\b/i },
  { key: 'risks', heading: 'Tradeoffs and risks', pattern: /trade-?offs?|risks?|what\s+could\s+go\s+wrong/i },
];

const FENCE = /^[ \t]{0,3}(`{3,}|~{3,})[^\n]*\n[\s\S]*?^[ \t]{0,3}\1[ \t]*$/gm;
const FENCE_TOKEN = '\u0000fenced-output\u0000';

/**
 * Pasted test output often contains `## ` lines, so fenced blocks are swapped
 * for a token before headings are parsed. A non-empty block keeps its token,
 * which is how the tests section proves it carries raw output.
 */
function maskFences(markdown) {
  return markdown.replace(FENCE, (block) => {
    const inner = block.split('\n').slice(1, -1).join('\n').trim();
    return inner ? FENCE_TOKEN : '';
  });
}

function matchesSection(title, pattern) {
  return pattern.test(title.replace(/[^\p{L}\p{N}\s/-]/gu, ' '));
}

/** A section runs until the next heading at its own level or higher. */
function sectionBody(headings, position) {
  const { level } = headings[position];
  let text = headings[position].body;
  for (let next = position + 1; next < headings.length && headings[next].level > level; next += 1) {
    text += `\n${headings[next].title}\n${headings[next].body}`;
  }
  return text;
}

/**
 * Check a pull request body against the PR notes rule.
 *
 * @param {string} body raw pull request body markdown
 * @param {{ labels?: string[], isBot?: boolean }} [options]
 * @returns {{ ok: boolean, exempt: boolean, problems: string[], sections: Record<string, boolean> }}
 */
export function checkPullRequestFormat(body, options = {}) {
  const source = String(body ?? '');
  const labels = (options.labels ?? []).map((label) => String(label).toLowerCase());
  const all = Object.fromEntries(SECTIONS.map((section) => [section.key, true]));

  if (options.isBot || labels.includes(EXEMPT_LABEL) || source.includes(EXEMPT_MARKER)) {
    return { ok: true, exempt: true, problems: [], sections: all };
  }

  const headings = parseHeadings(maskFences(source.replace(/\r\n/g, '\n')));
  const problems = [];
  const sections = {};

  for (const section of SECTIONS) {
    const position = headings.findIndex((candidate) => matchesSection(candidate.title, section.pattern));
    if (position === -1) {
      problems.push(`Missing a \`## ${section.heading}\` section.`);
      sections[section.key] = false;
      continue;
    }
    const text = sectionBody(headings, position);
    const written = visibleText(text).replace(/\s+/g, ' ').trim();
    if (written.length < MIN_SECTION_CHARS) {
      problems.push(
        `The \`${headings[position].title}\` section is empty or too thin ` +
          `(${written.length} characters, needs at least ${MIN_SECTION_CHARS}).`,
      );
      sections[section.key] = false;
      continue;
    }
    if (section.key === 'tests' && !text.includes(FENCE_TOKEN)) {
      problems.push(
        `The \`${headings[position].title}\` section needs the commands you ran and their ` +
          'unedited output in a fenced code block.',
      );
      sections[section.key] = false;
      continue;
    }
    sections[section.key] = true;
  }

  return { ok: problems.length === 0, exempt: false, problems, sections };
}

/** The failure text the CI job prints. */
export function failureMessage(problems) {
  return [
    'This pull request body is missing its PR notes (docs/pr-format.md):',
    '',
    ...problems.map((problem) => `- ${problem}`),
    '',
    'Edit the description; the check reruns on every edit. Start from .github/PULL_REQUEST_TEMPLATE.md.',
    `If the PR genuinely does not fit, add the \`${EXEMPT_LABEL}\` label.`,
  ].join('\n');
}

function readBody(path) {
  if (path) return readFileSync(path, 'utf8');
  if (process.env.PR_BODY !== undefined) return process.env.PR_BODY;
  return readFileSync(0, 'utf8');
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) {
  const labels = JSON.parse(process.env.PR_LABELS || '[]');
  const isBot = process.env.PR_AUTHOR_TYPE === 'Bot';
  const result = checkPullRequestFormat(readBody(process.argv[2]), { labels, isBot });
  if (result.exempt) {
    console.log('Exempt from the PR notes check.');
  } else if (result.ok) {
    console.log('PR notes present: what I did, codebase, users, tests, tradeoffs.');
  } else {
    console.error(failureMessage(result.problems));
    process.exit(1);
  }
}

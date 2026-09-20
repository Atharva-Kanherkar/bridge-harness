// Validates the two-audience issue format that this repository requires.
//
// Every issue body must carry a "For humans" section and a "For agents"
// section, in that order, each with real content. Humans get the shape of the
// problem — diagrams, architecture, the why — and agents get the implementation
// detail they need to open a correct pull request without re-deriving it.
//
// The rule is mechanical on purpose: the GitHub Action in
// .github/workflows/issue-format.yml closes non-conforming issues, so the
// check has to be something a contributor can reproduce locally and reason
// about, not a model's opinion.

/** Minimum non-whitespace characters a section needs to count as written. */
export const MIN_SECTION_CHARS = 40;

/** Marker a body can carry to opt out of the gate (used by automation). */
export const EXEMPT_MARKER = 'issue-format: exempt';

/** Label that exempts an issue from the gate. */
export const EXEMPT_LABEL = 'format-exempt';

const SECTIONS = [
  { key: 'humans', heading: 'For humans', pattern: /for\s+humans?\b/i },
  { key: 'agents', heading: 'For agents', pattern: /for\s+agents?\b/i },
];

// `## For humans`, `### 🧑 For humans`, `## For Humans:` all count. A heading
// deeper than h3 is a subsection of something else, so it does not.
const HEADING = /^[ \t]{0,3}(#{2,3})[ \t]+(.+?)[ \t]*$/gm;

/** Strip the parts of a body that are not content a reader would see. */
function visibleText(markdown) {
  return markdown
    .replace(/<!--[\s\S]*?-->/g, '') // HTML comments (template guidance)
    .replace(/^[ \t]*[-*+][ \t]+\[[ x]\][ \t]*$/gm, '') // empty checklist rows
    .replace(/^[ \t]*_?(?:TODO|TBD|N\/A)[.!]?_?[ \t]*$/gim, ''); // placeholders
}

/**
 * Split a markdown body into its h2/h3 headings and the text under each.
 * Returns `[{ level, title, body, index }]` in document order.
 */
export function parseHeadings(markdown) {
  const source = String(markdown ?? '');
  const found = [];
  for (const match of source.matchAll(HEADING)) {
    found.push({
      level: match[1].length,
      title: match[2].replace(/[:：]\s*$/, '').trim(),
      start: match.index ?? 0,
      contentStart: (match.index ?? 0) + match[0].length,
    });
  }
  return found.map((heading, position) => ({
    level: heading.level,
    title: heading.title,
    index: position,
    body: source.slice(
      heading.contentStart,
      position + 1 < found.length ? found[position + 1].start : source.length,
    ),
  }));
}

/** Does this heading name the given section, ignoring emoji and decoration? */
function matchesSection(title, pattern) {
  return pattern.test(title.replace(/[^\p{L}\p{N}\s/]/gu, ' '));
}

/**
 * Check an issue body against the two-audience rule.
 *
 * @param {string} body raw issue body markdown
 * @param {{ labels?: string[] }} [options]
 * @returns {{ ok: boolean, exempt: boolean, problems: string[], sections: Record<string, boolean> }}
 */
export function checkIssueFormat(body, options = {}) {
  const source = String(body ?? '');
  const labels = (options.labels ?? []).map((label) => String(label).toLowerCase());

  if (labels.includes(EXEMPT_LABEL) || source.includes(EXEMPT_MARKER)) {
    return { ok: true, exempt: true, problems: [], sections: { humans: true, agents: true } };
  }

  const headings = parseHeadings(source);
  const problems = [];
  const sections = {};
  const positions = {};

  for (const section of SECTIONS) {
    const heading = headings.find((candidate) => matchesSection(candidate.title, section.pattern));
    if (!heading) {
      problems.push(`Missing a \`## ${section.heading}\` section.`);
      sections[section.key] = false;
      continue;
    }
    positions[section.key] = heading.index;
    const written = visibleText(heading.body).replace(/\s+/g, ' ').trim();
    if (written.length < MIN_SECTION_CHARS) {
      problems.push(
        `The \`${heading.title}\` section is empty or too thin ` +
          `(${written.length} characters, needs at least ${MIN_SECTION_CHARS}).`,
      );
      sections[section.key] = false;
      continue;
    }
    sections[section.key] = true;
  }

  if (
    positions.humans !== undefined &&
    positions.agents !== undefined &&
    positions.humans > positions.agents
  ) {
    problems.push('`For humans` must come before `For agents`.');
  }

  return { ok: problems.length === 0, exempt: false, problems, sections };
}

/** The comment the gate posts when it closes an issue. */
export function rejectionComment(problems, { docsUrl } = {}) {
  const reference = docsUrl ?? 'docs/issue-format.md';
  return [
    'This issue was closed automatically because it does not follow the repository’s two-audience issue format.',
    '',
    ...problems.map((problem) => `- ${problem}`),
    '',
    'Every issue here needs both of these, in this order:',
    '',
    '```markdown',
    '## For humans',
    '',
    'TL;DR, a diagram, the architecture, the decision. Short.',
    '',
    '## For agents',
    '',
    'Files, symbols, wire shapes, acceptance criteria, tests. Long.',
    '```',
    '',
    `Edit the description to add the missing sections and this issue reopens itself. Full rule: [\`${reference}\`](${reference}).`,
    '',
    `If an issue genuinely does not fit the format, add the \`${EXEMPT_LABEL}\` label.`,
  ].join('\n');
}

/** The comment the gate posts when an edit fixes a previously closed issue. */
export function acceptanceComment() {
  return 'Format check passed — both `For humans` and `For agents` sections are present. Reopening.';
}

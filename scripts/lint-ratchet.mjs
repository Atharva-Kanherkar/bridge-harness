#!/usr/bin/env node
// Lint ratchet: no file may gain an oxlint violation. The rules in
// .oxlintrc.json are all errors; the violations that existed when a rule was
// adopted are recorded per rule and file in lint-baseline.json, and that
// count may only go down. A file with no entry starts at zero.
//
//   node scripts/lint-ratchet.mjs              check against the baseline
//   node scripts/lint-ratchet.mjs --update     rewrite the baseline
//   node scripts/lint-ratchet.mjs --input f    read `oxlint -f json` output from a file
//   node scripts/lint-ratchet.mjs --list rule  print every violation of one rule

import { spawnSync } from 'node:child_process';
import { readFileSync, writeFileSync, existsSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

import { compareCounts, totals } from './clippy-ratchet.mjs';

const projectRoot = join(dirname(fileURLToPath(import.meta.url)), '..');
const baselinePath = join(projectRoot, 'lint-baseline.json');
const LINT_PATHS = ['src', 'scripts/oxlint-plugin-bridge'];

/** `shadcn(no-restyle)` as oxlint prints it, `shadcn/no-restyle` as the config names it. */
export function ruleName(code) {
  const match = code?.match(/^([\w-]+)\((.+)\)$/);
  return match ? `${match[1]}/${match[2]}` : (code ?? 'unknown');
}

export function parseDiagnostics(oxlintJson) {
  const start = oxlintJson.indexOf('{');
  if (start === -1) throw new Error('oxlint printed no JSON report');
  return JSON.parse(oxlintJson.slice(start)).diagnostics ?? [];
}

/** Violations per rule and file. */
export function collectCounts(diagnostics) {
  const counts = {};
  for (const diagnostic of diagnostics) {
    const rule = ruleName(diagnostic.code);
    counts[rule] ??= {};
    counts[rule][diagnostic.filename] = (counts[rule][diagnostic.filename] ?? 0) + 1;
  }
  const sorted = {};
  for (const rule of Object.keys(counts).sort()) {
    sorted[rule] = {};
    for (const file of Object.keys(counts[rule]).sort()) sorted[rule][file] = counts[rule][file];
  }
  return sorted;
}

/** The violations in the files and rules that rose, so the fix is on screen. */
export function risenDiagnostics(diagnostics, rises) {
  const risen = new Set(rises.map((entry) => `${entry.lint}\u0000${entry.file}`));
  return diagnostics.filter((d) => risen.has(`${ruleName(d.code)}\u0000${d.filename}`));
}

export function formatDiagnostic(diagnostic) {
  const span = diagnostic.labels?.[0]?.span;
  const where = span ? `${diagnostic.filename}:${span.line}:${span.column}` : diagnostic.filename;
  return `  ${where}  ${ruleName(diagnostic.code)}\n      ${diagnostic.message}`;
}

function runOxlint() {
  const result = spawnSync('npx', ['oxlint', '-f', 'json', ...LINT_PATHS], {
    cwd: projectRoot,
    encoding: 'utf8',
    maxBuffer: 1024 * 1024 * 1024,
    stdio: ['ignore', 'pipe', 'inherit'],
  });
  if (result.error) throw result.error;
  // oxlint exits 1 when it finds errors; anything else is a crash.
  if (result.status !== 0 && result.status !== 1) {
    console.error(`oxlint failed with exit code ${result.status}`);
    process.exit(result.status ?? 1);
  }
  return result.stdout;
}

function main(argv) {
  const inputIndex = argv.indexOf('--input');
  const output = inputIndex === -1 ? runOxlint() : readFileSync(argv[inputIndex + 1], 'utf8');
  const diagnostics = parseDiagnostics(output);
  const current = collectCounts(diagnostics);

  const listIndex = argv.indexOf('--list');
  if (listIndex !== -1) {
    const rule = argv[listIndex + 1];
    for (const d of diagnostics.filter((d) => ruleName(d.code) === rule)) {
      console.log(formatDiagnostic(d));
    }
    return 0;
  }

  if (argv.includes('--update')) {
    writeFileSync(baselinePath, `${JSON.stringify(current, null, 2)}\n`);
    console.log('Wrote lint-baseline.json');
    console.table(totals(current));
    return 0;
  }

  const baseline = existsSync(baselinePath) ? JSON.parse(readFileSync(baselinePath, 'utf8')) : {};
  const { rises, drops } = compareCounts(current, baseline);
  if (drops.length > 0) {
    const removed = drops.reduce((sum, entry) => sum + entry.baseline - entry.current, 0);
    console.log(
      `${removed} violation(s) fixed since the baseline. ` +
        'Lock them in with `bun run lint:update`.',
    );
  }
  if (rises.length > 0) {
    console.error('Lint ratchet failed: these counts rose above the baseline.');
    for (const entry of rises) {
      console.error(`  ${entry.lint}  ${entry.file}  ${entry.baseline} -> ${entry.current}`);
    }
    console.error('\nEvery violation in those files, old and new:');
    for (const d of risenDiagnostics(diagnostics, rises)) console.error(formatDiagnostic(d));
    console.error(
      '\nFix the new violations; do not raise the baseline. docs/design/ui-rules.md says what to use instead.',
    );
    return 1;
  }
  console.log('Lint ratchet passed.');
  return 0;
}

if (import.meta.url === pathToFileURL(process.argv[1] ?? '').href) {
  process.exit(main(process.argv.slice(2)));
}

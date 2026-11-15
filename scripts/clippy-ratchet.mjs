#!/usr/bin/env node
// Clippy ratchet: the warn-tier lints in src-tauri/Cargo.toml may never gain
// a violation in any file. Counts are recorded per platform because cfg-gated
// code differs between Linux and macOS. Only the lints named in the manifest
// are counted, so a new toolchain's new default lints cannot fail the gate.
//
//   node scripts/clippy-ratchet.mjs              check against the baseline
//   node scripts/clippy-ratchet.mjs --update     rewrite this platform's baseline
//   node scripts/clippy-ratchet.mjs --input f    read clippy JSON from a file
//   node scripts/clippy-ratchet.mjs --platform p record or check another platform's section

import { spawnSync } from 'node:child_process';
import { readFileSync, writeFileSync, existsSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const projectRoot = join(dirname(fileURLToPath(import.meta.url)), '..');
const workspaceDir = join(projectRoot, 'src-tauri');
const baselinePath = join(workspaceDir, 'lint-baseline.json');
const manifestPath = join(workspaceDir, 'Cargo.toml');

const CLIPPY_ARGS = [
  'clippy',
  '--workspace',
  '--all-targets',
  '--locked',
  '--message-format=json',
];

export function platformKey(platform = process.platform) {
  return platform === 'darwin' ? 'macos' : platform;
}

/** The lints set to "warn" under [workspace.lints.*], as diagnostic codes. */
export function ratchetedLints(manifest) {
  const lints = [];
  let prefix = null;
  for (const raw of manifest.split('\n')) {
    const line = raw.trim();
    const table = line.match(/^\[(.+)\]$/);
    if (table) {
      const tool = table[1].match(/^workspace\.lints\.(\w+)$/)?.[1];
      prefix = tool === undefined ? null : tool === 'rust' ? '' : `${tool}::`;
      continue;
    }
    if (prefix === null) continue;
    const entry = line.match(/^([\w-]+)\s*=\s*"warn"/);
    if (entry) lints.push(`${prefix}${entry[1].replaceAll('-', '_')}`);
  }
  return lints.sort();
}

/**
 * Count warnings per lint and file. Clippy reports the same diagnostic once
 * per target that compiles a file, so each one is keyed by its primary span.
 */
export function collectCounts(clippyJson, lints) {
  const tracked = new Set(lints);
  const seen = new Set();
  const counts = {};
  for (const line of clippyJson.split('\n')) {
    if (!line.startsWith('{')) continue;
    let record;
    try {
      record = JSON.parse(line);
    } catch {
      continue;
    }
    if (record.reason !== 'compiler-message') continue;
    const message = record.message;
    const lint = message?.code?.code;
    if (!tracked.has(lint) || message.level !== 'warning') continue;
    const span = message.spans?.find((candidate) => candidate.is_primary);
    if (!span) continue;
    const key = [
      lint,
      span.file_name,
      span.line_start,
      span.column_start,
      span.line_end,
      span.column_end,
    ].join('\u0000');
    if (seen.has(key)) continue;
    seen.add(key);
    counts[lint] ??= {};
    counts[lint][span.file_name] = (counts[lint][span.file_name] ?? 0) + 1;
  }
  return sortCounts(counts);
}

/** The human-readable text of each error, which JSON output hides. */
export function renderedErrors(clippyJson) {
  const rendered = new Set();
  for (const line of clippyJson.split('\n')) {
    if (!line.startsWith('{')) continue;
    let record;
    try {
      record = JSON.parse(line);
    } catch {
      continue;
    }
    if (record.reason !== 'compiler-message' || record.message?.level !== 'error') continue;
    if (record.message.rendered) rendered.add(record.message.rendered);
  }
  return [...rendered];
}

function sortCounts(counts) {
  const sorted = {};
  for (const lint of Object.keys(counts).sort()) {
    sorted[lint] = {};
    for (const file of Object.keys(counts[lint]).sort()) {
      sorted[lint][file] = counts[lint][file];
    }
  }
  return sorted;
}

/** Every lint and file whose count differs from the baseline. */
export function compareCounts(current, baseline) {
  const rises = [];
  const drops = [];
  const lints = new Set([...Object.keys(current), ...Object.keys(baseline)]);
  for (const lint of [...lints].sort()) {
    const now = current[lint] ?? {};
    const before = baseline[lint] ?? {};
    const files = new Set([...Object.keys(now), ...Object.keys(before)]);
    for (const file of [...files].sort()) {
      const entry = { lint, file, baseline: before[file] ?? 0, current: now[file] ?? 0 };
      if (entry.current > entry.baseline) rises.push(entry);
      if (entry.current < entry.baseline) drops.push(entry);
    }
  }
  return { rises, drops };
}

export function totals(counts) {
  const result = {};
  for (const [lint, files] of Object.entries(counts)) {
    result[lint] = Object.values(files).reduce((sum, count) => sum + count, 0);
  }
  return result;
}

function runClippy() {
  const result = spawnSync('cargo', CLIPPY_ARGS, {
    cwd: workspaceDir,
    encoding: 'utf8',
    maxBuffer: 1024 * 1024 * 1024,
    stdio: ['ignore', 'pipe', 'inherit'],
  });
  if (result.error) throw result.error;
  if (result.status !== 0) {
    for (const rendered of renderedErrors(result.stdout)) console.error(rendered);
    console.error(`cargo clippy failed with exit code ${result.status}`);
    process.exit(result.status ?? 1);
  }
  return result.stdout;
}

function readBaseline() {
  if (!existsSync(baselinePath)) return {};
  return JSON.parse(readFileSync(baselinePath, 'utf8'));
}

function main(argv) {
  const update = argv.includes('--update');
  const inputIndex = argv.indexOf('--input');
  const clippyJson =
    inputIndex === -1 ? runClippy() : readFileSync(argv[inputIndex + 1], 'utf8');
  const lints = ratchetedLints(readFileSync(manifestPath, 'utf8'));
  const current = collectCounts(clippyJson, lints);
  const platformIndex = argv.indexOf('--platform');
  const platform = platformIndex === -1 ? platformKey() : argv[platformIndex + 1];
  const baseline = readBaseline();

  if (update) {
    baseline[platform] = current;
    const ordered = Object.fromEntries(
      Object.keys(baseline)
        .sort()
        .map((key) => [key, baseline[key]]),
    );
    writeFileSync(baselinePath, `${JSON.stringify(ordered, null, 2)}\n`);
    console.log(`Wrote the ${platform} baseline to src-tauri/lint-baseline.json`);
    console.table(totals(current));
    return 0;
  }

  if (!baseline[platform]) {
    console.error(
      `src-tauri/lint-baseline.json has no ${platform} section. ` +
        'Run `node scripts/clippy-ratchet.mjs --update` on this platform and commit the result.',
    );
    return 1;
  }

  const { rises, drops } = compareCounts(current, baseline[platform]);
  if (drops.length > 0) {
    const removed = drops.reduce((sum, entry) => sum + entry.baseline - entry.current, 0);
    console.log(
      `${removed} violation(s) fixed since the baseline. ` +
        'Lock them in with `node scripts/clippy-ratchet.mjs --update`.',
    );
  }
  if (rises.length > 0) {
    console.error('Clippy ratchet failed: these counts rose above the baseline.');
    for (const entry of rises) {
      console.error(`  ${entry.lint}  ${entry.file}  ${entry.baseline} -> ${entry.current}`);
    }
    console.error('Fix the new violations. The baseline only ever goes down.');
    return 1;
  }
  console.log(`Clippy ratchet passed on ${platform}.`);
  return 0;
}

if (import.meta.url === pathToFileURL(process.argv[1] ?? '').href) {
  process.exit(main(process.argv.slice(2)));
}

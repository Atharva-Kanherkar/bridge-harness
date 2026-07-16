#!/usr/bin/env node

import { execFileSync } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import { dirname, posix } from "node:path";

const OUTPUT = new URL("../src-tauri/src/community_skills.json", import.meta.url);
const LIMIT = 60;
const MAX_PER_SOURCE = 3;
const MAX_FILES_PER_SKILL = 256;
const MAX_BYTES_PER_SKILL = 5 * 1024 * 1024;

function gh(path, extra = []) {
  let lastError;
  for (let attempt = 0; attempt < 3; attempt += 1) {
    try {
      return execFileSync("gh", ["api", path, ...extra], { encoding: "utf8", maxBuffer: 16 * 1024 * 1024 });
    } catch (error) {
      lastError = error;
      Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, 500 * (attempt + 1));
    }
  }
  throw lastError;
}

function parseLeaderboard(html) {
  const marker = "initialSkills\\\":";
  const start = html.indexOf(marker) + marker.length;
  if (start < marker.length) throw new Error("skills.sh leaderboard payload was not found");
  let depth = 0;
  let end = -1;
  for (let index = start; index < html.length; index += 1) {
    if (html[index] === "[") depth += 1;
    if (html[index] === "]" && --depth === 0) { end = index + 1; break; }
  }
  if (end < 0) throw new Error("skills.sh leaderboard payload was incomplete");
  return JSON.parse(html.slice(start, end).replaceAll('\\"', '"').replaceAll("\\\\", "\\"));
}

function eligibleSkills(skills) {
  return skills.filter(skill => skill.source.includes("/") && !skill.isDuplicate && !/^(setup-|template-skill$|simple$)/.test(skill.skillId));
}

function descriptionFromSkill(content, fallback) {
  const frontmatter = content.match(/^---\s*\n([\s\S]*?)\n---/m)?.[1] ?? "";
  const lines = frontmatter.split("\n");
  const index = lines.findIndex(line => /^description\s*:/.test(line));
  if (index >= 0) {
    const first = lines[index].replace(/^description\s*:\s*/, "").trim();
    if (first && first !== ">" && first !== "|") return first.replace(/^['"]|['"]$/g, "").trim();
    const folded = [];
    for (const line of lines.slice(index + 1)) {
      if (/^[A-Za-z][\w-]*\s*:/.test(line)) break;
      if (line.trim()) folded.push(line.trim());
    }
    if (folded.length) return folded.join(" ");
  }
  const heading = content.split("\n").find(line => line.trim() && !line.startsWith("---") && !line.startsWith("#"));
  return heading?.trim() || fallback.replaceAll("-", " ");
}

function nameFromSkill(content) {
  const frontmatter = content.match(/^---\s*\n([\s\S]*?)\n---/m)?.[1] ?? "";
  const value = frontmatter.split("\n").find(line => /^name\s*:/.test(line))?.replace(/^name\s*:\s*/, "").trim();
  return value?.replace(/^['"]|['"]$/g, "") ?? null;
}

function classify(skill, content, files) {
  const haystack = `${skill.skillId} ${content}`.toLowerCase();
  const hasScripts = files.some(path => /(^|\/)scripts?\//.test(path) || /\.(sh|bash|py|js|mjs|ts|rb|ps1)$/.test(path));
  const network = /https?:\/\/|\b(fetch|curl|wget|browser|network|api|mcp)\b/.test(haystack);
  const credentials = /\b(api[-_ ]?key|token|oauth|credential|login|authenticate|secret)\b/.test(haystack);
  const writes = /\b(write|edit|modify|create|delete|deploy|install|commit|push|send|publish)\b/.test(haystack);
  const shell = hasScripts || /```(?:bash|sh|shell)|\b(cli|terminal|command line|execute command)\b/.test(haystack);
  const permissions = ["Read task and workspace context"];
  if (writes) permissions.push("May propose or modify workspace files");
  if (network) permissions.push("May request network or browser tools");
  if (shell) permissions.push("May request shell commands or bundled scripts");
  if (credentials) permissions.push("May request provider authentication or credential references");
  const elevated = credentials || (shell && network);
  const risk = elevated ? "elevated" : shell || writes ? "review" : "low";
  const riskSummary = elevated
    ? "Review authentication, network, and executable instructions before use."
    : risk === "review"
      ? "Review file-writing or command instructions before use."
      : "Instruction-only based on the pinned snapshot; normal agent permissions still apply.";
  const categories = [
    [/react|frontend|ui|design|css|tailwind|shadcn/, "design"],
    [/test|tdd|playwright|browser|review|debug|diagnos/, "quality"],
    [/deploy|azure|vercel|firebase|supabase|convex|cloud/, "infrastructure"],
    [/market|seo|copy|brand|content|social/, "growth"],
    [/video|image|remotion|pptx|media/, "media"],
    [/plan|triage|handoff|brainstorm|research|paper/, "workflow"],
  ].filter(([pattern]) => pattern.test(haystack)).map(([, category]) => category);
  return { permissions, risk, riskSummary, categories: [...new Set(categories.length ? categories : ["development"])] };
}

function candidatePath(paths, slug) {
  const scored = paths.map(path => {
    const parent = posix.basename(posix.dirname(path));
    let score = parent === slug ? 100 : 0;
    if (slug.endsWith(`-${parent}`) || parent.endsWith(`-${slug}`)) score += 80;
    if (path.includes(`/skills/${slug}/`)) score += 50;
    if (path.toLowerCase().includes(slug.toLowerCase())) score += 10;
    const slugTokens = new Set(slug.split("-").filter(token => token.length > 2));
    const sharedTokens = parent.split("-").filter(token => slugTokens.has(token)).length;
    score += sharedTokens * 3;
    return { path, score };
  }).sort((left, right) => right.score - left.score || left.path.length - right.path.length);
  return scored[0]?.score >= 6 ? scored[0].path : null;
}

const leaderboard = eligibleSkills(parseLeaderboard(await (await fetch("https://skills.sh")).text()));
const repositories = new Map();
const unavailableSources = new Set();
const perSource = new Map();
const output = [];
for (const skill of leaderboard) {
  const source = skill.source;
  if (unavailableSources.has(source)) continue;
  if ((perSource.get(source) ?? 0) >= MAX_PER_SOURCE) continue;
  let repository = repositories.get(source);
  if (!repository) {
    let metadata;
    let commit;
    let tree;
    try {
      metadata = JSON.parse(gh(`repos/${source}`));
      commit = JSON.parse(gh(`repos/${source}/commits/${metadata.default_branch}`)).sha;
      tree = JSON.parse(gh(`repos/${source}/git/trees/${commit}?recursive=1`)).tree;
    } catch {
      unavailableSources.add(source);
      process.stderr.write(`Skipped unavailable GitHub source ${source}\n`);
      continue;
    }
    repository = {
      commit,
      tree,
      skillFiles: tree.filter(entry => entry.type === "blob" && entry.path.endsWith("SKILL.md")).map(entry => entry.path),
      contents: new Map(),
    };
    repositories.set(source, repository);
  }
  const load = candidate => {
    if (!repository.contents.has(candidate)) repository.contents.set(candidate, gh(`repos/${source}/contents/${candidate}?ref=${repository.commit}`, ["-H", "Accept: application/vnd.github.raw+json"]));
    return repository.contents.get(candidate);
  };
  let skillPath = candidatePath(repository.skillFiles, skill.skillId);
  let content = skillPath ? load(skillPath) : null;
  if (skillPath && nameFromSkill(content) !== skill.skillId && !skill.skillId.endsWith(`-${nameFromSkill(content)}`)) {
    skillPath = null;
    content = null;
  }
  if (!skillPath) {
    for (const candidate of repository.skillFiles) {
      const candidateContent = load(candidate);
      if (nameFromSkill(candidateContent) === skill.skillId) { skillPath = candidate; content = candidateContent; break; }
    }
  }
  if (!skillPath) {
    process.stderr.write(`Skipped non-installable alias ${source}/${skill.skillId}\n`);
    continue;
  }
  const root = posix.dirname(skillPath);
  if (root.startsWith("/") || posix.normalize(root) !== root || root.split("/").includes("..")) {
    process.stderr.write(`Skipped unsafe path ${source}/${skill.skillId}\n`);
    continue;
  }
  const scopedEntries = repository.tree.filter(entry => entry.path === skillPath || entry.path.startsWith(`${root}/`));
  const unsafeEntry = scopedEntries.some(entry => entry.mode === "120000" || !["blob", "tree"].includes(entry.type));
  const fileEntries = scopedEntries.filter(entry => entry.type === "blob");
  const totalBytes = fileEntries.reduce((sum, entry) => sum + (entry.size ?? 0), 0);
  if (unsafeEntry || fileEntries.length > MAX_FILES_PER_SKILL || totalBytes > MAX_BYTES_PER_SKILL) {
    process.stderr.write(`Skipped unsafe or oversized skill ${source}/${skill.skillId}\n`);
    continue;
  }
  const files = fileEntries.map(entry => entry.path.slice(root.length + 1));
  const classification = classify(skill, content, files);
  output.push({
    id: `${source}/${skill.skillId}`,
    slug: skill.skillId,
    name: skill.name,
    description: descriptionFromSkill(content, skill.name),
    source,
    sourceUrl: `https://github.com/${source}/tree/${repository.commit}/${root}`,
    skillPath: root,
    pinnedRef: repository.commit,
    installs: skill.installs,
    official: skill.isOfficial === true,
    compatibility: ["codex", "claude"],
    fileCount: files.length,
    ...classification,
  });
  perSource.set(source, (perSource.get(source) ?? 0) + 1);
  process.stderr.write(`Pinned ${source}/${skill.skillId}@${repository.commit.slice(0, 8)}\n`);
  if (output.length === LIMIT) break;
}

if (output.length !== LIMIT) throw new Error(`Only ${output.length} installable community skills were found`);
output.sort((left, right) => right.installs - left.installs);
mkdirSync(dirname(OUTPUT.pathname), { recursive: true });
writeFileSync(OUTPUT, `${JSON.stringify(output, null, 2)}\n`);
console.log(`Wrote ${output.length} skills to ${OUTPUT.pathname}`);

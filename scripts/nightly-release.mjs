#!/usr/bin/env node
// Plan a nightly macOS pre-release.
//
// The GitHub Actions cron is 19:30 UTC, which is 01:00 Asia/Kolkata. The tag
// nightly-YYYY-MM-DD names the IST calendar day that just ended. Asia/Kolkata
// is a fixed UTC+05:30 offset (no daylight saving).
import { spawnSync } from "node:child_process";
import { appendFileSync, writeFileSync } from "node:fs";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";

export const IST_OFFSET_MS = (5 * 60 + 30) * 60 * 1000;

const GROUP_ORDER = [
  "Features / Improvements",
  "Bug fixes",
  "Performance",
  "Refactors",
  "Docs",
  "Chores & maintenance",
  "Other",
];

const TYPE_GROUP = {
  feat: "Features / Improvements",
  fix: "Bug fixes",
  perf: "Performance",
  refactor: "Refactors",
  docs: "Docs",
  chore: "Chores & maintenance",
  ci: "Chores & maintenance",
  build: "Chores & maintenance",
  test: "Chores & maintenance",
  style: "Chores & maintenance",
};

export function assertIsoDate(value) {
  if (!/^\d{4}-\d{2}-\d{2}$/.test(value)) {
    throw new Error(`nightly date must be YYYY-MM-DD, got ${JSON.stringify(value)}`);
  }
  const [year, month, day] = value.split("-").map(Number);
  const utc = new Date(Date.UTC(year, month - 1, day));
  if (utc.getUTCFullYear() !== year || utc.getUTCMonth() !== month - 1 || utc.getUTCDate() !== day) {
    throw new Error(`nightly date is not a real calendar day: ${value}`);
  }
  return value;
}

export function istCalendarDay(instant) {
  const shifted = new Date(instant.getTime() + IST_OFFSET_MS);
  const year = shifted.getUTCFullYear();
  const month = String(shifted.getUTCMonth() + 1).padStart(2, "0");
  const day = String(shifted.getUTCDate()).padStart(2, "0");
  return `${year}-${month}-${day}`;
}

// The IST day that just ended. At 01:00 IST this is yesterday's calendar date.
export function shippedIstDate(now) {
  const today = istCalendarDay(now);
  const [year, month, day] = today.split("-").map(Number);
  const istMidnightUtc = Date.UTC(year, month - 1, day) - IST_OFFSET_MS;
  return istCalendarDay(new Date(istMidnightUtc - 1000));
}

export function istDayBounds(isoDate) {
  assertIsoDate(isoDate);
  const [year, month, day] = isoDate.split("-").map(Number);
  const start = Date.UTC(year, month - 1, day) - IST_OFFSET_MS;
  return { start: new Date(start), end: new Date(start + 24 * 60 * 60 * 1000) };
}

// Cron fires one hour after the IST day ends (19:30 UTC). Include that second.
export function mainSnapshotBefore(isoDate) {
  const cron = istDayBounds(isoDate).end.getTime() + 60 * 60 * 1000;
  return new Date(cron + 1000).toISOString();
}

export function searchQuery(repo, isoDate) {
  const { start, end } = istDayBounds(isoDate);
  const startDay = start.toISOString().slice(0, 10);
  const endDay = new Date(end.getTime() - 1).toISOString().slice(0, 10);
  return `repo:${repo} is:pr is:merged base:main merged:${startDay}..${endDay}`;
}

export function conventionalType(title) {
  const match = String(title).trim().match(/^([A-Za-z]+)(?:\([^)\n]*\))?!?:\s*/);
  return match ? match[1].toLowerCase() : null;
}

export function groupForTitle(title) {
  const type = conventionalType(title);
  return (type && TYPE_GROUP[type]) || "Other";
}

export function selectPullRequests(prs, isoDate) {
  const { start, end } = istDayBounds(isoDate);
  const startMs = start.getTime();
  const endMs = end.getTime();
  const seen = new Set();
  const selected = [];
  for (const pr of prs) {
    const mergedMs = Date.parse(pr.mergedAt);
    if (!Number.isFinite(mergedMs) || mergedMs < startMs || mergedMs >= endMs) continue;
    if (!Number.isInteger(pr.number) || pr.number <= 0) continue;
    if (seen.has(pr.number)) continue;
    seen.add(pr.number);
    selected.push(pr);
  }
  selected.sort((a, b) => Date.parse(a.mergedAt) - Date.parse(b.mergedAt) || a.number - b.number);
  return selected;
}

function authorSuffix(author) {
  return typeof author === "string" && /^[A-Za-z0-9-]+$/.test(author) ? ` by @${author}` : "";
}

function prLink(pr) {
  const safe = typeof pr.url === "string" && /^https:\/\/github\.com\/[^)\s]+$/.test(pr.url);
  return safe ? `[#${pr.number}](${pr.url})` : `#${pr.number}`;
}

export function renderNightlyNotes({ date, sha, prs }) {
  assertIsoDate(date);
  if (!/^[0-9a-f]{40}$/.test(sha)) throw new Error("notes require the full main commit SHA");
  const buckets = new Map(GROUP_ORDER.map((group) => [group, []]));
  for (const pr of selectPullRequests(prs, date)) {
    buckets.get(groupForTitle(pr.title)).push(pr);
  }
  const lines = [
    `Nightly macOS build for ${date} (Asia/Kolkata).`,
    "",
    "Pre-release only. This tag does not become GitHub Latest and does not update the stable in-app updater.",
    "",
    `Built from \`main\` at \`${sha}\`.`,
    "",
  ];
  for (const group of GROUP_ORDER) {
    const items = buckets.get(group);
    if (items.length === 0) continue;
    lines.push(`## ${group}`, "");
    for (const pr of items) {
      lines.push(`- ${String(pr.title).replace(/\s+/g, " ").trim()} (${prLink(pr)})${authorSuffix(pr.author)}`);
    }
    lines.push("");
  }
  lines.push("Claude requires Node 18 or later on PATH.", "");
  return lines.join("\n");
}

export function planNightly({ now, date, prs, releaseExists, tagExists, sha }) {
  const shipped = date ? assertIsoDate(date) : shippedIstDate(now);
  const tag = `nightly-${shipped}`;
  if (releaseExists) return { skip: true, reason: "release-exists", tag, date: shipped, sha: "", notes: "" };
  if (tagExists) return { skip: true, reason: "tag-exists", tag, date: shipped, sha: "", notes: "" };
  const included = selectPullRequests(prs, shipped);
  if (included.length === 0) return { skip: true, reason: "no-merged-prs", tag, date: shipped, sha: "", notes: "" };
  if (!/^[0-9a-f]{40}$/.test(sha)) throw new Error("publishing a nightly requires the full main commit SHA");
  return {
    skip: false,
    reason: "publish",
    tag,
    date: shipped,
    sha,
    notes: renderNightlyNotes({ date: shipped, sha, prs: included }),
  };
}

function gh(args) {
  return spawnSync("gh", args, { encoding: "utf8" });
}

function missingIsOk(out, label) {
  if (out.status === 0) return false;
  const text = `${out.stderr || ""}\n${out.stdout || ""}`;
  if (/not found/i.test(text)) return true;
  throw new Error(`unable to check ${label}: ${text.trim() || `exit ${out.status}`}`);
}

export function createDefaultIo(env) {
  const repo = env.GITHUB_REPOSITORY;
  if (!repo) throw new Error("GITHUB_REPOSITORY is required");
  return {
    releaseExists(tag) {
      const args = ["release", "view", tag, "--repo", repo, "--json", "tagName"];
      return !missingIsOk(gh(args), `release ${tag}`);
    },
    tagExists(tag) {
      const args = ["api", `repos/${repo}/git/ref/tags/${tag}`];
      return !missingIsOk(gh(args), `tag ${tag}`);
    },
    fetchPullRequests(isoDate) {
      const q = searchQuery(repo, isoDate);
      const items = [];
      for (let page = 1; page <= 10; page += 1) {
        const path = `search/issues?q=${encodeURIComponent(q)}&per_page=100&page=${page}`;
        const out = gh(["api", "-H", "Accept: application/vnd.github+json", path]);
        if (out.status !== 0) throw new Error(`PR search failed: ${(out.stderr || out.stdout || "").trim()}`);
        const body = JSON.parse(out.stdout);
        if (body.incomplete_results) throw new Error("GitHub search returned incomplete results");
        const pageItems = Array.isArray(body.items) ? body.items : [];
        items.push(...pageItems);
        if (pageItems.length === 0 || items.length >= body.total_count) break;
        if (page === 10 && items.length < body.total_count) {
          throw new Error(`PR search truncated at ${items.length} of ${body.total_count}`);
        }
      }
      return items.map((item) => ({
        number: item.number,
        title: item.title,
        url: item.html_url,
        mergedAt: item.pull_request && item.pull_request.merged_at,
        author: item.user && item.user.login,
      }));
    },
    resolveSha(isoDate) {
      const before = mainSnapshotBefore(isoDate);
      const out = spawnSync("git", ["rev-list", "-1", `--before=${before}`, "origin/main"], { encoding: "utf8" });
      if (out.status !== 0) throw new Error((out.stderr || "git rev-list failed").trim());
      const sha = out.stdout.trim();
      if (!/^[0-9a-f]{40}$/.test(sha)) throw new Error(`no main commit at or before ${before}`);
      return sha;
    },
  };
}

export function runPlan(env, io) {
  const explicit = (env.NIGHTLY_DATE || "").trim();
  const now = env.NIGHTLY_NOW ? new Date(env.NIGHTLY_NOW) : new Date();
  if (Number.isNaN(now.getTime())) throw new Error("invalid NIGHTLY_NOW");
  const date = explicit || shippedIstDate(now);
  assertIsoDate(date);
  const client = io || createDefaultIo(env);
  const tag = `nightly-${date}`;
  const releaseExists = client.releaseExists(tag);
  const tagExists = releaseExists ? false : client.tagExists(tag);
  if (releaseExists || tagExists) {
    return planNightly({ date, prs: [], releaseExists, tagExists, sha: "" });
  }
  const prs = client.fetchPullRequests(date);
  const included = selectPullRequests(prs, date);
  const sha = included.length === 0 ? "" : client.resolveSha(date);
  return planNightly({ date, prs, releaseExists: false, tagExists: false, sha });
}

function writeOutputs(file, values) {
  const body = Object.entries(values).map(([key, value]) => `${key}=${value}`).join("\n") + "\n";
  if (file) appendFileSync(file, body);
  else process.stdout.write(body);
}

function main() {
  const result = runPlan(process.env);
  if (!result.skip) {
    const notesPath = process.env.NIGHTLY_NOTES_PATH;
    if (!notesPath) throw new Error("NIGHTLY_NOTES_PATH is required when publishing");
    writeFileSync(notesPath, result.notes);
  }
  writeOutputs(process.env.GITHUB_OUTPUT, {
    skip: result.skip ? "true" : "false",
    reason: result.reason,
    tag: result.tag,
    date: result.date,
    sha: result.sha,
  });
  console.log(`nightly ${result.tag}: ${result.reason}`);
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) main();

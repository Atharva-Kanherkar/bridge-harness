#!/usr/bin/env node
// Auto-review and incremental re-review end to end, in the browser preview
// linked to a real `bridged --listen` (testing/feat-review-auto-incremental.md,
// E2E): Settings persists the three controls, then a posted review, a new
// commit and a re-review that leaves the repeat out, marks the posted finding
// fixed and raises only what is new. Fully offline: GitHub
// is a local bare repository holding PR #103 plus the shared fake `gh`
// (testing/fixtures/github), and the reviewer is Bridge's real Claude sidecar
// against the fake SDK (testing/fixtures/workflow-e2e). Everything lives in a
// scratch folder with its own HOME.
//
//   BRIDGED_BIN=<path to bridged> BRIDGE_PLAYWRIGHT_MODULE=<playwright> \
//     node scripts/e2e-review-auto-incremental.mjs [--out <screenshot dir>]
import assert from "node:assert/strict";
import { execFileSync, spawn } from "node:child_process";
import fs from "node:fs";
import net from "node:net";
import os from "node:os";
import path from "node:path";

const { chromium } = await import(process.env.BRIDGE_PLAYWRIGHT_MODULE ?? "playwright");
const repoRoot = path.resolve(path.dirname(new URL(import.meta.url).pathname), "..");
const bridged = process.env.BRIDGED_BIN;
assert.ok(bridged && fs.existsSync(bridged), "set BRIDGED_BIN to a built bridged");
const outIndex = process.argv.indexOf("--out");
const out = outIndex > 0 ? path.resolve(process.argv[outIndex + 1]) : fs.mkdtempSync(path.join(os.tmpdir(), "review-incremental-shots-"));
fs.mkdirSync(out, { recursive: true });

const scratch = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), "review-incremental-e2e-")));
const git = (cwd, ...args) => execFileSync("git", args, { cwd, encoding: "utf8" }).trim();
const freePort = () => new Promise(resolve => { const server = net.createServer().listen(0, "127.0.0.1", () => { const { port } = server.address(); server.close(() => resolve(port)); }); });
const waitFor = async (check, what, ms = 30_000) => {
  const deadline = Date.now() + ms;
  while (Date.now() < deadline) { if (await check()) return; await new Promise(resolve => setTimeout(resolve, 100)); }
  throw new Error(`timed out waiting for ${what}`);
};

// A repository whose origin stands in for GitHub, with PR #103 changing README.md line 2.
const repo = path.join(scratch, "repo");
const github = path.join(scratch, "github.git");
fs.mkdirSync(repo);
git(repo, "init", "-q", "-b", "main");
for (const [key, value] of [["user.email", "t@example.invalid"], ["user.name", "T"], ["commit.gpgsign", "false"]]) git(repo, "config", key, value);
fs.writeFileSync(path.join(repo, "README.md"), "base\n");
git(repo, "add", "."); git(repo, "commit", "-q", "-m", "base");
git(scratch, "init", "-q", "--bare", "-b", "main", github);
git(repo, "remote", "add", "origin", github);
git(repo, "push", "-q", "origin", "main");
git(repo, "checkout", "-q", "-b", "pr");
fs.writeFileSync(path.join(repo, "README.md"), "base\nchanged by the PR\n");
git(repo, "commit", "-q", "-am", "the PR");
git(repo, "push", "-q", "origin", "HEAD:refs/pull/103/head");
git(repo, "checkout", "-q", "main");

// The shared fake gh, first on PATH, resolving the repository by its default.
const gh = path.join(scratch, "gh");
fs.mkdirSync(gh);
const fixtures = path.join(repoRoot, "testing/fixtures/github");
fs.writeFileSync(path.join(gh, "fixture-root"), fixtures);
fs.writeFileSync(path.join(gh, "auth-exit"), "0");
fs.writeFileSync(path.join(gh, "default-repository"), "fixture/project");
fs.symlinkSync(path.join(fixtures, "fake-gh.sh"), path.join(gh, "gh"));
fs.writeFileSync(path.join(gh, "review-comments.json"), JSON.stringify([{ id: 9001, pull_request_review_id: 555, path: "README.md", line: 2, original_line: 2, side: "RIGHT", body: "posted" }]));

const e2e = path.join(repoRoot, "testing/fixtures/workflow-e2e");
const home = path.join(scratch, "home");
fs.mkdirSync(home);
const data = path.join(scratch, "data");
const env = {
  HOME: home, USER: os.userInfo().username, TMPDIR: os.tmpdir(), LANG: "en_US.UTF-8",
  PATH: [gh, path.join(e2e, "bin"), path.dirname(process.execPath), "/opt/homebrew/bin", "/usr/local/bin", "/usr/bin", "/bin"].join(":"),
  BRIDGE_CLAUDE_SIDECAR: path.join(e2e, "sidecar.mjs"),
};

const vitePort = await freePort();
const daemonPort = await freePort();
const origin = `http://127.0.0.1:${vitePort}`;
const daemon = (extra = []) => spawn(bridged, ["--data-dir", data, "--socket", path.join(scratch, "d.sock"), "--health-addr", "none", ...extra], { env, stdio: ["ignore", "pipe", "pipe"] });
const children = [];
const stopAll = () => { for (const child of children) child.kill("SIGTERM"); };
process.on("exit", stopAll);

// The first start creates the store; then a project, its workspace and a chat.
const boot = daemon();
await waitFor(() => fs.existsSync(path.join(data, "daemon.token")) && fs.existsSync(path.join(scratch, "d.sock")), "the first daemon start");
boot.kill("SIGTERM");
await new Promise(resolve => boot.on("exit", resolve));
execFileSync("sqlite3", [path.join(data, "bridge.db"), `
  INSERT INTO projects(id,name,path,created_at) VALUES('p','Demo','${repo}','now');
  INSERT INTO workspaces(id,project_id,city,title,branch,path,status,created_at) VALUES('w','p','Oslo','Demo','main','${repo}','ready','now');
  INSERT INTO sessions(id,workspace_id,harness,label,status,kind) VALUES('chat','w','claude','Chat','ready','direct');`]);

const live = daemon(["--listen", `127.0.0.1:${daemonPort}`, "--allowed-origin", origin]);
children.push(live);
let daemonLog = "";
live.stdout.on("data", chunk => { daemonLog += chunk; });
live.stderr.on("data", chunk => { daemonLog += chunk; });
await waitFor(() => daemonLog.includes("WebSocket"), "the daemon's WebSocket listener");
const token = fs.readFileSync(path.join(data, "daemon.token"), "utf8").trim();

const vite = spawn(path.join(repoRoot, "node_modules/.bin/vite"), ["--port", String(vitePort), "--strictPort", "--host", "127.0.0.1"], { cwd: repoRoot, stdio: ["ignore", "pipe", "pipe"] });
children.push(vite);
await waitFor(async () => { try { return (await fetch(origin)).ok; } catch { return false; } }, "the preview server", 60_000);

const sql = query => execFileSync("sqlite3", [path.join(data, "bridge.db"), query], { encoding: "utf8" }).trim();
const link = `${origin}/#bridge-daemon=${encodeURIComponent(`ws://127.0.0.1:${daemonPort}`)}&bridge-token=${encodeURIComponent(token)}`;

const openSettings = async page => {
  await page.getByText("Settings", { exact: true }).first().click();
  const advanced = page.getByRole("button", { name: "Advanced" }).first();
  await advanced.waitFor({ timeout: 15_000 });
  if (!(await page.getByText("Background tasks", { exact: true }).first().isVisible())) await advanced.click();
  await page.getByText("Background tasks", { exact: true }).first().click({ timeout: 15_000 });
  const group = page.locator('[data-settings-row="review-jobs"]');
  await group.waitFor({ timeout: 15_000 });
  await group.scrollIntoViewIfNeeded();
  return group;
};
const openPullRequest = async page => {
  await page.getByText("New chat", { exact: true }).first().click({ timeout: 30_000 });
  await page.getByRole("button", { name: "GitHub", exact: true }).first().click({ timeout: 30_000 });
  const pane = page.locator("[data-gh-pane-root]");
  await pane.getByText("Changes requested", { exact: true }).first().click({ timeout: 30_000 });
  return pane;
};
/** Review with Claude from the pane and wait for its draft; returns the job id. */
const review = async (page, pane) => {
  const before = await pane.locator("[data-review-job]").evaluateAll(rows => rows.map(row => row.getAttribute("data-review-job")));
  await pane.getByRole("button", { name: "Review", exact: true }).click();
  await pane.getByRole("group", { name: "Review harness" }).getByRole("button", { name: "Claude", exact: true }).click();
  await page.waitForFunction(known => [...document.querySelectorAll("[data-review-job]")].some(row => !known.includes(row.getAttribute("data-review-job"))), before);
  const jobId = await pane.locator("[data-review-job]").evaluateAll((rows, known) => rows.map(row => row.getAttribute("data-review-job")).find(id => !known.includes(id)), before);
  await pane.locator(`[data-review-draft="${jobId}"] [data-finding]`).first().waitFor({ timeout: 120_000 });
  return jobId;
};

const browser = await chromium.launch({ headless: true });
const results = {};
try {
  const page = await browser.newPage({ viewport: { width: 1440, height: 900 }, colorScheme: "light" });
  const errors = [];
  page.on("pageerror", error => errors.push(String(error)));
  await page.goto(link);

  // 1. Settings: the three controls, saved and still there after a reload.
  let group = await openSettings(page);
  assert.match(await group.innerText(), /Reviews start only when you ask/);
  await group.getByRole("button", { name: "All PRs", exact: true }).click();
  await group.getByText("Every open pull request is reviewed").waitFor();
  await group.getByRole("switch", { name: "Incremental re-review" }).click();
  await group.getByText("remembers what was kept, dropped or resolved").waitFor();
  await page.waitForTimeout(400);
  await group.screenshot({ path: path.join(out, "light-1-settings.png") });
  await page.reload();
  group = await openSettings(page);
  assert.equal(await group.getByRole("switch", { name: "Incremental re-review" }).getAttribute("aria-checked"), "true", "incremental persisted");
  assert.match(await group.innerText(), /Every open pull request is reviewed/, "auto-review persisted");
  assert.equal(sql("SELECT json_extract(payload,'$.autoReview')||','||json_extract(payload,'$.incremental') FROM configuration_entries WHERE kind='review_job_settings'"), "all,1");
  // Auto-review stays out of the rest of the run: its trigger has its own tests.
  await group.getByRole("button", { name: "Off", exact: true }).click();
  await group.getByText("Reviews start only when you ask").waitFor();

  // 2. A first review, kept and posted.
  await page.goto(link);
  let pane = await openPullRequest(page);
  const first = await review(page, pane);
  const firstDraft = pane.locator(`[data-review-draft="${first}"]`);
  await firstDraft.locator("[data-finding]").first().focus();
  await page.keyboard.press("k");
  await page.waitForFunction(id => document.querySelector(`[data-review-draft="${id}"] [data-finding]`)?.getAttribute("data-decision") === "kept", first);
  await firstDraft.getByRole("button", { name: "Post 1 finding" }).click();
  await pane.locator(`[data-review-job="${first}"]`).getByRole("link", { name: /View on GitHub/ }).waitFor({ timeout: 60_000 });
  const firstHead = sql(`SELECT head_sha FROM review_jobs WHERE id='${first}'`);

  // 3. A new commit lands on the PR.
  git(repo, "checkout", "-q", firstHead);
  fs.writeFileSync(path.join(repo, "README.md"), "base\nchanged by the PR\nthe PR again\n");
  git(repo, "commit", "-q", "-am", "a later push");
  git(repo, "push", "-q", "-f", "origin", "HEAD:refs/pull/103/head");
  const newHead = git(repo, "rev-parse", "HEAD");
  git(repo, "checkout", "-q", "main");

  // 4. The re-review: since the posted head, the repeat left out, the posted
  // finding marked fixed, only the new finding to triage.
  const second = await review(page, pane);
  const row = pane.locator(`[data-review-job="${second}"]`);
  assert.match(await row.innerText(), new RegExp(`Re-review since ${firstHead.slice(0, 7)}`));
  const findings = pane.locator(`[data-review-draft="${second}"] [data-finding]`);
  assert.equal(await findings.count(), 1, "only what is new");
  assert.match(await findings.first().innerText(), /README\.md:3/);
  assert.match(await findings.first().innerText(), /The third line repeats the second/);
  assert.equal(sql(`SELECT head_sha FROM review_jobs WHERE id='${second}'`), newHead);
  assert.match(sql(`SELECT summary FROM review_jobs WHERE id='${second}'`), new RegExp(`^Re-review of ${firstHead.slice(0, 12)}\\.\\.${newHead.slice(0, 12)}`));
  assert.equal(sql(`SELECT fixed_in_sha FROM review_findings WHERE job_id='${first}'`), newHead, "the posted finding is marked fixed");
  await pane.getByRole("region", { name: "Reviews" }).scrollIntoViewIfNeeded();
  await pane.screenshot({ path: path.join(out, "light-2-re-review.png") });
  results.light = { errors: [...errors] };

  // 5. The same screens in dark.
  const dark = await browser.newPage({ viewport: { width: 1440, height: 900 }, colorScheme: "dark" });
  const darkErrors = [];
  dark.on("pageerror", error => darkErrors.push(String(error)));
  await dark.goto(link);
  pane = await openPullRequest(dark);
  await pane.locator(`[data-review-draft="${second}"] [data-finding]`).first().waitFor({ timeout: 30_000 });
  await pane.getByRole("region", { name: "Reviews" }).scrollIntoViewIfNeeded();
  await pane.screenshot({ path: path.join(out, "dark-2-re-review.png") });
  group = await openSettings(dark);
  await group.screenshot({ path: path.join(out, "dark-1-settings.png") });
  results.dark = { errors: darkErrors };
  results.jobs = { first, second, firstHead, newHead };
} finally {
  await browser.close();
  stopAll();
}
for (const [scheme, result] of Object.entries(results)) if (result.errors) assert.deepEqual(result.errors, [], `${scheme} page errors`);
console.log(JSON.stringify({ out, scratch, results }, null, 2));

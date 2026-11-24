#!/usr/bin/env node
// The Reviews dashboard end to end, in the browser preview linked to a real
// `bridged --listen` (testing/feat-review-dashboard.md, E2E). The store is
// seeded with reviews in two repositories as the runner leaves them; the
// dashboard must list both, show the numbers the store holds, and keep and
// post a draft through the real post path to the shared fake `gh`.
// Everything lives in a scratch folder with its own HOME.
//
//   BRIDGED_BIN=<path to bridged> BRIDGE_PLAYWRIGHT_MODULE=<playwright> \
//     node scripts/e2e-review-dashboard.mjs [--out <screenshot dir>]
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
const out = outIndex > 0 ? path.resolve(process.argv[outIndex + 1]) : fs.mkdtempSync(path.join(os.tmpdir(), "review-dashboard-shots-"));
fs.mkdirSync(out, { recursive: true });

const scratch = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), "review-dashboard-e2e-")));
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
const prHead = git(repo, "rev-parse", "HEAD");
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

let first = daemon();
await waitFor(() => fs.existsSync(path.join(data, "daemon.token")) && fs.existsSync(path.join(scratch, "d.sock")), "the first daemon start");
first.kill("SIGTERM");
await new Promise(resolve => first.on("exit", resolve));

// Reviews as the runner leaves them, minutes apart, in two repositories:
// fixture/project (this workspace) and octo/web (a second workspace).
const ago = minutes => new Date(Date.now() - minutes * 60_000).toISOString();
const sql = value => value === null ? "NULL" : `'${String(value).replace(/'/g, "''")}'`;
const seeds = [];
const seed = ({ id, workspace = "w", owner = "fixture", name = "project", number, title, status, harnesses, minutes, took, findings = [], cost = [] }) => {
  const created = ago(minutes);
  const done = new Date(Date.parse(created) + took * 1000).toISOString();
  const terminal = ["posted", "failed", "cancelled", "interrupted"].includes(status);
  seeds.push(`INSERT INTO review_jobs(id,workspace_id,repo_host,repo_owner,repo_name,pr_number,head_sha,trigger_kind,post_mode,harnesses,dedupe_key,publication_key,status,summary,attempts,created_at,started_at,finished_at,pr_title,pr_author,error)
    VALUES(${sql(id)},${sql(workspace)},'github.com',${sql(owner)},${sql(name)},${number},${sql(number === 103 ? prHead : "sha-" + id)},'manual','draft',${sql(JSON.stringify(harnesses))},${sql("dedupe-" + id)},${sql(id)},${sql(status)},${sql(status === "failed" ? null : "Looks mostly fine; one real problem.")},1,${sql(created)},${sql(created)},${sql(terminal ? done : null)},${sql(title)},'someone',${sql(status === "failed" ? "codex could not review: Codex is signed out" : null)});`);
  harnesses.forEach((harness, slot) => seeds.push(`INSERT INTO review_passes(id,job_id,slot,attempt,harness,status,error,tokens_in,tokens_out,cost_microusd,created_at,started_at,finished_at)
    VALUES(${sql(`${id}-${harness}`)},${sql(id)},${slot},1,${sql(harness)},${sql(status === "failed" ? "failed" : "done")},${sql(status === "failed" ? "Codex is signed out" : null)},${harness === "claude" ? 40000 : 900000},${harness === "claude" ? 1200 : 3000},${cost[slot] ?? 0},${sql(created)},${sql(created)},${sql(done)});`));
  findings.forEach(([severity, path, line, findingTitle, decision, extra = {}], index) => seeds.push(`INSERT INTO review_findings(id,job_id,pass_id,path,line,side,severity,title,body,confidence,fingerprint,agreed_by,pass_ids,outside_diff,decision,fixed_in_sha,posted_at,created_at)
    VALUES(${sql(`${id}-f${index}`)},${sql(id)},${sql(`${id}-${extra.harness ?? harnesses[0]}`)},${sql(path)},${line},'RIGHT',${sql(severity)},${sql(findingTitle)},${sql("Why it matters, and the fix.")},0.8,${sql(`fp-${id}-${index}`)},${extra.agreed ? 2 : 1},${extra.agreed ? sql(JSON.stringify(harnesses.map(harness => `${id}-${harness}`))) : "NULL"},0,${sql(decision)},${sql(extra.fixed ? "later-sha" : null)},${sql(status === "posted" && decision !== "dropped" ? done : null)},${sql(created)});`));
};
seed({ id: "draft", number: 103, title: "Changes requested", status: "drafted", harnesses: ["claude", "codex"], minutes: 30, took: 150, cost: [410000, 0],
  findings: [["critical", "README.md", 2, "Token is written to the log", "pending", { agreed: true }], ["minor", "README.md", 2, "Name the constant", "pending"]] });
seed({ id: "posted-web", workspace: "w2", owner: "octo", name: "web", number: 51, title: "Cache the session lookup", status: "posted", harnesses: ["claude"], minutes: 600, took: 95, cost: [380000],
  findings: [["major", "src/session.ts", 40, "Cache never expires", "kept", { fixed: true }], ["nit", "src/session.ts", 12, "Unused import", "dropped"]] });
seed({ id: "posted-api", number: 99, title: "Retry on a busy store", status: "posted", harnesses: ["opencode"], minutes: 1500, took: 240, cost: [1900000],
  findings: [["major", "src/store.rs", 88, "Retry loop never gives up", "edited"]] });
seed({ id: "failed", workspace: "w2", owner: "octo", name: "web", number: 52, title: "Bump deps", status: "failed", harnesses: ["codex"], minutes: 200, took: 20 });
execFileSync("sqlite3", [path.join(data, "bridge.db"), `
  INSERT INTO projects(id,name,path,created_at) VALUES('p','Demo','${repo}','now');
  INSERT INTO workspaces(id,project_id,city,title,branch,path,status,created_at) VALUES('w','p','Oslo','Demo','main','${repo}','ready','now');
  INSERT INTO sessions(id,workspace_id,harness,label,status,kind) VALUES('chat','w','claude','Chat','ready','direct');
  ${seeds.join("\n")}`]);

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

const openReviews = async page => {
  await page.getByText("Apps", { exact: true }).first().click({ timeout: 30_000 });
  await page.getByRole("button", { name: /^Reviews/ }).first().click({ timeout: 15_000 });
  await page.getByRole("heading", { name: "Reviews", exact: true }).waitFor({ timeout: 30_000 });
};
const tileText = async (page, label) => (await page.locator(`[data-tile="${label}"]`).innerText()).replace(/\s+/g, " ");

const browser = await chromium.launch({ headless: true });
const results = [];
try {
  for (const scheme of ["light", "dark"]) {
    const page = await browser.newPage({ viewport: { width: 1440, height: 1000 }, colorScheme: scheme });
    const errors = [];
    page.on("pageerror", error => errors.push(String(error)));
    await page.goto(`${origin}/#bridge-daemon=${encodeURIComponent(`ws://127.0.0.1:${daemonPort}`)}&bridge-token=${encodeURIComponent(token)}`);
    await openReviews(page);
    await page.getByRole("radio", { name: "All", exact: true }).first().click();
    await page.locator('[data-tile="Reviews"]').waitFor();
    await page.waitForFunction(() => document.querySelector('[data-tile="Reviews"]')?.textContent?.includes("4"));

    // The numbers the store holds.
    assert.match(await tileText(page, "Reviews"), /Reviews 4 .*2 posted · 1 waiting · 1 failed/);
    assert.match(await tileText(page, "Findings"), /Findings 5 .*1 both reviewers flagged/);
    assert.match(await tileText(page, "Kept"), /Kept 67% 2 of 3 decided/);
    assert.match(await tileText(page, "Fixed later"), /Fixed later 1/);
    assert.match(await tileText(page, "Spend"), /\$2\.69/);
    const repositories = await page.locator('select[aria-label="Repository"] option').allInnerTexts();
    assert.deepEqual(repositories, ["All repositories", "fixture/project", "octo/web"]);
    assert.match(await page.locator('[data-reviewer="codex"]').innerText(), /Not reported/);
    await page.waitForTimeout(400);
    await page.screenshot({ path: path.join(out, `${scheme}-1-dashboard.png`), fullPage: true });

    // One repository: only its reviews.
    await page.locator('select[aria-label="Repository"]').selectOption("octo/web");
    await page.waitForFunction(() => document.querySelector('[data-tile="Reviews"]')?.textContent?.match(/Reviews\s*2/));
    assert.deepEqual(await page.locator("[data-job-row]").evaluateAll(rows => rows.map(row => row.getAttribute("data-job-row"))), ["failed", "posted-web"]);
    await page.locator('select[aria-label="Repository"]').selectOption("");

    // Findings, searchable.
    await page.getByRole("radio", { name: /^Findings/ }).click();
    await page.getByLabel("Search findings").fill("expires");
    assert.deepEqual(await page.locator("[data-finding-row]").evaluateAll(rows => rows.map(row => row.getAttribute("data-finding-row"))), ["posted-web-f0"]);
    await page.screenshot({ path: path.join(out, `${scheme}-2-findings.png`), fullPage: true });
    await page.getByLabel("Search findings").fill("");
    await page.getByRole("radio", { name: /^Reviews \d/ }).click();

    if (scheme === "light") {
      // Triage from the dashboard: keep the agreed finding, post through gh.
      await page.locator('[aria-label="Needs you"] [data-open-job="draft"]').click();
      const sheet = page.locator('[data-review-detail="draft"]');
      await sheet.waitFor();
      assert.match(await sheet.innerText(), /Changes requested/);
      assert.match(await sheet.innerText(), /Looks mostly fine/);
      const agreed = sheet.locator('[data-finding="draft-f0"]');
      await agreed.waitFor();
      assert.match(await agreed.innerText(), /Both reviewers/);
      await page.screenshot({ path: path.join(out, `${scheme}-3-triage.png`) });
      await agreed.focus();
      await page.keyboard.press("k");
      await page.waitForFunction(() => document.querySelector('[data-finding="draft-f0"]')?.getAttribute("data-decision") === "kept");
      await sheet.getByRole("button", { name: "Post 1 finding" }).click();
      await page.waitForFunction(() => document.querySelector('[data-review-detail="draft"]')?.textContent?.includes("Posted review"), null, { timeout: 60_000 });
      const posted = JSON.parse(fs.readFileSync(path.join(gh, "review-posted-1.json"), "utf8"));
      assert.equal(posted.event, "COMMENT");
      assert.equal(posted.comments.length, 1);
      await page.keyboard.press("Escape");
      // Live: the dashboard moved the job out of Needs you on its own.
      await page.waitForFunction(() => !document.querySelector('[aria-label="Needs you"] [data-open-job="draft"]'), null, { timeout: 15_000 });
      assert.match(await tileText(page, "Reviews"), /3 posted · 1 failed/);
      results.push({ scheme, posted: posted.comments.length });
    } else {
      results.push({ scheme });
    }
    assert.deepEqual(errors, [], "no page errors");
    await page.close();
  }
} finally {
  await browser.close();
  stopAll();
}
console.log(JSON.stringify({ out, scratch, results }, null, 2));

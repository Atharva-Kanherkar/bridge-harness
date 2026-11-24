#!/usr/bin/env node
// The Reviews section end to end, in the browser preview linked to a real
// `bridged --listen` (testing/feat-review-pane.md, E2E). Fully offline: GitHub
// is a local bare repository holding PR #103 plus the shared fake `gh`
// (testing/fixtures/github), and the reviewer is Bridge's real Claude sidecar
// against the fake SDK (testing/fixtures/workflow-e2e). Everything lives in a
// scratch folder with its own HOME.
//
//   BRIDGED_BIN=<path to bridged> BRIDGE_PLAYWRIGHT_MODULE=<playwright> \
//     node scripts/e2e-review-pane.mjs [--out <screenshot dir>]
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
const out = outIndex > 0 ? path.resolve(process.argv[outIndex + 1]) : fs.mkdtempSync(path.join(os.tmpdir(), "review-pane-shots-"));
fs.mkdirSync(out, { recursive: true });

const scratch = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), "review-pane-e2e-")));
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
let first = daemon();
await waitFor(() => fs.existsSync(path.join(data, "daemon.token")) && fs.existsSync(path.join(scratch, "d.sock")), "the first daemon start");
first.kill("SIGTERM");
await new Promise(resolve => first.on("exit", resolve));
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

const browser = await chromium.launch({ headless: true });
const results = [];
try {
  for (const scheme of ["light", "dark"]) {
    const page = await browser.newPage({ viewport: { width: 1440, height: 900 }, colorScheme: scheme });
    const errors = [];
    page.on("pageerror", error => errors.push(String(error)));
    await page.goto(`${origin}/#bridge-daemon=${encodeURIComponent(`ws://127.0.0.1:${daemonPort}`)}&bridge-token=${encodeURIComponent(token)}`);
    // Open the seeded chat, then its GitHub pane.
    await page.getByText("New chat", { exact: true }).first().click({ timeout: 30_000 });
    await page.getByRole("button", { name: "GitHub", exact: true }).first().click({ timeout: 30_000 });
    const pane = page.locator("[data-gh-pane-root]");
    await pane.getByText("Changes requested", { exact: true }).first().click({ timeout: 30_000 });
    const shot = async name => pane.screenshot({ path: path.join(out, `${scheme}-${name}.png`) });

    // Review with Claude: a job appears in Reviews and runs on its own. Jobs
    // an earlier pass left load first, so the new one can be told apart.
    if (results.length) await pane.locator("[data-review-job]").first().waitFor({ timeout: 30_000 });
    const before = await pane.locator("[data-review-job]").evaluateAll(rows => rows.map(row => row.getAttribute("data-review-job")));
    await pane.getByRole("button", { name: "Review", exact: true }).click();
    await pane.getByRole("menuitem", { name: "Claude" }).click();
    const reviews = pane.getByRole("region", { name: "Reviews" });
    await reviews.waitFor({ timeout: 30_000 });
    await page.waitForFunction(known => [...document.querySelectorAll("[data-review-job]")].some(row => !known.includes(row.getAttribute("data-review-job"))), before);
    const jobId = await pane.locator("[data-review-job]").evaluateAll((rows, known) => rows.map(row => row.getAttribute("data-review-job")).find(id => !known.includes(id)), before);
    const row = pane.locator(`[data-review-job="${jobId}"]`);
    await shot("1-started");
    const draft = reviews.locator(`[data-review-draft="${jobId}"]`);
    await draft.waitFor({ timeout: 120_000 });
    const finding = draft.locator("[data-finding]").first();
    await finding.waitFor();
    assert.match(await finding.innerText(), /README\.md:2/);
    assert.match(await row.innerText(), /Draft ready/);
    await shot("2-drafted");

    // Keep it from the keyboard, then post.
    await finding.focus();
    await page.keyboard.press("k");
    await page.waitForFunction(id => document.querySelector(`[data-review-draft="${id}"] [data-finding]`)?.getAttribute("data-decision") === "kept", jobId);
    await shot("3-kept");
    await draft.getByRole("button", { name: "Post 1 finding" }).click();
    const link = row.getByRole("link", { name: /View on GitHub/ });
    await link.waitFor({ timeout: 60_000 });
    assert.equal(await link.getAttribute("href"), "https://github.com/fixture/project/pull/103#pullrequestreview-555");
    assert.match(await row.innerText(), /Posted/);
    await shot("4-posted");

    const posted = JSON.parse(fs.readFileSync(path.join(gh, `review-posted-${scheme === "light" ? 1 : 2}.json`), "utf8"));
    assert.equal(posted.event, "COMMENT");
    assert.equal(posted.comments.length, 1);
    assert.equal(posted.comments[0].path, "README.md");
    results.push({ scheme, posted: { event: posted.event, comments: posted.comments.length }, errors: [...errors] });
    // The whole window: the review's toasts reach the person wherever they are.
    await page.screenshot({ path: path.join(out, `${scheme}-4-window.png`) });
    if (scheme === "light") {
      // Settings → Background tasks: the Review jobs group, every behaviour a visible control.
      await page.getByText("Settings", { exact: true }).first().click();
      // Background tasks sits in the rail's Advanced group, folded at first.
      const advanced = page.getByRole("button", { name: "Advanced" }).first();
      await advanced.waitFor({ timeout: 15_000 });
      if (!(await page.getByText("Background tasks", { exact: true }).first().isVisible())) await advanced.click();
      await page.getByText("Background tasks", { exact: true }).first().click({ timeout: 15_000 });
      const group = page.locator('[data-settings-anchor="review-jobs"], #review-jobs').first();
      await page.getByText("Review jobs", { exact: true }).first().scrollIntoViewIfNeeded();
      await page.waitForTimeout(500);
      await (await group.count() ? group : page.locator("[data-reviewer-settings]")).screenshot({ path: path.join(out, "light-5-settings.png") });
    }
    await page.close();
    // The second pass reviews afresh: the first job is posted (terminal).
  }
} finally {
  await browser.close();
  stopAll();
}
console.log(JSON.stringify({ out, scratch, results }, null, 2));

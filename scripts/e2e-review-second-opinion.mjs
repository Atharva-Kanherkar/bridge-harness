#!/usr/bin/env node
// A second opinion's draft end to end, in the browser preview linked to a real
// `bridged --listen` (testing/feat-review-second-opinion.md). There is no
// offline stand-in for a second harness, so the store is seeded with what the
// runner leaves after two passes and a merge: a drafted job on Claude and
// Codex, one finding both flagged (agreed_by 2, both pass ids) and one each.
// The real pane renders it, a person keeps the agreed finding, and the real
// post path sends it to the shared fake `gh`. Everything lives in a scratch
// folder with its own HOME.
//
//   BRIDGED_BIN=<path to bridged> BRIDGE_PLAYWRIGHT_MODULE=<playwright> \
//     node scripts/e2e-review-second-opinion.mjs [--out <screenshot dir>]
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
const out = outIndex > 0 ? path.resolve(process.argv[outIndex + 1]) : fs.mkdtempSync(path.join(os.tmpdir(), "second-opinion-shots-"));
fs.mkdirSync(out, { recursive: true });

const scratch = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), "second-opinion-e2e-")));
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

// The first start creates the store; then a project, its workspace, a chat,
// and per colour scheme one drafted second-opinion job as the runner leaves it.
let first = daemon();
await waitFor(() => fs.existsSync(path.join(data, "daemon.token")) && fs.existsSync(path.join(scratch, "d.sock")), "the first daemon start");
first.kill("SIGTERM");
await new Promise(resolve => first.on("exit", resolve));
const job = (scheme, minutes) => {
  const id = `job-${scheme}`;
  const at = new Date(Date.now() - minutes * 60_000).toISOString();
  const summary = "**Claude:** `get` panics at the end of the slice; `Sum_All` is not snake case.\n\n**Codex:** One real problem: the bound check lets `index == len` through.";
  return `
  INSERT INTO review_jobs(id,workspace_id,repo_host,repo_owner,repo_name,pr_number,head_sha,trigger_kind,post_mode,harnesses,dedupe_key,publication_key,status,summary,attempts,created_at,started_at,finished_at)
    VALUES('${id}','w','github.com','fixture','project',103,'${prHead}','manual','draft','["claude","codex"]','dedupe-${scheme}','${id}','drafted','${summary.replace(/'/g, "''")}',1,'${at}','${at}','${at}');
  INSERT INTO review_passes(id,job_id,slot,attempt,harness,status,tokens_in,tokens_out,cost_microusd,created_at,started_at,finished_at)
    VALUES('${id}-claude','${id}',0,1,'claude','done',1200,300,41000,'${at}','${at}','${at}'),
          ('${id}-codex','${id}',1,1,'codex','done',90000,400,0,'${at}','${at}','${at}');
  INSERT INTO review_findings(id,job_id,pass_id,path,line,side,severity,title,body,confidence,fingerprint,agreed_by,pass_ids,outside_diff,created_at)
    VALUES('${id}-agreed','${id}','${id}-codex','README.md',2,'RIGHT','critical','Reject indices equal to the slice length',
           'The \`<=\` check allows \`index == items.len()\`, so \`items[index]\` panics instead of returning the documented \`None\`. Use \`<\` or \`items.get(index).copied()\`.',
           0.9,'fp-agreed',2,'["${id}-codex","${id}-claude"]',0,'${at}'),
          ('${id}-naming','${id}','${id}-claude','README.md',2,'RIGHT','minor','\`Sum_All\` is not snake case',
           'Rust functions are snake_case, so this should be \`sum_all\`; rustc warns with \`non_snake_case\`.',
           0.7,'fp-naming',1,NULL,0,'${at}'),
          ('${id}-tests','${id}','${id}-codex','src/lib.rs',40,'RIGHT','nit','Add regression tests for the boundary',
           'Cover an empty slice and \`index == len\`.',0.5,'fp-tests',1,NULL,1,'${at}');`;
};
execFileSync("sqlite3", [path.join(data, "bridge.db"), `
  INSERT INTO projects(id,name,path,created_at) VALUES('p','Demo','${repo}','now');
  INSERT INTO workspaces(id,project_id,city,title,branch,path,status,created_at) VALUES('w','p','Oslo','Demo','main','${repo}','ready','now');
  INSERT INTO sessions(id,workspace_id,harness,label,status,kind) VALUES('chat','w','claude','Chat','ready','direct');
  ${job("light", 3)}
  ${job("dark", 2)}`]);

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
  for (const [index, scheme] of ["light", "dark"].entries()) {
    const page = await browser.newPage({ viewport: { width: 1440, height: 900 }, colorScheme: scheme });
    const errors = [];
    page.on("pageerror", error => errors.push(String(error)));
    await page.goto(`${origin}/#bridge-daemon=${encodeURIComponent(`ws://127.0.0.1:${daemonPort}`)}&bridge-token=${encodeURIComponent(token)}`);
    await page.getByText("New chat", { exact: true }).first().click({ timeout: 30_000 });
    await page.getByRole("button", { name: "GitHub", exact: true }).first().click({ timeout: 30_000 });
    const pane = page.locator("[data-gh-pane-root]");
    await pane.getByText("Changes requested", { exact: true }).first().click({ timeout: 30_000 });
    const jobId = `job-${scheme}`;
    const reviews = pane.getByRole("region", { name: "Reviews" });
    await reviews.waitFor({ timeout: 30_000 });
    const row = pane.locator(`[data-review-job="${jobId}"]`);
    await row.waitFor({ timeout: 30_000 });
    const draft = reviews.locator(`[data-review-draft="${jobId}"]`);
    if (!(await draft.count())) await row.click();
    await draft.waitFor({ timeout: 30_000 });
    const shot = async name => {
      await draft.scrollIntoViewIfNeeded();
      await pane.screenshot({ path: path.join(out, `${scheme}-${name}.png`) });
    };

    // The finding both reviewers flagged comes first, badged and naming both.
    const findings = draft.locator("[data-finding]");
    const agreed = draft.locator(`[data-finding="${jobId}-agreed"]`);
    assert.equal(await findings.first().getAttribute("data-finding"), `${jobId}-agreed`, "the agreed finding ranks first");
    const agreedText = await agreed.innerText();
    assert.match(agreedText, /Both reviewers/);
    assert.match(agreedText, /Codex and Claude/);
    const single = await draft.locator(`[data-finding="${jobId}-naming"]`).innerText();
    assert.doesNotMatch(single, /Both reviewers|Codex/);
    assert.match(single, /Claude/);
    await shot("1-draft");

    // Keep it and post: the comment names both reviewers.
    await agreed.focus();
    await page.keyboard.press("k");
    await page.waitForFunction(id => document.querySelector(`[data-finding="${id}-agreed"]`)?.getAttribute("data-decision") === "kept", jobId);
    await draft.getByRole("button", { name: "Post 1 finding" }).click();
    await row.getByRole("link", { name: /View on GitHub/ }).waitFor({ timeout: 60_000 });
    const posted = JSON.parse(fs.readFileSync(path.join(gh, `review-posted-${index + 1}.json`), "utf8"));
    assert.equal(posted.event, "COMMENT");
    assert.equal(posted.comments.length, 1, "one comment for the one finding both flagged");
    assert.match(posted.comments[0].body, /Bridge review · codex, claude/);
    assert.match(posted.body, /\*\*Claude:\*\*[\s\S]*\*\*Codex:\*\*/, "each reviewer's summary, labelled");
    await pane.locator(`[data-review-job="${jobId}"]`).scrollIntoViewIfNeeded();
    await pane.screenshot({ path: path.join(out, `${scheme}-2-posted.png`) });
    results.push({ scheme, comment: posted.comments[0].body.split("\n").pop(), errors: [...errors] });
    assert.deepEqual(errors, [], "no page errors");
    await page.close();
  }
} finally {
  await browser.close();
  stopAll();
}
console.log(JSON.stringify({ out, scratch, results }, null, 2));

import assert from "node:assert/strict";
import test from "node:test";
import { BUILD_CASES, ROUTING_ASKS } from "../eval-personal-apps/cases.mjs";
import { buildOutcome, classifyRouting, markdownReport, routingSummary, scoreBuild, toolCalls } from "../eval-personal-apps/score.mjs";

const started = (id, server, tool, args) => ({ kind: "tool.started", title: tool, data: { id, server, tool, arguments: args, status: "inProgress" } });
const completed = (id, server, tool, args, extra = {}) => ({ kind: "tool.completed", title: tool, data: { id, server, tool, arguments: args, status: "completed", ...extra } });
const draftResult = { structuredContent: { draftId: "d1", appId: "docs-search", contentHash: "sha256:x" } };

test("no case or ask touches Slack, and every routing ask names a real case or none", () => {
  const text = JSON.stringify({ BUILD_CASES, ROUTING_ASKS }).toLowerCase();
  assert.ok(!text.includes("slack"));
  const ids = new Set(BUILD_CASES.map(item => item.id));
  for (const ask of ROUTING_ASKS) assert.ok(ask.expect === null || ids.has(ask.expect), ask.id);
  assert.ok(ROUTING_ASKS.filter(ask => ask.expect === null).length >= 8, "enough asks that should open nothing");
});

test("tool calls merge their start and completion", () => {
  const calls = toolCalls([started("a", "bridge", "apps_draft", { name: "x" }), completed("a", "bridge", "apps_draft", { name: "x" }, { result: draftResult })]);
  assert.equal(calls.length, 1);
  assert.equal(calls[0].status, "completed");
  assert.deepEqual(calls[0].result, draftResult);
});

test("Claude's tool events read the same as Codex's", () => {
  const events = [
    { kind: "tool.started", itemId: "t1", title: "mcp__bridge__apps_draft", data: { id: "t1", name: "mcp__bridge__apps_draft", input: { server: "docs" } } },
    { kind: "tool.completed", itemId: "t1", title: null, data: { tool_use_id: "t1", is_error: true, content: [{ type: "text", text: "No agent that can run personal apps has docs connected" }] } },
    { kind: "tool.started", itemId: "t2", title: "mcp__bridge__apps_draft", data: { id: "t2", name: "mcp__bridge__apps_draft", input: { server: "openaiDeveloperDocs" } } },
    { kind: "tool.completed", itemId: "t2", title: null, data: { tool_use_id: "t2", is_error: false, content: [{ type: "text", text: JSON.stringify(draftResult.structuredContent) }] } },
  ];
  const [refused, made] = toolCalls(events);
  assert.deepEqual([refused.server, refused.tool, refused.status], ["bridge", "apps_draft", "failed"]);
  assert.match(refused.error, /No agent/);
  assert.deepEqual(made.arguments, { server: "openaiDeveloperDocs" });
  const outcome = buildOutcome(events);
  assert.equal(outcome.drafted, true);
  assert.equal(outcome.draft.appId, "docs-search");
  assert.equal(outcome.attempts, 2);
});

test("a build outcome finds the draft, refusals and direct server calls", () => {
  const events = [
    { type: "session", sessionId: "s1" },
    completed("a", "bridge", "apps_draft", {}, { status: "failed", error: { message: "No agent that can run personal apps has x connected" } }),
    completed("b", "openaiDeveloperDocs", "search_openai_docs", { query: "x" }),
    completed("c", "bridge", "apps_draft", {}, { result: draftResult }),
    { kind: "turn.completed", data: { turn: { items: [{ type: "agentMessage", text: "Done." }] } } },
  ];
  const outcome = buildOutcome(events);
  assert.equal(outcome.drafted, true);
  assert.equal(outcome.draft.appId, "docs-search");
  assert.equal(outcome.attempts, 2);
  assert.match(outcome.refusals[0], /No agent/);
  assert.deepEqual(outcome.directServerCalls, ["openaiDeveloperDocs/search_openai_docs"]);
  assert.equal(outcome.sessionId, "s1");
  assert.equal(outcome.answer, "Done.");
  assert.equal(buildOutcome([{ kind: "turn.completed", data: { turn: { items: [] } } }]).drafted, false, "a chat that built something else drafted nothing");
});

test("build scoring passes a good app and names each failure", () => {
  const testCase = BUILD_CASES.find(item => item.id === "docs-codex");
  const good = {
    outcome: { drafted: true, attempts: 1, draft: { appId: "docs-search" } },
    manifest: { server: "openaiDeveloperDocs", harness: "codex", reads: ["search_openai_docs"], writes: [] },
    liveRead: { tool: "search_openai_docs", ok: true, isError: false, ms: 900 },
    render: { light: { ok: true, luminance: 0.97, networkAttempts: [], cspViolations: [] }, dark: { ok: true, luminance: 0.1, networkAttempts: [], cspViolations: [] } },
    viewCalls: [{ tool: "search_openai_docs", ok: true, isError: false }],
    lock: { ok: true, kind: "locked" },
  };
  const scored = scoreBuild(testCase, good);
  assert.equal(scored.passed, scored.total, JSON.stringify(scored.checks.filter(item => !item.pass)));
  const bad = scoreBuild(testCase, {
    ...good,
    manifest: { ...good.manifest, writes: ["delete_doc"] },
    liveRead: { tool: "search_openai_docs", ok: true, isError: true, ms: 1 },
    render: { light: { ok: true, luminance: 0.1, networkAttempts: ["https://cdn.example/x.js"], cspViolations: [] }, dark: { ok: false, error: "blank", luminance: 0.1, networkAttempts: [], cspViolations: [] } },
    viewCalls: [],
  });
  const failed = bad.checks.filter(item => !item.pass).map(item => item.name);
  assert.deepEqual(failed, ["writes match the ask", "live read with sample arguments", "renders (dark)", "follows Bridge's theme", "the view's own call works", "no network from the view"]);
  assert.equal(scoreBuild(testCase, { outcome: { drafted: false, refusals: [] } }).total, 1, "no draft stops at one failed check");
  const transparent = { ...good, render: { light: { ok: true, luminance: null, textLuminance: 0.1, networkAttempts: [], cspViolations: [] }, dark: { ok: true, luminance: null, textLuminance: 0.95, networkAttempts: [], cspViolations: [] } } };
  assert.ok(scoreBuild(testCase, transparent).checks.find(item => item.name === "follows Bridge's theme").pass, "a transparent view whose text flips follows the theme");
  const stuck = { ...good, render: { light: { ok: true, luminance: null, textLuminance: 0.9, networkAttempts: [], cspViolations: [] }, dark: { ok: true, luminance: null, textLuminance: 0.9, networkAttempts: [], cspViolations: [] } } };
  assert.ok(!scoreBuild(testCase, stuck).checks.find(item => item.name === "follows Bridge's theme").pass, "light text on Bridge's light card does not");
});

test("routing verdicts tell every outcome apart", () => {
  const open = app => [completed("o", "bridge", "apps_open", { app_id: app, prefill: { query: "x" } })];
  assert.equal(classifyRouting(open("docs"), "docs").verdict, "opened-expected");
  assert.deepEqual(classifyRouting(open("docs"), "docs").prefill, { query: "x" });
  assert.equal(classifyRouting(open("memory"), "docs").verdict, "opened-wrong");
  assert.equal(classifyRouting(open("docs-2"), ["docs", "docs-2"]).verdict, "opened-expected", "either app for the server is right");
  assert.equal(classifyRouting(open("docs"), null).verdict, "false-open");
  assert.equal(classifyRouting([], null).verdict, "correct-none");
  assert.equal(classifyRouting([completed("d", "railway", "list_services", {})], "railway-app").verdict, "answered-directly");
  assert.equal(classifyRouting([], "docs").verdict, "missed");
  assert.equal(classifyRouting([completed("n", "bridge", "apps_draft", {})], "docs").verdict, "drafted-instead", "a new draft is not opening the saved app");
  const helpers = [completed("h", "cua_repl", "js", {}), completed("h2", "cua_repl", "js", {})];
  assert.equal(classifyRouting(helpers, "docs", new Set(["railway"])).verdict, "missed", "an agent's own helpers are not the server");
  assert.deepEqual(classifyRouting([completed("d", "railway", "list_services", {}), completed("e", "railway", "list_services", {})], "x", new Set(["railway"])).direct, ["railway/list_services"], "listed once");
});

test("routing totals give recall and precision per agent, and the report renders", () => {
  const rows = [
    { chat: "codex", ask: "a", expect: "docs-codex", expectAppId: "docs", result: { verdict: "opened-expected", opened: ["docs"], direct: [] } },
    { chat: "codex", ask: "b", expect: "docs-codex", expectAppId: "docs", result: { verdict: "missed", opened: [], direct: [] } },
    { chat: "codex", ask: "c", expect: null, expectAppId: null, result: { verdict: "false-open", opened: ["docs"], direct: [] } },
  ];
  const summary = routingSummary(rows).codex;
  assert.equal(summary.recall, 0.5);
  assert.equal(summary.precision, 0.5);
  const report = markdownReport({ builds: [], routing: { rows } });
  assert.match(report, /\| codex \| 3 \| 50% \| 50% \| 0 \| 1 \| 0 \|/);
});

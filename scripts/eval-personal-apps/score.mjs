// Scoring for the personal-app evals. Pure functions over what a run
// recorded (the chat's event stream, Bridge's stored draft, the live read
// and the rendered view), so they are unit-tested without any model.

/** Tool calls in one chat's `bridge exec --json` event stream. Codex and
 *  OpenCode report `{server, tool, arguments, status, result, error}`;
 *  Claude reports `{name: "mcp__<server>__<tool>", input}` and completes
 *  with `{tool_use_id, content, is_error}`. Both become one shape. */
export function toolCalls(events) {
  const calls = new Map();
  for (const event of events) {
    if (event.kind !== "tool.started" && event.kind !== "tool.completed") continue;
    const data = event.data ?? {};
    const id = data.id ?? data.tool_use_id ?? event.itemId ?? `${event.sequence}`;
    const previous = calls.get(id) ?? {};
    const named = /^mcp__(.+?)__(.+)$/.exec(data.tool ?? data.name ?? event.title ?? "");
    // Claude's content is a list of parts or, for a tool error, plain text.
    const parts = typeof data.content === "string" ? [{ type: "text", text: data.content }] : Array.isArray(data.content) ? data.content : null;
    const claudeResult = parts ? { content: parts, isError: Boolean(data.is_error) } : null;
    const claudeError = data.is_error ? (parts?.find(part => part.type === "text")?.text ?? "error") : null;
    calls.set(id, {
      ...previous,
      server: data.server ?? named?.[1] ?? previous.server ?? null,
      tool: named?.[2] ?? data.tool ?? data.name ?? event.title ?? previous.tool ?? null,
      arguments: data.arguments ?? data.input ?? previous.arguments ?? null,
      status: event.kind === "tool.completed" ? (data.is_error ? "failed" : data.status ?? "completed") : previous.status ?? "inProgress",
      error: data.error?.message ?? claudeError ?? previous.error ?? null,
      result: data.result ?? claudeResult ?? previous.result ?? null,
      durationMs: data.durationMs ?? previous.durationMs ?? null,
    });
  }
  return [...calls.values()];
}

const isBridge = call => call.server === "bridge" || call.server === null && /^apps_/.test(call.tool ?? "");

/** The draft a build chat made, if any, and how it got there. */
export function buildOutcome(events) {
  const calls = toolCalls(events);
  const drafts = calls.filter(call => isBridge(call) && call.tool === "apps_draft");
  const made = drafts.filter(call => call.status === "completed" && !call.error).map(call => call.result?.structuredContent ?? parseText(call.result)).filter(Boolean).at(-1) ?? null;
  const final = [...events].reverse().find(event => event.kind === "turn.completed");
  const answer = final?.data?.turn?.items?.filter(item => item.type === "agentMessage").at(-1)?.text ?? null;
  return {
    drafted: Boolean(made?.draftId),
    draft: made,
    attempts: drafts.length,
    refusals: drafts.filter(call => call.error || call.status === "failed").map(call => call.error ?? "failed"),
    directServerCalls: calls.filter(call => !isBridge(call) && call.server).map(call => `${call.server}/${call.tool}`),
    answer,
    sessionId: events.find(event => event.type === "session")?.sessionId ?? events.find(event => event.sessionId)?.sessionId ?? null,
  };
}

function parseText(result) {
  const text = result?.content?.find(part => part.type === "text")?.text;
  try {
    return text ? JSON.parse(text) : null;
  } catch {
    return null;
  }
}

/** Whether an MCP result is an error the server reported. */
export function isToolError(result) {
  return Boolean(result?.isError);
}

/**
 * The build checks for one case. `run` holds:
 * - `outcome`: from `buildOutcome`;
 * - `manifest`: Bridge's stored draft (camelCase, as stored);
 * - `liveRead`: `{tool, ok, error, isError, ms}` for the first read with the sample arguments;
 * - `render`: `{light, dark}` each `{ok, networkAttempts: [...], cspViolations: [...], viewCalls: [...]}`;
 * - `lock`: `{ok, kind, error}`.
 */
export function scoreBuild(testCase, run) {
  const checks = [];
  const check = (name, pass, detail = "") => checks.push({ name, pass: Boolean(pass), detail });
  const { outcome, manifest, liveRead, render, lock } = run;
  check("drafted", outcome?.drafted, outcome?.drafted ? `${outcome.attempts} apps_draft call(s)` : (outcome?.refusals?.join("; ") || "no apps_draft call"));
  if (!outcome?.drafted || !manifest) return finish(testCase, checks, run);
  check("server", sameServer(manifest.server, testCase.server), `${manifest.server} via ${manifest.harness}`);
  check("reads chosen", manifest.reads.length > 0, manifest.reads.join(", "));
  check("writes match the ask", testCase.needsWrites ? manifest.writes.length > 0 : manifest.writes.length === 0, manifest.writes.join(", ") || "none");
  check("live read with sample arguments", liveRead?.ok && !liveRead?.isError, liveRead ? `${liveRead.tool}: ${liveRead.ok ? (liveRead.isError ? "server error" : `ok in ${liveRead.ms} ms`) : liveRead.error}` : "not run");
  for (const theme of ["light", "dark"]) {
    const shot = render?.[theme];
    check(`renders (${theme})`, shot?.ok, shot ? (shot.ok ? "" : shot.error ?? "blank") : "not run");
  }
  // A view follows the theme when its own background is lighter in light
  // than in dark, or, if it paints none (Bridge's card shows through), when
  // its text is darker in light than in dark.
  const light = render?.light?.luminance, dark = render?.dark?.luminance;
  const lightText = render?.light?.textLuminance, darkText = render?.dark?.textLuminance;
  const number = value => typeof value === "number";
  const byBackground = number(light) && number(dark);
  const follows = byBackground ? light - dark > 0.25 : number(lightText) && number(darkText) && darkText - lightText > 0.25;
  check("follows Bridge's theme", follows, byBackground ? `background ${light.toFixed(2)} light, ${dark.toFixed(2)} dark` : number(lightText) && number(darkText) ? `transparent; text ${lightText.toFixed(2)} light, ${darkText.toFixed(2)} dark` : "no colors read");
  const calls = run.viewCalls ?? [];
  check("the view's own call works", calls.some(item => item.ok && !item.isError), calls.length ? calls.map(item => `${item.tool}${item.preview ? " (preview)" : ""}: ${item.ok ? (item.isError ? "server error" : "ok") : item.error}`).join("; ") : "no call after typing and pressing its first button");
  const network = ["light", "dark"].flatMap(theme => render?.[theme]?.networkAttempts ?? []);
  const csp = ["light", "dark"].flatMap(theme => render?.[theme]?.cspViolations ?? []);
  check("no network from the view", network.length === 0 && csp.length === 0, [...network, ...csp].slice(0, 3).join("; ") || "0 attempts");
  check("locks", lock?.ok, lock?.ok ? lock.kind : lock?.error ?? "not run");
  return finish(testCase, checks, run);
}

function sameServer(a, b) {
  const wire = name => String(name ?? "").toLowerCase().replace(/[^a-z0-9]+/g, "");
  return wire(a) === wire(b);
}

function finish(testCase, checks, run) {
  const passed = checks.filter(item => item.pass).length;
  return { id: testCase.id, chat: testCase.chat, server: testCase.server, checks, passed, total: checks.length, score: checks.length ? passed / checks.length : 0, appId: run.outcome?.draft?.appId ?? null };
}

/**
 * What a routing chat did with one ask:
 * - `opened-expected`: `apps_open` with the expected app;
 * - `opened-wrong`: `apps_open` with another app;
 * - `false-open`: an app opened for an ask that wanted none;
 * - `correct-none`: nothing opened, and nothing should have;
 * - `drafted-instead`: a new app was drafted instead of opening the saved one;
 * - `answered-directly`: the server's own tools answered instead of the app;
 * - `missed`: nothing opened that should have.
 */
export function classifyRouting(events, expected, appServers = null) {
  // Any of several apps can be right: two chats may each have built an app
  // for the same server.
  const acceptable = expected === null ? null : new Set([expected].flat());
  const calls = toolCalls(events);
  const opens = calls.filter(call => isBridge(call) && call.tool === "apps_open");
  const opened = opens.map(call => call.arguments?.app_id).filter(Boolean);
  // Only the app servers count: an agent's own helpers (Codex's computer-use
  // REPL, a browser) are not the server answering instead of its app.
  const fold = name => String(name).toLowerCase().replace(/[^a-z0-9]+/g, "");
  const servers = appServers && new Set([...appServers].map(fold));
  const direct = [...new Set(calls.filter(call => !isBridge(call) && call.server && (!servers || servers.has(fold(call.server)))).map(call => `${call.server}/${call.tool}`))];
  const prefill = opens.find(call => acceptable?.has(call.arguments?.app_id))?.arguments?.prefill ?? null;
  let verdict;
  if (acceptable === null) verdict = opened.length ? "false-open" : "correct-none";
  else if (opened.some(app => acceptable.has(app))) verdict = "opened-expected";
  else if (opened.length) verdict = "opened-wrong";
  else if (calls.some(call => isBridge(call) && call.tool === "apps_draft")) verdict = "drafted-instead";
  else if (direct.length) verdict = "answered-directly";
  else verdict = "missed";
  return { verdict, opened, direct, prefill, openErrors: opens.map(call => call.error).filter(Boolean) };
}

/** Routing totals: precision and recall of opening, per agent. */
export function routingSummary(rows) {
  const byChat = {};
  for (const row of rows) {
    const total = byChat[row.chat] ??= { asks: 0, expectedOpens: 0, opened: 0, wrong: 0, falseOpens: 0, correctNone: 0, direct: 0, missed: 0 };
    total.asks += 1;
    if (row.expect) total.expectedOpens += 1;
    const v = row.result.verdict;
    if (v === "opened-expected") total.opened += 1;
    if (v === "opened-wrong") total.wrong += 1;
    if (v === "false-open") total.falseOpens += 1;
    if (v === "correct-none") total.correctNone += 1;
    if (v === "answered-directly") total.direct += 1;
    if (v === "drafted-instead") total.drafted = (total.drafted ?? 0) + 1;
    if (v === "missed") total.missed += 1;
  }
  for (const total of Object.values(byChat)) {
    const opensMade = total.opened + total.wrong + total.falseOpens;
    total.recall = total.expectedOpens ? total.opened / total.expectedOpens : null;
    total.precision = opensMade ? total.opened / opensMade : null;
  }
  return byChat;
}

/** A Markdown table for the PR notes. */
export function markdownReport({ builds, routing }) {
  const lines = ["## Build eval", "", "| Case | Chat | Server | Score | Failed checks |", "|---|---|---|---|---|"];
  for (const build of builds) {
    const failed = build.checks.filter(item => !item.pass).map(item => `${item.name}${item.detail ? ` (${item.detail})` : ""}`).join("; ") || "none";
    lines.push(`| ${build.id} | ${build.chat} | ${build.server} | ${build.passed}/${build.total} | ${failed.replace(/\|/g, "\\|")} |`);
  }
  if (routing?.rows?.length) {
    lines.push("", "## Routing eval", "", "| Chat | Asks | Recall | Precision | Wrong app | False opens | Drafted instead | Answered directly | Missed |", "|---|---|---|---|---|---|---|---|---|");
    for (const [chat, total] of Object.entries(routingSummary(routing.rows))) {
      const pct = value => value === null ? "n/a" : `${Math.round(value * 100)}%`;
      lines.push(`| ${chat} | ${total.asks} | ${pct(total.recall)} | ${pct(total.precision)} | ${total.wrong} | ${total.falseOpens} | ${total.drafted ?? 0} | ${total.direct} | ${total.missed} |`);
    }
    lines.push("", "| Ask | Chat | Expected | Verdict | Opened | Direct tools |", "|---|---|---|---|---|---|");
    for (const row of routing.rows) {
      lines.push(`| ${row.ask.replace(/\|/g, "\\|")} | ${row.chat} | ${[row.expectAppId ?? "none"].flat().join(" or ")} | ${row.result.verdict} | ${row.result.opened.join(", ") || "-"} | ${row.result.direct.join(", ") || "-"} |`);
    }
  }
  return lines.join("\n") + "\n";
}

import { test } from "node:test";
import assert from "node:assert/strict";
import { probeUsage, usageFrame } from "../usage.mjs";

const usageMethod = "usage_EXPERIMENTAL_MAY_CHANGE_DO_NOT_RELY_ON_THIS_API_YET";
const response = {
  rate_limits_available: true, subscription_type: "max",
  rate_limits: {
    five_hour: { utilization: 0, resets_at: "2026-09-19T14:20:00Z" },
    seven_day: { utilization: null, resets_at: null },
    model_scoped: [{ display_name: "Fable", utilization: 31, resets_at: null }],
  },
};

test("usage probe sends no model turn and disables hooks, tools, persistence, MCP and transcript scans", async () => {
  let captured;
  let closed = false;
  let promptResult;
  const frame = await probeUsage(({ options, prompt }) => {
    captured = options;
    promptResult = prompt.next();
    return {
      initializationResult: async () => ({ account: { email: "account@example.test" } }),
      [usageMethod]: async (opts) => { assert.deepEqual(opts, { skipBehaviors: true }); return response; },
      close: () => { closed = true; },
    };
  }, { cwd: "/private/probe", executablePath: "/bin/claude" });
  assert.deepEqual(await promptResult, { done: true, value: undefined });
  assert.equal(frame.type, "claude_usage");
  assert.equal(frame.account.email, "account@example.test");
  assert.equal(frame.rateLimits.five_hour.utilization, 0);
  assert.equal(frame.rateLimits.seven_day.utilization, null);
  assert.equal(frame.rateLimits.model_scoped[0].utilization, 31);
  assert.equal(captured.persistSession, false);
  assert.deepEqual(captured.settings, { disableAllHooks: true });
  for (const key of ["tools", "allowedTools", "plugins", "settingSources"]) assert.deepEqual(captured[key], []);
  assert.equal(captured.strictMcpConfig, true);
  assert.deepEqual(captured.mcpServers, {});
  assert.equal(captured.env.ENABLE_CLAUDEAI_MCP_SERVERS, "false");
  assert.equal(captured.env.CLAUDE_CODE_AUTO_CONNECT_IDE, "0");
  assert.equal(captured.abortController.signal.aborted, true);
  assert.equal(closed, true);
});

test("only public account fields and quota windows cross the sidecar boundary", () => {
  const frame = usageFrame({ account: { email: "test", accessToken: "SECRET" } }, {
    ...response, session: { transcript: "SECRET" }, behaviors: { private: "SECRET" },
  });
  assert.equal(JSON.stringify(frame).includes("SECRET"), false);
  assert.equal(usageFrame({}, { rate_limits_available: false, rate_limits: null }).rateLimitsAvailable, false);
  for (const invalid of [undefined, {}, { rate_limits_available: true, rate_limits: null },
    { rate_limits_available: true, rate_limits: { five_hour: { utilization: "0" } } }]) {
    assert.throws(() => usageFrame({}, invalid));
  }
});

test("old SDK and failures return stable sanitized errors and always close", async () => {
  for (const phase of ["unsupported", "initialization_failed", "usage_failed"]) {
    let closed = false;
    const frame = await probeUsage(() => ({
      initializationResult: async () => {
        if (phase === "initialization_failed") throw new Error("secret credential");
        return {};
      },
      ...(phase === "unsupported" ? {} : { [usageMethod]: async () => { throw new Error("secret credential"); } }),
      close: () => { closed = true; },
    }));
    assert.deepEqual(frame, { type: "claude_usage_error", code: phase });
    assert.equal(closed, true);
  }
});

test("both initialization and usage have deadlines and abort the SDK", async () => {
  for (const hungStage of ["init", "usage"]) {
    let options;
    let closed = false;
    const frame = await probeUsage((input) => {
      options = input.options;
      return {
        initializationResult: () => hungStage === "init" ? new Promise(() => {}) : Promise.resolve({}),
        [usageMethod]: () => new Promise(() => {}),
        close: () => { closed = true; },
      };
    }, {}, { initializationMs: 10, usageMs: 10 });
    assert.deepEqual(frame, { type: "claude_usage_error", code: "timeout" });
    assert.equal(options.abortController.signal.aborted, true);
    assert.equal(closed, true);
  }
});

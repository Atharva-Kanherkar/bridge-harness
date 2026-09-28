// Like T3 Code's capability probe, ask Claude to own authentication and usage.
// No prompt is yielded; this never starts a model turn or imports transcripts.
const method = "usage_EXPERIMENTAL_MAY_CHANGE_DO_NOT_RELY_ON_THIS_API_YET";

const publicText = (value) => typeof value === "string" && value.trim()
  ? value.replace(/[\x00-\x1f\x7f]/g, "").trim().slice(0, 200) : undefined;
const window = (value) => {
  if (!value || typeof value !== "object" || Array.isArray(value)) throw new Error("malformed");
  if (value.utilization !== null && !(Number.isFinite(value.utilization) && value.utilization >= 0)) {
    throw new Error("malformed");
  }
  return { utilization: value.utilization, resets_at: publicText(value.resets_at) ?? null };
};

export function usageFrame(init, usage) {
  if (typeof usage?.rate_limits_available !== "boolean") throw new Error("malformed");
  const limits = {};
  if (usage.rate_limits_available) {
    if (!usage.rate_limits || typeof usage.rate_limits !== "object" || Array.isArray(usage.rate_limits)) {
      throw new Error("malformed");
    }
    for (const key of ["five_hour", "seven_day", "seven_day_opus", "seven_day_sonnet", "seven_day_oauth_apps"]) {
      if (usage.rate_limits[key] != null) limits[key] = window(usage.rate_limits[key]);
    }
    if (usage.rate_limits.model_scoped != null) {
      if (!Array.isArray(usage.rate_limits.model_scoped)) throw new Error("malformed");
      limits.model_scoped = usage.rate_limits.model_scoped.slice(0, 32).map((item) => {
        const display_name = publicText(item?.display_name);
        if (!display_name) throw new Error("malformed");
        return { display_name, ...window(item) };
      });
    }
  }
  return {
    type: "claude_usage",
    account: {
      email: publicText(init?.account?.email),
      subscriptionType: publicText(init?.account?.subscriptionType ?? usage.subscription_type),
    },
    rateLimitsAvailable: usage.rate_limits_available,
    rateLimits: limits,
  };
}

export async function probeUsage(query, config = {}, deadlines = {}) {
  const abort = new AbortController();
  let run;
  let stage = "initialization_failed";
  const within = async (operation, milliseconds) => {
    let timer;
    try {
      return await Promise.race([
        operation(),
        new Promise((_, reject) => { timer = setTimeout(() => reject(new Error("timeout")), milliseconds); }),
      ]);
    } finally { clearTimeout(timer); }
  };
  try {
    run = query({
      prompt: (async function* () {
        if (!abort.signal.aborted) await new Promise((resolve) => abort.signal.addEventListener("abort", resolve, { once: true }));
      })(),
      options: {
        abortController: abort, persistSession: false,
        ...(config.executablePath ? { pathToClaudeCodeExecutable: config.executablePath } : {}),
        ...(config.cwd ? { cwd: config.cwd } : {}),
        settingSources: [], settings: { disableAllHooks: true },
        allowedTools: [], tools: [], plugins: [], mcpServers: {}, strictMcpConfig: true,
        env: { ...process.env, ENABLE_CLAUDEAI_MCP_SERVERS: "false", FORCE_CODE_TERMINAL: undefined,
          CLAUDE_CODE_AUTO_CONNECT_IDE: "0", CLAUDE_CODE_IDE_SKIP_AUTO_INSTALL: "1", DISABLE_AUTOUPDATER: "1" },
        stderr: () => {},
      },
    });
    if (typeof run[method] !== "function") return { type: "claude_usage_error", code: "unsupported" };
    const init = await within(() => run.initializationResult(), deadlines.initializationMs ?? 15_000);
    stage = "usage_failed";
    const usage = await within(() => run[method]({ skipBehaviors: true }), deadlines.usageMs ?? 15_000);
    stage = "malformed";
    return usageFrame(init, usage);
  } catch (error) {
    // Never pass SDK stderr, exceptions, tokens, or conversation data to Rust.
    return { type: "claude_usage_error", code: error?.message === "timeout" ? "timeout" : stage };
  } finally {
    abort.abort();
    try { run?.close(); } catch { /* Rust also terminates the entire process group. */ }
  }
}

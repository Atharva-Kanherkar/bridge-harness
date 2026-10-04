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

// The live window, as Claude Code itself measures it (the data behind
// `/context`). Read once after each turn so Bridge can show what fills the
// window instead of guessing. Names are tool, server and skill identifiers;
// no message text crosses this boundary.
const CONTEXT_KINDS = new Set(["used", "free", "buffer", "deferred"]);
const tokenCount = (value) => Number.isFinite(value) && value >= 0 ? Math.round(value) : 0;
const contextName = (value) => typeof value === "string" && value.trim()
  ? value.replace(/[\x00-\x1f\x7f]/g, "").trim().slice(0, 80) : undefined;
const bySize = (left, right) => right.tokens - left.tokens;

export function contextUsageFrame(usage) {
  if (!usage || typeof usage !== "object") return null;
  const used = usage.totalTokens;
  const window = usage.maxTokens;
  if (!Number.isFinite(used) || used < 0 || !Number.isFinite(window) || window <= 0) return null;
  const categories = (Array.isArray(usage.categories) ? usage.categories : [])
    .map((category) => ({
      name: contextName(category?.name),
      tokens: tokenCount(category?.tokens),
      kind: CONTEXT_KINDS.has(category?.kind) ? category.kind : category?.isDeferred ? "deferred" : "used",
    }))
    .filter((category) => category.name)
    .sort(bySize)
    .slice(0, 16);
  const servers = new Map();
  for (const tool of Array.isArray(usage.mcpTools) ? usage.mcpTools : []) {
    const name = contextName(tool?.serverName);
    if (!name) continue;
    // With tool search on, Claude Code sends an MCP tool's schema only once
    // the model has looked it up; until then the tool costs its name. The SDK
    // still reports every schema's size, so only a loaded tool is a per-turn
    // cost. `isLoaded` is absent when tool search is off, and then every tool
    // is loaded.
    const server = servers.get(name) ?? { name, tokens: 0, tools: 0, loaded: 0, deferredTokens: 0 };
    const tokens = tokenCount(tool?.tokens);
    server.tools += 1;
    if (tool?.isLoaded === false) {
      server.deferredTokens += tokens;
    } else {
      server.tokens += tokens;
      server.loaded += 1;
    }
    servers.set(name, server);
  }
  const memoryFiles = Array.isArray(usage.memoryFiles) ? usage.memoryFiles : [];
  const breakdown = usage.messageBreakdown;
  return {
    type: "context_usage",
    model: contextName(usage.model) ?? null,
    usedTokens: tokenCount(used),
    windowTokens: tokenCount(window),
    autoCompactTokens: Number.isFinite(usage.autoCompactThreshold) && usage.autoCompactThreshold > 0
      ? Math.round(usage.autoCompactThreshold) : null,
    autoCompactEnabled: usage.isAutoCompactEnabled === true,
    categories,
    mcpServers: [...servers.values()].sort(bySize).slice(0, 16),
    memoryFiles: {
      count: memoryFiles.length,
      tokens: memoryFiles.reduce((sum, file) => sum + tokenCount(file?.tokens), 0),
    },
    skills: usage.skills && typeof usage.skills === "object"
      ? { included: tokenCount(usage.skills.includedSkills), total: tokenCount(usage.skills.totalSkills), tokens: tokenCount(usage.skills.tokens) }
      : null,
    agentsTokens: (Array.isArray(usage.agents) ? usage.agents : []).reduce((sum, agent) => sum + tokenCount(agent?.tokens), 0),
    messages: breakdown && typeof breakdown === "object" ? {
      toolCalls: tokenCount(breakdown.toolCallTokens),
      toolResults: tokenCount(breakdown.toolResultTokens),
      attachments: tokenCount(breakdown.attachmentTokens),
      assistant: tokenCount(breakdown.assistantMessageTokens),
      user: tokenCount(breakdown.userMessageTokens),
      toolsByType: (Array.isArray(breakdown.toolCallsByType) ? breakdown.toolCallsByType : [])
        .map((tool) => ({ name: contextName(tool?.name), tokens: tokenCount(tool?.callTokens) + tokenCount(tool?.resultTokens) }))
        .filter((tool) => tool.name)
        .sort(bySize)
        .slice(0, 8),
    } : null,
  };
}

/** One bounded read; any failure is silence, never a crashed session. */
export async function readContextUsage(run, timeoutMs = 8_000) {
  if (typeof run?.getContextUsage !== "function") return null;
  let timer;
  try {
    const usage = await Promise.race([
      run.getContextUsage(),
      new Promise((resolve) => { timer = setTimeout(() => resolve(null), timeoutMs); }),
    ]);
    return contextUsageFrame(usage);
  } catch {
    return null;
  } finally {
    clearTimeout(timer);
  }
}

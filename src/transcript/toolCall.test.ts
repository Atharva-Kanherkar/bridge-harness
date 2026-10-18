import { describe, expect, it } from "vitest";
import { readToolCall } from "./toolCall";

describe("fallback tool action labels", () => {
  it("uses a trimmed title for the provider-neutral fallback", () => {
    const tool = readToolCall({ title: "  Resolve project context\n", text: "", status: "inProgress", surface: "activity", data: { type: "unknownAction" } });
    expect(tool).toMatchObject({ doing: "Running: Resolve project context", done: "Finished: Resolve project context", status: "running" });
    expect(tool.target).toBeUndefined();
  });

  it.each([undefined, "", "   "])("keeps an honest fallback when no action is named (%s)", (title) => {
    const tool = readToolCall({ title, text: "", surface: "activity", data: {} });
    expect(tool).toMatchObject({ doing: "Using a tool", done: "Used a tool" });
    expect(tool.target).toBeUndefined();
  });

  it.each(["pending", "inProgress", "streaming", "running"])("waits for identity on an empty anonymous %s call", (status) => {
    const tool = readToolCall({ title: "   ", text: "", status, surface: "activity", data: { kind: "other" } });
    expect(tool.pendingIdentity).toBe(true);
  });

  it.each(["completed", "failed"])("keeps an anonymous %s call inspectable", (status) => {
    const tool = readToolCall({ text: "", status, surface: "activity", data: {} });
    expect(tool.pendingIdentity).not.toBe(true);
  });

  it.each([
    { text: "Output arrived" },
    { title: "Resolve project context" },
    { data: { name: "lookup_context" } },
    { data: { kind: "read" } },
    { data: { type: "commandExecution" } },
  ])("shows meaningful activity as soon as it arrives (%j)", (overrides) => {
    const tool = readToolCall({ text: "", status: "inProgress", surface: "activity", data: {}, ...overrides });
    expect(tool.pendingIdentity).not.toBe(true);
  });

  it.each([
    ["think", "Thinking", "Thought", "brain"],
    ["switch_mode", "Switching mode", "Switched mode", "navigation"],
  ])("classifies ACP %s", (kind, doing, done, glyph) => {
    expect(readToolCall({ title: "Plan", text: "", surface: "activity", data: { kind } }))
      .toMatchObject({ doing, done, glyph, target: "Plan" });
  });

  it("says a Claude tool is being prepared until its arguments have landed", () => {
    // content_block_start: Bash named, no input yet, nothing running.
    const preparing = readToolCall({ title: "Bash", text: "", status: "inProgress", surface: "activity", data: { type: "tool_use", name: "Bash", input: {}, phase: "preparing" } });
    expect(preparing).toMatchObject({ doing: "Preparing", target: "Bash", status: "running" });
    // The snapshot carries the command and the tool actually runs.
    const running = readToolCall({ title: "gh pr create", text: "", status: "inProgress", surface: "activity", data: { type: "tool_use", name: "Bash", input: { command: "gh pr create" }, phase: "running" } });
    expect(running).toMatchObject({ doing: "Opening", target: "a pull request", command: "gh pr create" });
  });

  it("drops a half-written description while the call is still being prepared", () => {
    const preparing = readToolCall({ title: "Bash", text: "", status: "inProgress", surface: "activity", data: { type: "tool_use", name: "Bash", input: { description: "List the" }, phase: "preparing" } });
    expect(preparing.intent).toBeUndefined();
  });
});

describe("shell intent and steps", () => {
  const claudeBash = (input: Record<string, unknown>) =>
    readToolCall({ title: String(input.command ?? "Bash"), text: "", status: "completed", surface: "activity", data: { type: "tool_use", name: "Bash", input } });

  it("takes Claude's own description as the intent, and keeps the command", () => {
    const tool = claudeBash({ command: "ls tools/ 2>/dev/null; df -h", description: "Look around tools/ before reproducing" });
    expect(tool.intent).toBe("Look around tools/ before reproducing");
    expect(tool.command).toBe("ls tools/ 2>/dev/null; df -h");
    expect(tool.effect).toBe("read");
    expect(tool.steps?.map(step => step.done)).toEqual(["Listed", "Checked"]);
  });

  it("never invents an intent", () => {
    expect(claudeBash({ command: "ls" }).intent).toBeUndefined();
    expect(readToolCall({ title: "ls", text: "", surface: "activity", data: { type: "commandExecution", command: "ls" } }).intent).toBeUndefined();
  });

  it("carries steps only for a chain", () => {
    expect(claudeBash({ command: "cat a.ts | head -20" }).steps).toBeUndefined();
    expect(claudeBash({ command: "cat a.ts; cat b.ts" }).steps).toHaveLength(2);
  });

  it("reads a Codex command out of its zsh wrapper", () => {
    const tool = readToolCall({ title: "x", text: "", surface: "activity", data: { type: "commandExecution", command: `/bin/zsh -lc "sed -n '1,40p' service/chat.go"` } });
    expect(tool).toMatchObject({ command: "sed -n '1,40p' service/chat.go", verb: "read", done: "Read", target: "chat.go", effect: "read" });
  });

  it("recognises a wrapped test run as a check", () => {
    const tool = readToolCall({ title: "x", text: "test result: ok. 12 passed; 0 failed", surface: "activity", data: { type: "commandExecution", command: `/bin/zsh -lc "cargo test -p bridge-core"` } });
    expect(tool.check?.kind).toBe("test");
  });

  it("reads a Cursor command out of its backticked title, with no intent", () => {
    const tool = readToolCall({ title: "`git status -sb && git diff --stat`", text: "", surface: "activity", data: { kind: "execute" } });
    expect(tool.command).toBe("git status -sb && git diff --stat");
    expect(tool.intent).toBeUndefined();
    expect(tool.glyph).toBe("git");
  });

  it("treats an ACP title that differs from the command as intent", () => {
    const tool = readToolCall({ title: "Run the auth tests", text: "", surface: "activity", data: { kind: "execute", command: "bun test src/auth" } });
    expect(tool.intent).toBe("Run the auth tests");
  });
});

  it("preserves tool names and recognized categories", () => {
    expect(readToolCall({ title: "Context", text: "", surface: "activity", data: { name: "lookup_context" } }))
      .toMatchObject({ doing: "Using lookup_context", done: "Used lookup_context" });
    expect(readToolCall({ title: "project", text: "", surface: "activity", data: { kind: "read" } }))
      .toMatchObject({ doing: "Reading", done: "Read", target: "project" });
  });
});

describe("harness subagent facet (issue #667)", () => {
  it.each([
    { type: "collabAgentToolCall", prompt: "Map the login flow", agentsStates: { child: { status: "inProgress" } } },
    { type: "dynamicToolCall", arguments: { prompt: "Map the login flow", subagent_type: "Explore" } },
  ])("keeps a title-less active $type visible once its subagent is identified", (data) => {
    const tool = readToolCall({ text: "", status: "inProgress", surface: "activity", data });
    expect(tool.subagent).toMatchObject({ prompt: "Map the login flow" });
    expect(tool.pendingIdentity).not.toBe(true);
  });

  it("reads agent type, description and prompt off a Task call", () => {
    const tool = readToolCall({
      title: "Task", text: "", status: "inProgress", surface: "activity",
      data: { name: "Task", input: { description: "Explore auth", prompt: "Map the login flow", subagent_type: "Explore" } },
    });
    expect(tool).toMatchObject({ verb: "tool", glyph: "fork", doing: "Delegating", target: "Explore auth" });
    expect(tool.subagent).toEqual({ agentType: "Explore", description: "Explore auth", prompt: "Map the login flow" });
  });

  it("matches task case-insensitively via tool alias and nested state input", () => {
    const tool = readToolCall({
      title: "task", text: "", surface: "activity",
      data: { tool: "task", state: { input: { description: "Research", prompt: "Dig in" } } },
    });
    expect(tool.subagent).toMatchObject({ description: "Research", prompt: "Dig in" });
  });

  it("recognizes Claude's Agent tool as a subagent call", () => {
    const tool = readToolCall({
      title: "Agent", text: "", status: "inProgress", surface: "activity",
      data: { name: "Agent", input: { description: "Investigate", prompt: "Check the logs", agent: "Investigate" } },
    });
    expect(tool).toMatchObject({ verb: "tool", glyph: "fork", doing: "Delegating", target: "Investigate" });
    expect(tool.subagent).toEqual({ agentType: "Investigate", description: "Investigate", prompt: "Check the logs" });
  });

  it("recognizes a collab-agent item type", () => {
    const tool = readToolCall({
      title: "Explore", text: "done", surface: "activity",
      data: { type: "collabAgentToolCall", description: "Explore repo", prompt: "Summarize" },
    });
    expect(tool.subagent).toMatchObject({ description: "Explore repo", prompt: "Summarize" });
  });

  it("leaves generic tools without a subagent facet", () => {
    for (const data of [
      { name: "Bash", input: { command: "bun test" } },
      { name: "Read", input: { file_path: "src/lib.rs" } },
      { name: "mcp__github__search", input: { query: "x" } },
    ]) {
      expect(readToolCall({ title: "t", text: "", surface: "activity", data }).subagent).toBeUndefined();
    }
  });

  it("preserves a bare collab-agent lifecycle record and surfaces its child result", () => {
    const tool = readToolCall({
      title: "Explore", text: "", status: "completed", surface: "activity",
      data: {
        type: "collabAgentToolCall",
        threadId: "t-child",
        agentsStates: { "t-child": { status: "completed", message: "Found three call sites." } },
      },
    });
    expect(tool.subagent).toMatchObject({ status: "completed" });
    expect(tool.output).toBe("Found three call sites.");
  });

  it("reads child lifecycle status from agentsStates", () => {
    const running = readToolCall({
      title: "Explore", text: "", status: "completed", surface: "activity",
      data: { type: "collabAgentToolCall", threadId: "t-child", agentsStates: { "t-child": { status: "inProgress" } } },
    });
    const failed = readToolCall({
      title: "Explore", text: "", status: "completed", surface: "activity",
      data: { type: "collabAgentToolCall", threadId: "t-child", agentsStates: { "t-child": { status: "failed" } } },
    });
    expect(running.subagent?.status).toBe("running");
    expect(failed.subagent?.status).toBe("failed");
  });

  it("does not claim ACP other-kind rows without a task payload", () => {
    expect(readToolCall({ title: "Plan", text: "", surface: "activity", data: { kind: "other" } }).subagent).toBeUndefined();
  });

  it("reads Codex dynamic tool calls from the arguments bag", () => {
    const tool = readToolCall({
      title: "d", text: "", surface: "activity",
      data: { type: "dynamicToolCall", arguments: { description: "Refactor", prompt: "Consolidate handlers", subagent_type: "Refactor" } },
    });
    expect(tool.subagent).toEqual({ agentType: "Refactor", description: "Refactor", prompt: "Consolidate handlers" });
  });

  it("does not claim dynamic tool calls without both type and prompt fields", () => {
    expect(readToolCall({ title: "d", text: "", surface: "activity", data: { type: "dynamicToolCall", prompt: "only prompt" } }).subagent).toBeUndefined();
  });
});

describe("tool marks", () => {
  const shell = (command: string) => readToolCall({ text: "", status: "completed", surface: "activity", data: { name: "Bash", input: { command } } });

  it.each([
    ["git status", "git"],
    ["git commit -m 'x' && git push", "git"],
    ["gh pr view 1", "github"],
    ["bun test", "terminal"],
  ])("gives `%s` the %s mark", (command, glyph) => {
    expect(shell(command).glyph).toBe(glyph);
  });

  it("keeps an exploratory non-git command on its own glyph", () => {
    expect(shell("ls src").glyph).not.toMatch(/git|github/);
  });

  it("names the MCP server and routes GitHub's to the GitHub mark", () => {
    const call = (name: string) => readToolCall({ text: "", status: "completed", surface: "activity", data: { name } });
    expect(call("mcp__notion__search")).toMatchObject({ glyph: "mcp", server: "notion", done: "Used notion", target: "search" });
    expect(call("mcp__claude_ai_Slack__slack_send_message")).toMatchObject({ glyph: "mcp", server: "claude_ai_Slack", done: "Used Slack" });
    expect(call("mcp__github__get_pr")).toMatchObject({ glyph: "github", server: "github" });
  });

  it("reads a Codex MCP item's server off the item", () => {
    const tool = readToolCall({ text: "", status: "completed", surface: "activity", data: { type: "mcpToolCall", server: "linear", tool: "list_issues" } });
    expect(tool).toMatchObject({ glyph: "mcp", server: "linear", target: "list issues" });
  });
});

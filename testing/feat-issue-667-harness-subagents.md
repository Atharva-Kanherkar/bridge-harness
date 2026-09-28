# feat/issue-667-harness-subagents — Test Contract

Issue: #667 — inspecting a local harness's subagents. Harnesses (Claude Code Task, Codex collabAgentToolCall/dynamicToolCall, OpenCode task, ACP kind:other) spawn their own nested subagents. Today they render as one opaque `Delegating <desc>` → `Delegated <desc>` row buried in the collapsed ActivityGroup, with prompt + agent type dropped. No way to see what the subagent was asked or what it did without digging in raw JSON.

Scope: frontend-only. No Rust normalizer change — Claude `Task` already arrives as `tool.started/completed` with `data.{name,input}`, Codex collab/dynamic as `tool.*` with the item bag, OpenCode/ACP the same. The gap is closed in `src/transcript/toolCall.ts` (parse) + `src/components/AgentConversation.tsx` (render), keyed off the normalized tool shape — never off a harness id (contract invariant 1 + harnessBranchGate).

## Functional Behavior

- A harness-spawned subagent call is recognized provider-neutrally:
  - Claude/OpenCode: `data.name`/`data.tool` == `task` (case-insensitive).
  - Codex: `data.type` in {`collabAgentToolCall`, `dynamicToolCall`? no — only when it carries a subagent payload} — conservative: `collabAgentToolCall` always; `dynamicToolCall`/`mcpToolCall` only if they carry `prompt`/`subagent_type`-like fields. Never a bare harness-id branch.
  - ACP: `data.kind == "other"` with a task-like payload is NOT auto-claimed (stays generic tool row).
- Recognized subagent rows show, in order:
  1. Row label: `Delegating <description>` while running → `Delegated <description>` when done (unchanged wording, same `fork` glyph).
  2. Expanded body (new `subagent` body kind, alongside `patch`/`terminal`/`output`): agent-type chip when `subagent_type` present, the `prompt` text, then the result `output`. Prompt and output are separately capped with the existing "earlier output hidden — show all" pattern.
  3. While running with no output yet: body shows the prompt so a minutes-long subagent is not one `inProgress` pulse with nothing under it.
- Non-subagent tool rows are byte-identical in behavior. Grouping is untouched: subagent rows still fold into the ActivityGroup run like any tool work (invariant 1, 2, 7 hold — type/position/status only, no payload-field branching in `grouping.ts`).
- The ActivityGroup summary counts subagent calls as tool work as before (`used N tools`); no new summary vocabulary.

## Unit Tests

- `src/transcript/toolCall.test.ts` — new `describe("harness subagent facet")`:
  - `Task_SubagentTypeAndPrompt` — `readToolCall({data:{name:"Task",input:{description:"Explore auth",prompt:"Map the login flow",subagent_type:"Explore"}}})` → `subagent == {agentType:"Explore",description:"Explore auth",prompt:"Map the login flow"}`, verb `tool`, glyph `fork`.
  - `Task_CaseInsensitiveAndToolAlias` — `data.tool:"task"` also recognized; `data.state.input` nesting (OpenCode shape) also recognized.
  - `Codex_CollabAgentRecognized` — `data:{type:"collabAgentToolCall"}` with description-ish fields → `subagent` present.
  - `GenericTool_NotSubagent` — `Bash`/`Read`/plain `mcp` calls → `subagent` undefined.
  - `ACP_Other_NotClaimed` — `data:{kind:"other"}` without task payload → `subagent` undefined.
- Reducer parity: existing `codec`/`reducer`/`golden` tests keep passing unmodified (no new item type, no grouping change).

## Integration / Functional Tests

- `src/components/AgentConversation.transcript.test.tsx` (extend or new case): a turn containing a `Task` start + completion renders an `ActionRow` whose expanded body shows the prompt text and the result output; a `Bash` row in the same turn renders exactly as before.
- `harnessBranchGate.test.ts` passes unmodified (no harness-id conditionals added).

## Smoke Tests

- `bun run build` (tsc -b && vite build) green.
- Targeted vitest: `toolCall.test.ts`, `grouping.test.ts`, `golden.test.ts`, `AgentConversation.transcript.test.tsx` green.
- Manual: open a session with a Claude `Task` call, expand the ActivityGroup → the Task row → prompt + agent type + result are all readable; collapse state persists per existing behavior.

## E2E Tests

N/A — no new user journey; this is transcript rendering of existing events.

## Manual / cURL Tests

N/A (desktop UI). Manual steps for the reviewer:
1. `bun run dev`, open any chat whose transcript includes a `Task`/`collabAgentToolCall`.
2. Expand Activity → expand the `Delegated …` row → confirm: agent-type chip (if any), prompt block, result block.
3. Confirm a live (running) subagent row shows its prompt before any output arrives.

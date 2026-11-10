import test from "node:test";
import assert from "node:assert/strict";
import { stopTaskFrame } from "../subagents.mjs";
import { buildOptions } from "../options.mjs";

test("stop_task addresses one task through the parent query", async () => {
  const ids=[];
  await stopTaskFrame({stopTask: async id => ids.push(id)}, {type:"stop_task",task_id:"child-task"});
  assert.deepEqual(ids,["child-task"]);
  await assert.rejects(stopTaskFrame({stopTask: async id => ids.push(id)}, {task_id:""}));
  assert.deepEqual(ids,["child-task"]);
});
test("child prose and per-task Stop are enabled without progress model calls", () => {
  const options=buildOptions({sessionId:"s",writeMode:"Full"});
  assert.equal(options.forwardSubagentText,true);
  assert.equal(options.perTaskStopAffordance,true);
  assert.notEqual(options.agentProgressSummaries,true);
});

test("child approvals wait for the matching parent-channel response", async () => {
  const { childPermissions } = await import("../subagents.mjs");
  const frames=[];
  const permissions=childPermissions(frame => frames.push(frame), "parent-session");
  const abort=new AbortController();
  const answer=permissions.canUseTool("Read",{path:"/tmp/test"},{agentID:"task-child",toolUseID:"tool",signal:abort.signal});
  assert.equal(frames[0].request.agentID,"task-child");
  permissions.respond({response:{request_id:frames[0].request_id,response:{behavior:"allow"}}});
  assert.deepEqual(await answer,{behavior:"allow",updatedInput:{path:"/tmp/test"}});
  const cancelled=permissions.canUseTool("Read",{},{agentID:"task-child",signal:abort.signal});
  abort.abort();
  assert.equal((await cancelled).behavior,"deny");
  const parent=await permissions.canUseTool("Read",{},{signal:new AbortController().signal});
  assert.equal(parent.behavior,"deny");
});

test("a briefing run keeps its own gate, so reviewed reads still run", async () => {
  const { childPermissions, withChildPermissions } = await import("../subagents.mjs");
  const briefing={allowedTools:["mcp__notion__search"],allowedServers:["notion"],deniedBuiltins:["Task"]};
  const options=withChildPermissions(buildOptions({sessionId:"s",cwd:"/tmp",briefing}),childPermissions(()=>{},"s"));
  const signal=new AbortController().signal;
  assert.equal((await options.canUseTool("mcp__notion__search",{},{signal,toolUseID:"t"})).behavior,"allow");
  assert.equal((await options.canUseTool("mcp__notion__create_page",{},{signal,toolUseID:"t",agentID:"task"})).behavior,"deny");
  const direct=childPermissions(()=>{},"s");
  assert.equal(withChildPermissions(buildOptions({sessionId:"s",writeMode:"Shared"}),direct).canUseTool,direct.canUseTool);
});

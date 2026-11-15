import { describe, expect, it } from "vitest";
import { defaultsFromSettings, harnessLaunchOverrides, layerChatDefaults, sameDefaultsRevision, settingSourceLabel } from "./chatDefaults";
import type { ChatDefaults, ResolvedChatSettings } from "./protocol/generated/protocol";

const settings: ResolvedChatSettings = { workspaceId:"a",projectId:"p",revision:{application:2,project:4},target:{kind:"direct"},model:{harness:"codex",model:null},effort:null,access:"ask",createWorktree:false,activeTurnInput:"queue",sources:{model:"project"} };
describe("personal chat defaults",()=>{
  it("names every host setting source in user-facing words",()=>{
    expect(["application","project","chat","agent","default"].map(settingSourceLabel)).toEqual(["application default","project default","this chat","selected agent","Bridge default"]);
    expect(settingSourceLabel(undefined)).toBe("this chat");
  });
  it("keeps explicit false, no agent and provider default choices above inherited fields",()=>{
    expect(layerChatDefaults({access:"full",createWorktree:true,target:{kind:"agent",agentId:"research"},effort:{value:"high"}},{access:"ask",createWorktree:false,target:{kind:"direct"},effort:{value:null}})).toEqual({access:"ask",createWorktree:false,target:{kind:"direct"},effort:{value:null}});
    expect(layerChatDefaults({access:"full"},{access:null})).toEqual({access:"full"});
  });
  it("exports only reusable preferences, excluding transient and secret context",()=>{
    const enriched={...settings,cwd:"/private",branch:"main",secret:"do not save",attachments:["image"]};
    expect(defaultsFromSettings(enriched)).toEqual({target:{kind:"direct"},model:{harness:"codex",model:null},effort:{value:null},access:"ask",createWorktree:false,activeTurnInput:"queue"});
  });
  it("compares both revisions independently of object serialization order",()=>{
    expect(sameDefaultsRevision({application:2,project:4},{project:4,application:2})).toBe(true);
    expect(sameDefaultsRevision({application:2,project:4},{application:2,project:5})).toBe(false);
    expect(sameDefaultsRevision({application:2,project:4},{application:3,project:4})).toBe(false);
  });
  it("seeded fuzz: per-project overlays never mutate or leak into another project",()=>{
    let seed=0x8402026;
    for(let i=0;i<10000;i++){
      seed^=seed<<13;seed^=seed>>>17;seed^=seed<<5;
      const app:ChatDefaults={access:seed&1?"full":"ask",createWorktree:!!(seed&2)};
      const project:ChatDefaults=seed&4?{access:"ask",createWorktree:false}:{};
      const before=JSON.stringify(app);
      const a=layerChatDefaults(app,project);const b=layerChatDefaults(app);
      expect(JSON.stringify(app)).toBe(before);expect(b).toEqual(app);
      if(seed&4){expect(a.access).toBe("ask");expect(a.createWorktree).toBe(false);}
    }
  });
});

describe("harnessLaunchOverrides", () => {
  const claude = { id: "claude", defaultModel: "sonnet", models: [
    { id: "sonnet", label: "Sonnet", tier: "standard" as const, defaultForTier: true, supportedEffortLevels: ["low", "high"] },
    { id: "opus", label: "Opus", tier: "strong" as const, defaultForTier: true, supportedEffortLevels: ["low", "high", "xhigh"] },
  ] };
  const codex = { id: "codex", defaultModel: "gpt", models: [{ id: "gpt", label: "GPT", tier: "standard" as const, defaultForTier: true, supportedEffortLevels: ["medium"] }] };
  const defaults = { model: { harness: "claude", model: "opus" } };

  it("asks for nothing on the composer's harness, so the saved model and effort apply", () => {
    expect(harnessLaunchOverrides("claude", defaults, [claude, codex], [])).toEqual({});
  });
  it("uses another harness's saved model and effort", () => {
    expect(harnessLaunchOverrides("claude", { model: { harness: "codex", model: "gpt" } }, [claude, codex], [{ id: "claude", defaultModel: "opus", effort: "xhigh" }]))
      .toEqual({ model: { harness: "claude", model: "opus" }, effort: { value: "xhigh" } });
  });
  it("falls back to the provider default and drops an effort that model lacks", () => {
    expect(harnessLaunchOverrides("claude", { model: { harness: "codex", model: "gpt" } }, [claude, codex], [{ id: "claude", defaultModel: "gone", effort: "xhigh" }]))
      .toEqual({ model: { harness: "claude", model: "sonnet" }, effort: { value: null } });
  });
  it("still names a harness when the defaults could not be resolved", () => {
    expect(harnessLaunchOverrides("codex", null, [claude, codex], [])).toEqual({ model: { harness: "codex", model: "gpt" }, effort: { value: null } });
  });
});

// @vitest-environment jsdom
import { describe, expect, it } from "vitest";
import { bridgeApi } from "./api";
import { codexSkill } from "./chatToolsMock";
import type { ChatToolItem } from "./protocol/generated/protocol";

const find = (items: ChatToolItem[], id: string) => items.find(item => item.id === id);

describe("mock tools of a chat", () => {
  it("lists believable items for each harness with the source of each figure", async () => {
    const codex = await bridgeApi.chatTools({ sessionId: "session-2" });
    expect(find(codex.items, "github")).toMatchObject({ kind: "server", state: "on", tokens: null });
    expect(find(codex.items, "bridge")?.state).toBe("fixed");
    expect(find(codex.items, "codex_apps")).toMatchObject({ kind: "apps", label: "Apps", state: "on", tokens: null });
    expect(find(codex.items, "Notion")).toMatchObject({ kind: "app", state: "fixed" });
    expect(find(codex.items, "pdf")?.tokens?.source).toBe("codex.skills/list bytes_div4_v1");
    expect(codex.sources.map(source => source.available)).toEqual([true, true, true]);
    expect(codex.sources.map(source => [source.label, source.note])).toEqual([
      ["Codex mcpServerStatus/list", null],
      ["Codex skills/list", "The skill figures are estimates. A change after the first turn can make the next request larger"],
      ["Codex config/read", null],
    ]);

    const claude = await bridgeApi.chatTools({ draft: { harness: "claude", workspaceId: "demo-1" } });
    expect(find(claude.items, "claude.ai Gmail")).toMatchObject({ label: "Gmail", group: "claude.ai", tokens: { source: "claude.getContextUsage" } });
    expect(find(claude.items, "superpowers:brainstorming")?.group).toBe("superpowers");
    expect(find(claude.items, "superpowers")).toMatchObject({ kind: "plugin", state: "fixed" });
    expect(claude.sources[1]).toMatchObject({ kind: "claudeInit", label: "Claude init of Plan the release checklist", available: true });

    const opencode = await bridgeApi.chatTools({ draft: { harness: "opencode" } });
    expect(opencode.items).toEqual([expect.objectContaining({ kind: "allSkills", state: "fixed" })]);
    expect(opencode.sources).toEqual([expect.objectContaining({ available: false, note: "Available after the first turn" })]);
    expect(opencode.selection).toMatchObject({ revision: 0, applies: true });

    const cursor = await bridgeApi.chatTools({ draft: { harness: "cursor" } });
    expect(cursor.items).toEqual([]);
    expect(cursor.selection).toMatchObject({ applies: false, notAppliedReason: "This harness has no tool choice" });
  });

  it("saves with the revision, refuses a busy chat and throws on a stale revision", async () => {
    const selection = { harness: "codex", serversOff: ["github"], codexSkillsOff: ["pdf"] };
    await expect(bridgeApi.updateChatTools({ sessionId: "session-2", expectedRevision: 0, selection })).resolves.toEqual({ revision: 1, refusal: null });
    await expect(bridgeApi.updateChatTools({ sessionId: "session-2", expectedRevision: 0, selection })).rejects.toThrow("changed in another window");
    const saved = await bridgeApi.chatTools({ sessionId: "session-2" });
    expect(saved.selection).toMatchObject({ revision: 1, applies: true });
    expect(saved.choice?.serversOff).toEqual(["github"]);
    expect(find(saved.items, "github")?.state).toBe("off");
    expect(find(saved.items, "pdf")?.state).toBe("off");

    const busy = await bridgeApi.updateChatTools({ sessionId: "session-1", expectedRevision: 0, selection });
    expect(busy).toEqual({ revision: null, refusal: "A turn of this chat runs now. Try again when it ends." });
    expect((await bridgeApi.chatTools({ sessionId: "session-1" })).selection.revision).toBe(0);
    await expect(bridgeApi.updateChatTools({ sessionId: "session-2", expectedRevision: 1, selection: { harness: "codex", serversOff: ["bridge"] } })).rejects.toThrow("The Bridge server stays");
  });

  it("stores the first choice of a new chat and refuses one for another harness", async () => {
    const create = (harness: string) => bridgeApi.createConfiguredChat({
      overrides: { target: { kind: "direct" }, model: { harness: "codex" } },
      expectedRevision: { application: 0, project: 0 },
      toolSelection: { harness, serversOff: ["linear"] },
    });
    const created = await create("codex");
    const tools = await bridgeApi.chatTools({ sessionId: created.sessionId });
    expect(tools.selection).toMatchObject({ revision: 1, applies: true });
    expect(find(tools.items, "linear")).toMatchObject({ state: "off", seen: false });
    expect(tools.sources).toEqual([expect.objectContaining({ label: "Codex app-server", note: "Available after the first turn" })]);

    const count = created.state.sessions.length;
    await expect(create("opencode")).rejects.toThrow("The tool choice was made for OpenCode (own login), but the chat starts on Codex (own login)");
    expect((await bridgeApi.state()).sessions).toHaveLength(count);
  });

  it("fixes and refuses a Codex skill name that a config value cannot hold", async () => {
    for (const name of ['say"hi', "back\\slash"]) {
      expect(codexSkill(name, true)).toMatchObject({ id: name, state: "fixed", fixedReason: "This skill name has a quote or a backslash. Bridge cannot turn it off safely." });
      const { selection } = await bridgeApi.chatTools({ sessionId: "session-2" });
      await expect(bridgeApi.updateChatTools({ sessionId: "session-2", expectedRevision: selection.revision, selection: { harness: "codex", codexSkillsOff: [name] } }))
        .rejects.toThrow(`The Codex skill name ${JSON.stringify(name)} cannot go in a config value`);
    }
    expect(codexSkill("pdf", false)).toMatchObject({ state: "off", fixedReason: null });
  });

  it("accepts a choice with each item on for each harness", async () => {
    const allOn = { serversOff: [], codexAppsOff: false, claudeSkills: null, codexSkillsOff: [] };
    for (const harness of ["claude", "codex", "opencode"]) {
      const { sessionId } = await bridgeApi.createConfiguredChat({
        overrides: { target: { kind: "direct" }, model: { harness } },
        expectedRevision: { application: 0, project: 0 },
        toolSelection: { harness, serversOff: ["linear"] },
      });
      await expect(bridgeApi.updateChatTools({ sessionId, expectedRevision: 1, selection: { harness, ...allOn } })).resolves.toEqual({ revision: 2, refusal: null });
      const tools = await bridgeApi.chatTools({ sessionId });
      expect(tools.choice).toEqual({ harness, account: null, ...allOn });
      expect(tools.items.filter(item => item.state === "off")).toEqual([]);
    }
  });

  it("refuses a worker, a missing chat and two targets", async () => {
    await expect(bridgeApi.chatTools({ sessionId: "session-1w" })).rejects.toThrow("Only a top-level chat can have a tool choice");
    await expect(bridgeApi.chatTools({ sessionId: "missing" })).rejects.toThrow("This chat does not exist");
    await expect(bridgeApi.chatTools({})).rejects.toThrow("Give exactly one of sessionId and draft");
  });
});

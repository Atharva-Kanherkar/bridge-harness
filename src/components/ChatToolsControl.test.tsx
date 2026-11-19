// @vitest-environment jsdom
// The composer control for the tools of a chat: a chip with the count of
// left-out items, and a panel that saves one switch at a time with the revision.
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { bridgeApi } from "../api";
import type { ChatToolItem, ChatTools, ChatToolSelection } from "../types";
import { ChatToolsControl } from "./ChatToolsControl";

let container: HTMLDivElement;
let root: Root;

const BRIDGE_REASON = "Bridge needs this server to run the chat.";
const APP_REASON = "Codex attaches each app with the Apps switch.";

const item = (patch: Partial<ChatToolItem> & Pick<ChatToolItem, "id">): ChatToolItem =>
  ({ kind: "server", label: patch.id, group: "MCP servers", state: "on", fixedReason: null, seen: true, tokens: null, ...patch });

/** A warm Codex chat: the Bridge server and the apps stay, the rest can go. */
function codexTools(off: string[] = [], revision = 3): ChatTools {
  const state = (id: string): ChatToolItem["state"] => off.includes(id) ? "off" : "on";
  return {
    harness: "codex",
    account: null,
    choice: off.length ? { harness: "codex", account: null, serversOff: off.filter(id => id !== "pdf"), codexSkillsOff: off.filter(id => id === "pdf") } : null,
    items: [
      item({ id: "bridge", state: "fixed", fixedReason: BRIDGE_REASON }),
      item({ id: "github", state: state("github"), tokens: { value: 4120, source: "codex.mcpServerStatus bytes_div4_v1", at: null } }),
      item({ id: "linear", state: state("linear") }),
      item({ id: "Notion", kind: "app", group: "Apps", state: "fixed", fixedReason: APP_REASON }),
      item({ id: "pdf", kind: "skill", group: "Skills", state: state("pdf") }),
    ],
    sources: [{ kind: "runtime", label: "Codex mcpServerStatus/list", available: true, at: new Date().toISOString(), note: null }],
    selection: { revision, harness: "codex", account: null, applies: true, notAppliedReason: null },
  };
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>(done => { resolve = done; });
  return { promise, resolve };
}

async function settle(rounds = 4) {
  for (let round = 0; round < rounds; round += 1) await act(async () => { await Promise.resolve(); });
}

const chip = () => container.querySelector<HTMLButtonElement>("button[aria-haspopup='menu']");
const row = (id: string) => document.querySelector<HTMLElement>(`[data-tool-item="${id}"]`);
const switchOf = (id: string) => row(id)?.querySelector<HTMLButtonElement>('[role="menuitemcheckbox"]') ?? null;

async function openPanel() {
  act(() => { chip()!.click(); });
  await settle();
}

beforeEach(() => {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.restoreAllMocks();
});

describe("ChatToolsControl", () => {
  it("shows a skeleton while the tools load", async () => {
    const read = deferred<ChatTools>();
    vi.spyOn(bridgeApi, "chatTools").mockReturnValue(read.promise);
    act(() => { root.render(<ChatToolsControl sessionId="s-1" harness="codex" />); });
    expect(chip()!.getAttribute("aria-busy")).toBe("true");
    expect(chip()!.getAttribute("aria-label")).toBe("Tools: loading");
    expect(chip()!.querySelector(".skeleton-shimmer")).not.toBeNull();

    act(() => { chip()!.click(); });
    const region = document.querySelector<HTMLElement>('[role="status"][aria-label="Loading tools"]');
    expect(region?.getAttribute("aria-busy")).toBe("true");
    expect(document.querySelector('[role="menuitemcheckbox"]')).toBeNull();

    await act(async () => { read.resolve(codexTools()); await read.promise; });
    await settle();
    expect(document.querySelector('[aria-label="Loading tools"]')).toBeNull();
    expect(chip()!.hasAttribute("aria-busy")).toBe(false);
    expect(chip()!.getAttribute("aria-label")).toBe("Tools: all on");
    expect(switchOf("github")?.getAttribute("aria-checked")).toBe("true");
  });

  it("shows the count of left-out items on the chip", async () => {
    vi.spyOn(bridgeApi, "chatTools").mockResolvedValue(codexTools(["github", "linear", "pdf"]));
    act(() => { root.render(<ChatToolsControl sessionId="s-1" harness="codex" />); });
    await settle();
    expect(chip()!.textContent).toContain("3 off");
    expect(chip()!.getAttribute("aria-label")).toBe("Tools: 3 left out");

    await openPanel();
    const groups = [...document.querySelectorAll<HTMLElement>('[role="menu"] [role="group"]')].map(group => group.getAttribute("aria-label"));
    expect(groups).toEqual(["Connectors", "Skills"]);
    expect(["github", "linear", "pdf"].map(id => switchOf(id)?.getAttribute("aria-checked"))).toEqual(["false", "false", "false"]);
  });

  it("turns a left-out connector on and saves with the revision", async () => {
    const read = vi.spyOn(bridgeApi, "chatTools")
      .mockResolvedValueOnce(codexTools(["github", "linear"], 3))
      .mockResolvedValueOnce(codexTools(["github", "linear"], 3))
      .mockResolvedValue(codexTools(["linear"], 4));
    const save = deferred<{ revision: number; refusal: null }>();
    const update = vi.spyOn(bridgeApi, "updateChatTools").mockReturnValue(save.promise);
    act(() => { root.render(<ChatToolsControl sessionId="s-1" harness="codex" />); });
    await settle();
    expect(chip()!.textContent).toContain("2 off");
    await openPanel();

    act(() => { switchOf("github")!.click(); });
    expect(update).toHaveBeenCalledTimes(1);
    expect(update).toHaveBeenCalledWith({
      sessionId: "s-1",
      expectedRevision: 3,
      selection: expect.objectContaining({ harness: "codex", serversOff: ["linear"], codexSkillsOff: [] }),
    });
    // The switch shows the new state with a spinner until the host answers.
    expect(switchOf("github")!.getAttribute("aria-checked")).toBe("true");
    expect(switchOf("github")!.disabled).toBe(true);
    expect(row("github")!.querySelector('[aria-label="Loading"]')).not.toBeNull();

    await act(async () => { save.resolve({ revision: 4, refusal: null }); await save.promise; });
    await settle();
    expect(read).toHaveBeenLastCalledWith({ sessionId: "s-1" });
    expect(switchOf("github")!.getAttribute("aria-checked")).toBe("true");
    expect(switchOf("github")!.disabled).toBe(false);
    expect(row("github")!.querySelector('[aria-label="Loading"]')).toBeNull();
    expect(document.querySelector('[role="menu"] [role="status"]')?.textContent).toBe("Saved. The change applies at the next turn.");
    expect(chip()!.textContent).toContain("1 off");

    act(() => { switchOf("linear")!.click(); });
    expect(update).toHaveBeenLastCalledWith(expect.objectContaining({ expectedRevision: 4, selection: expect.objectContaining({ serversOff: [] }) }));
    await settle();
  });

  it("shows the busy refusal and keeps the old choice", async () => {
    // The mock chat session-1 runs a turn, so the mock host refuses each save.
    const read = vi.spyOn(bridgeApi, "chatTools");
    const update = vi.spyOn(bridgeApi, "updateChatTools");
    act(() => { root.render(<ChatToolsControl sessionId="session-1" harness="codex" />); });
    await settle();
    await openPanel();
    const first = document.querySelector<HTMLButtonElement>('[role="menu"] [role="menuitemcheckbox"]')!;
    const id = first.closest<HTMLElement>("[data-tool-item]")!.dataset.toolItem!;
    const before = first.getAttribute("aria-checked");
    const chipBefore = chip()!.getAttribute("aria-label");
    const reads = read.mock.calls.length;

    act(() => { first.click(); });
    await settle();
    expect(update).toHaveBeenCalledWith(expect.objectContaining({ sessionId: "session-1", expectedRevision: 0 }));
    await expect(update.mock.results[0]!.value).resolves.toEqual({ revision: null, refusal: "A turn of this chat runs now. Try again when it ends." });
    expect(document.querySelector('[role="menu"] [role="alert"]')?.textContent).toBe("A turn of this chat runs now. Try again when it ends.");
    expect(switchOf(id)!.getAttribute("aria-checked")).toBe(before);
    expect(switchOf(id)!.disabled).toBe(false);
    expect(chip()!.getAttribute("aria-label")).toBe(chipBefore);
    expect(read.mock.calls.length).toBe(reads);
    expect((await bridgeApi.chatTools({ sessionId: "session-1" })).selection.revision).toBe(0);
  });

  it("shows fixed items with the reason and no switch", async () => {
    vi.spyOn(bridgeApi, "chatTools").mockResolvedValue(codexTools());
    act(() => { root.render(<ChatToolsControl sessionId="s-1" harness="codex" />); });
    await settle();
    await openPanel();
    expect(row("bridge")!.textContent).toContain(BRIDGE_REASON);
    expect(switchOf("bridge")).toBeNull();
    expect(row("Notion")!.textContent).toContain(APP_REASON);
    expect(switchOf("Notion")).toBeNull();
    expect(switchOf("github")).not.toBeNull();
    expect(row("github")!.textContent).not.toContain(BRIDGE_REASON);
  });

  it("shows a choice for another harness as not applied", async () => {
    const reason = "Chosen for Claude Code (own login), not applied";
    vi.spyOn(bridgeApi, "chatTools").mockResolvedValue({
      ...codexTools(),
      choice: { harness: "claude", account: null, serversOff: ["chrome-devtools"] },
      selection: { revision: 2, harness: "claude", account: null, applies: false, notAppliedReason: reason },
    });
    act(() => { root.render(<ChatToolsControl sessionId="s-1" harness="codex" />); });
    await settle();
    expect(chip()!.getAttribute("aria-label")).toBe("Tools: not applied");
    expect(chip()!.textContent).toContain("not applied");
    await openPanel();
    expect(document.querySelector('[role="menu"] [role="note"]')?.textContent).toBe(reason);
    act(() => root.unmount());

    // A draft keeps its choice locally, so the control finds the mismatch itself.
    root = createRoot(container);
    const onChoiceChange = vi.fn();
    const choice: ChatToolSelection = { harness: "claude", account: null, serversOff: ["chrome-devtools"] };
    vi.spyOn(bridgeApi, "chatTools").mockResolvedValue(codexTools([], 0));
    act(() => { root.render(<ChatToolsControl draft={{ harness: "codex", account: null, workspaceId: "demo-1" }} choice={choice} onChoiceChange={onChoiceChange} />); });
    await settle();
    expect(chip()!.getAttribute("aria-label")).toBe("Tools: not applied");
    await openPanel();
    expect(document.querySelector('[role="menu"] [role="note"]')?.textContent).toBe(reason);
    act(() => { switchOf("github")!.click(); });
    expect(onChoiceChange).toHaveBeenCalledWith(expect.objectContaining({ harness: "codex", account: null, serversOff: ["github"] }));
  });

  it("shows and edits a choice of another account as no choice", async () => {
    const reason = "Chosen for Claude Code (account home), not applied";
    const choice: ChatToolSelection = { harness: "claude", account: "home", serversOff: ["chrome-devtools"], claudeSkills: { only: ["review"] } };
    // The host sets the states from a saved choice of the same harness, whatever its account.
    const claudeTools = (saved: ChatToolSelection | null, states: Record<string, ChatToolItem["state"]>): ChatTools => ({
      harness: "claude",
      account: "work",
      choice: saved,
      items: [
        item({ id: "bridge", state: "fixed", fixedReason: BRIDGE_REASON }),
        item({ id: "chrome-devtools", state: states["chrome-devtools"] ?? "on" }),
        item({ id: "github" }),
        item({ id: "review", kind: "skill", group: "Skills" }),
        item({ id: "simplify", kind: "skill", group: "Skills", state: states.simplify ?? "on" }),
      ],
      sources: [],
      selection: saved
        ? { revision: 2, harness: "claude", account: "home", applies: false, notAppliedReason: reason }
        : { revision: 0, harness: "claude", account: "work", applies: true, notAppliedReason: null },
    });
    const ids = ["chrome-devtools", "github", "review", "simplify"];
    vi.spyOn(bridgeApi, "chatTools").mockResolvedValue(claudeTools(choice, { "chrome-devtools": "off", simplify: "off" }));
    const update = vi.spyOn(bridgeApi, "updateChatTools").mockReturnValue(new Promise(() => undefined));
    act(() => { root.render(<ChatToolsControl sessionId="s-1" harness="claude" account="work" />); });
    await settle();
    await openPanel();
    expect(document.querySelector('[role="menu"] [role="note"]')?.textContent).toBe(reason);
    expect(ids.map(id => switchOf(id)?.getAttribute("aria-checked"))).toEqual(["true", "true", "true", "true"]);
    act(() => { switchOf("github")!.click(); });
    expect(update).toHaveBeenCalledWith({
      sessionId: "s-1",
      expectedRevision: 2,
      selection: { harness: "claude", account: "work", serversOff: ["github"], codexAppsOff: false, claudeSkills: null, codexSkillsOff: [] },
    });
    act(() => root.unmount());

    // A draft keeps the choice of another account too, and a switch starts a new one.
    root = createRoot(container);
    vi.spyOn(bridgeApi, "chatTools").mockResolvedValue(claudeTools(null, {}));
    const onChoiceChange = vi.fn();
    act(() => { root.render(<ChatToolsControl draft={{ harness: "claude", account: "work", workspaceId: null }} choice={choice} onChoiceChange={onChoiceChange} />); });
    await settle();
    await openPanel();
    expect(document.querySelector('[role="menu"] [role="note"]')?.textContent).toBe(reason);
    expect(ids.map(id => switchOf(id)?.getAttribute("aria-checked"))).toEqual(["true", "true", "true", "true"]);
    act(() => { switchOf("simplify")!.click(); });
    expect(onChoiceChange).toHaveBeenCalledWith({ harness: "claude", account: "work", serversOff: [], codexAppsOff: false, claudeSkills: { only: ["review"] }, codexSkillsOff: [] });
  });

  it("shows a saved replacement as applied when the read after the save fails", async () => {
    const reason = "Chosen for Claude Code (account home), not applied";
    const tools: ChatTools = {
      harness: "claude",
      account: "work",
      choice: { harness: "claude", account: "home", serversOff: ["github"] },
      items: [item({ id: "github", state: "off" }), item({ id: "linear" })],
      sources: [],
      selection: { revision: 2, harness: "claude", account: "home", applies: false, notAppliedReason: reason },
    };
    vi.spyOn(bridgeApi, "chatTools").mockResolvedValueOnce(tools).mockRejectedValue(new Error("The host did not answer"));
    vi.spyOn(bridgeApi, "updateChatTools").mockResolvedValue({ revision: 3, refusal: null });
    act(() => { root.render(<ChatToolsControl sessionId="s-1" harness="claude" account="work" />); });
    await settle();
    await openPanel();
    expect(document.querySelector('[role="menu"] [role="note"]')?.textContent).toBe(reason);

    act(() => { switchOf("linear")!.click(); });
    await settle();
    expect(document.querySelector('[role="menu"] [role="status"]')?.textContent).toBe("Saved. The change applies at the next turn.");
    expect(document.querySelector('[role="menu"] [role="note"]')).toBeNull();
    expect(chip()!.getAttribute("aria-label")).toBe("Tools: 1 left out");
    expect(["github", "linear"].map(id => switchOf(id)?.getAttribute("aria-checked"))).toEqual(["true", "false"]);
  });

  it("keeps a save of one chat out of the state of the next chat", async () => {
    const read = vi.spyOn(bridgeApi, "chatTools").mockImplementation(async params => codexTools([], params.sessionId === "a" ? 3 : 7));
    const saveA = deferred<{ revision: number; refusal: null }>();
    let failA!: (error: Error) => void;
    const rejected = new Promise<{ revision: number; refusal: null }>((_, reject) => { failA = reject; });
    vi.spyOn(bridgeApi, "updateChatTools").mockImplementation(params => params.sessionId === "a" ? rejected : saveA.promise);
    act(() => { root.render(<ChatToolsControl sessionId="a" harness="codex" />); });
    await settle();
    await openPanel();
    act(() => { switchOf("github")!.click(); });
    expect(switchOf("github")!.disabled).toBe(true);

    // The composer moves to chat b, which starts its own save.
    act(() => { root.render(<ChatToolsControl sessionId="b" harness="codex" />); });
    await settle();
    expect(switchOf("linear")!.disabled).toBe(false);
    act(() => { switchOf("linear")!.click(); });
    expect(switchOf("linear")!.disabled).toBe(true);
    const reads = read.mock.calls.length;

    await act(async () => { failA(new Error("The tool choice changed in another window. Reload it and try again")); await rejected.catch(() => undefined); });
    await settle();
    expect(document.querySelector('[role="menu"] [role="alert"]')).toBeNull();
    expect(read.mock.calls.slice(reads)).toEqual([]);
    expect(chip()!.getAttribute("aria-label")).toBe("Tools: all on");
    expect(switchOf("linear")!.disabled).toBe(true);
    expect(row("linear")!.querySelector('[aria-label="Loading"]')).not.toBeNull();
  });

  it("reads a chat again when its own save ends after a return to it", async () => {
    let committed = false;
    const read = vi.spyOn(bridgeApi, "chatTools").mockImplementation(async params =>
      params.sessionId === "b" ? codexTools([], 7) : committed ? codexTools(["github"], 4) : codexTools([], 3));
    const saveA = deferred<{ revision: number; refusal: null }>();
    const update = vi.spyOn(bridgeApi, "updateChatTools").mockReturnValueOnce(saveA.promise).mockReturnValue(new Promise(() => undefined));
    act(() => { root.render(<ChatToolsControl sessionId="a" harness="codex" />); });
    await settle();
    await openPanel();
    act(() => { switchOf("github")!.click(); });

    // The person moves to chat b and back to chat a before the save of a ends.
    act(() => { root.render(<ChatToolsControl sessionId="b" harness="codex" />); });
    await settle();
    act(() => { root.render(<ChatToolsControl sessionId="a" harness="codex" />); });
    await settle();
    expect(switchOf("github")!.disabled).toBe(true);
    const reads = read.mock.calls.length;

    committed = true;
    await act(async () => { saveA.resolve({ revision: 4, refusal: null }); await saveA.promise; });
    await settle();
    expect(read.mock.calls.slice(reads)).toEqual([[{ sessionId: "a" }]]);
    expect(switchOf("github")!.disabled).toBe(false);
    expect(switchOf("github")!.getAttribute("aria-checked")).toBe("false");
    expect(chip()!.getAttribute("aria-label")).toBe("Tools: 1 left out");
    act(() => { switchOf("linear")!.click(); });
    expect(update).toHaveBeenLastCalledWith(expect.objectContaining({ sessionId: "a", expectedRevision: 4 }));
  });

  it("starts the read after a save only when the read in flight ends", async () => {
    const reopen = deferred<ChatTools>();
    const read = vi.spyOn(bridgeApi, "chatTools")
      .mockResolvedValueOnce(codexTools([], 3))
      .mockReturnValueOnce(reopen.promise)
      .mockResolvedValue(codexTools(["github"], 4));
    vi.spyOn(bridgeApi, "updateChatTools").mockResolvedValue({ revision: 4, refusal: null });
    act(() => { root.render(<ChatToolsControl sessionId="s-1" harness="codex" />); });
    await settle();
    // A reopen reads the chat again and keeps the last list on screen meanwhile.
    await openPanel();
    expect(read).toHaveBeenCalledTimes(2);

    act(() => { switchOf("github")!.click(); });
    await settle();
    expect(read).toHaveBeenCalledTimes(2);
    expect(switchOf("github")!.disabled).toBe(true);

    await act(async () => { reopen.resolve(codexTools([], 3)); await reopen.promise; });
    await settle();
    expect(read).toHaveBeenCalledTimes(3);
    expect(switchOf("github")!.disabled).toBe(false);
    expect(switchOf("github")!.getAttribute("aria-checked")).toBe("false");
  });

  it("changes nothing and reads nothing when a save ends after the control unmounts", async () => {
    const read = vi.spyOn(bridgeApi, "chatTools").mockResolvedValue(codexTools([], 3));
    let fail!: (error: Error) => void;
    const failed = new Promise<{ revision: number; refusal: null }>((_, reject) => { fail = reject; });
    const saved = deferred<{ revision: number; refusal: null }>();
    vi.spyOn(bridgeApi, "updateChatTools").mockReturnValueOnce(failed).mockReturnValueOnce(saved.promise);
    const errors = vi.spyOn(console, "error");
    for (const end of [
      () => { fail(new Error("The tool choice changed in another window. Reload it and try again")); return failed.catch(() => undefined); },
      () => { saved.resolve({ revision: 4, refusal: null }); return saved.promise; },
    ]) {
      act(() => { root.render(<ChatToolsControl sessionId="s-1" harness="codex" />); });
      await settle();
      await openPanel();
      act(() => { switchOf("github")!.click(); });
      act(() => root.unmount());
      root = createRoot(container);
      const reads = read.mock.calls.length;
      await act(async () => { await end(); });
      await settle();
      expect(read.mock.calls.length).toBe(reads);
    }
    expect(errors).not.toHaveBeenCalled();
  });

  it("shares one read of a chat between the mount and an early open", async () => {
    const first = deferred<ChatTools>();
    const read = vi.spyOn(bridgeApi, "chatTools").mockReturnValueOnce(first.promise).mockResolvedValue(codexTools(["github"], 4));
    act(() => { root.render(<ChatToolsControl sessionId="s-1" harness="codex" />); });
    // A warm Codex read asks its runtime three things, under a limit of four for each chat.
    act(() => { chip()!.click(); });
    expect(read).toHaveBeenCalledTimes(1);
    await act(async () => { first.resolve(codexTools()); await first.promise; });
    await settle();
    expect(switchOf("github")?.getAttribute("aria-checked")).toBe("true");

    // An open after the read answered reads again.
    act(() => { chip()!.click(); });
    await openPanel();
    expect(read).toHaveBeenCalledTimes(2);
    expect(switchOf("github")?.getAttribute("aria-checked")).toBe("false");
  });

  it("keeps the focus on a focused switch that a click turned", async () => {
    const frames = () => act(async () => { await new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))); });
    // A browser turns Enter or Space on a focused button into a click, but jsdom does not.
    // So the test focuses the switch and clicks it, and the menu stays open.
    const press = async (target: HTMLButtonElement) => {
      act(() => { target.focus(); target.click(); });
      await frames();
    };
    vi.spyOn(bridgeApi, "chatTools").mockResolvedValue(codexTools([], 0));
    const onChoiceChange = vi.fn();
    act(() => { root.render(<ChatToolsControl draft={{ harness: "codex", account: null, workspaceId: null }} choice={null} onChoiceChange={onChoiceChange} />); });
    await settle();
    await openPanel();
    await press(switchOf("linear")!);
    expect(onChoiceChange).toHaveBeenCalledTimes(1);
    expect(document.activeElement).toBe(switchOf("linear"));
    act(() => root.unmount());

    // A chat disables the switch while the save runs, and gives the focus back when it ends.
    root = createRoot(container);
    vi.spyOn(bridgeApi, "chatTools").mockResolvedValue(codexTools());
    const save = deferred<{ revision: number; refusal: null }>();
    vi.spyOn(bridgeApi, "updateChatTools").mockReturnValue(save.promise);
    act(() => { root.render(<ChatToolsControl sessionId="s-1" harness="codex" />); });
    await settle();
    await openPanel();
    await press(switchOf("linear")!);
    expect(switchOf("linear")!.disabled).toBe(true);
    await act(async () => { save.resolve({ revision: 4, refusal: null }); await save.promise; });
    await settle();
    await frames();
    expect(switchOf("linear")!.disabled).toBe(false);
    expect(document.activeElement).toBe(switchOf("linear"));
  });

  it("finds the switch by its kind and its id when a server and a skill share a name", async () => {
    const frames = () => act(async () => { await new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))); });
    const switchIn = (kind: string, id: string) =>
      document.querySelector<HTMLButtonElement>(`[data-tool-kind="${kind}"][data-tool-item="${id}"] [role="menuitemcheckbox"]`);
    vi.spyOn(bridgeApi, "chatTools").mockResolvedValue({
      harness: "claude",
      account: null,
      choice: null,
      items: [item({ id: "review" }), item({ id: "review", kind: "skill", group: "Skills" }), item({ id: "simplify", kind: "skill", group: "Skills" })],
      sources: [],
      selection: { revision: 1, harness: "claude", account: null, applies: true, notAppliedReason: null },
    });
    const save = deferred<{ revision: number; refusal: null }>();
    vi.spyOn(bridgeApi, "updateChatTools").mockReturnValue(save.promise);
    act(() => { root.render(<ChatToolsControl sessionId="s-1" harness="claude" />); });
    await settle();
    await openPanel();
    const skill = switchIn("skill", "review")!;
    act(() => { skill.focus(); skill.click(); });
    await frames();
    // Only the skill row shows the save in flight.
    expect(switchIn("skill", "review")!.getAttribute("aria-checked")).toBe("false");
    expect(switchIn("server", "review")!.getAttribute("aria-checked")).toBe("true");
    await act(async () => { save.resolve({ revision: 2, refusal: null }); await save.promise; });
    await settle();
    await frames();
    expect(document.activeElement).toBe(switchIn("skill", "review"));
  });

  it("drops the focus request when the menu closes", async () => {
    const frames = () => act(async () => { await new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))); });
    vi.spyOn(bridgeApi, "chatTools").mockResolvedValue(codexTools());
    const save = deferred<{ revision: number; refusal: null }>();
    vi.spyOn(bridgeApi, "updateChatTools").mockReturnValue(save.promise);
    act(() => { root.render(<ChatToolsControl sessionId="s-1" harness="codex" />); });
    await settle();
    await openPanel();
    const linear = switchOf("linear")!;
    act(() => { linear.focus(); linear.click(); });
    await frames();

    // Escape closes the menu while the save runs, and the person opens it again.
    act(() => { document.querySelector('[role="menu"]')!.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true })); });
    await frames();
    await openPanel();
    expect(switchOf("linear")!.disabled).toBe(true);
    await act(async () => { save.resolve({ revision: 4, refusal: null }); await save.promise; });
    await settle();
    await frames();
    expect(switchOf("linear")!.disabled).toBe(false);
    expect(document.activeElement).not.toBe(switchOf("linear"));
  });

  it("cancels the queued focus frame when the control unmounts", async () => {
    vi.spyOn(bridgeApi, "chatTools").mockResolvedValue(codexTools([], 0));
    act(() => { root.render(<ChatToolsControl draft={{ harness: "codex", account: null, workspaceId: null }} choice={null} onChoiceChange={() => undefined} />); });
    await settle();
    await openPanel();
    const queue = vi.spyOn(window, "requestAnimationFrame");
    const cancel = vi.spyOn(window, "cancelAnimationFrame");
    act(() => { switchOf("linear")!.click(); });
    const frame = queue.mock.results.at(-1)!.value as number;
    act(() => root.unmount());
    root = createRoot(container);
    expect(cancel).toHaveBeenCalledWith(frame);
  });

  it("turns all items on with one action that saves the set of no choice", async () => {
    const allOn: ChatToolSelection = { harness: "claude", account: null, serversOff: [], codexAppsOff: false, claudeSkills: null, codexSkillsOff: [] };
    const claudeTools = (choice: ChatToolSelection | null, revision: number, off: string[]): ChatTools => ({
      harness: "claude",
      account: null,
      choice,
      items: ["github", "linear"].map(id => item({ id, state: off.includes(id) ? "off" : "on" }))
        .concat(["review", "simplify"].map(id => item({ id, kind: "skill", group: "Skills", state: off.includes(id) ? "off" : "on" }))),
      sources: [],
      selection: { revision, harness: "claude", account: null, applies: true, notAppliedReason: null },
    });
    const turnAllOn = () => [...document.querySelectorAll<HTMLButtonElement>('[role="menu"] [role="menuitem"]')].find(button => button.textContent === "Turn all on");
    vi.spyOn(bridgeApi, "chatTools")
      .mockResolvedValueOnce(claudeTools({ harness: "claude", account: null, serversOff: ["github"], claudeSkills: { only: ["review"] } }, 5, ["github", "simplify"]))
      .mockResolvedValueOnce(claudeTools({ harness: "claude", account: null, serversOff: ["github"], claudeSkills: { only: ["review"] } }, 5, ["github", "simplify"]))
      .mockResolvedValue(claudeTools(allOn, 6, []));
    const update = vi.spyOn(bridgeApi, "updateChatTools").mockResolvedValue({ revision: 6, refusal: null });
    act(() => { root.render(<ChatToolsControl sessionId="s-1" harness="claude" />); });
    await settle();
    await openPanel();
    act(() => { turnAllOn()!.click(); });
    expect(update).toHaveBeenCalledWith({ sessionId: "s-1", expectedRevision: 5, selection: allOn });
    await settle();
    expect(chip()!.getAttribute("aria-label")).toBe("Tools: all on");
    expect(turnAllOn()).toBeUndefined();
    act(() => root.unmount());

    // A Claude list that names each skill is not the set of no choice, so the action stays.
    root = createRoot(container);
    vi.spyOn(bridgeApi, "chatTools").mockResolvedValue(claudeTools({ harness: "claude", account: null, claudeSkills: { only: ["review", "simplify"] } }, 2, []));
    act(() => { root.render(<ChatToolsControl sessionId="s-1" harness="claude" />); });
    await settle();
    await openPanel();
    expect(chip()!.getAttribute("aria-label")).toBe("Tools: all on");
    expect(turnAllOn()).toBeDefined();
    act(() => root.unmount());

    // A draft drops its choice, which is the same set and fits each target.
    root = createRoot(container);
    vi.spyOn(bridgeApi, "chatTools").mockResolvedValue(codexTools([], 0));
    const onChoiceChange = vi.fn();
    act(() => { root.render(<ChatToolsControl draft={{ harness: "codex", account: null, workspaceId: null }} choice={{ harness: "codex", account: null, codexAppsOff: true }} onChoiceChange={onChoiceChange} />); });
    await settle();
    await openPanel();
    act(() => { turnAllOn()!.click(); });
    expect(onChoiceChange).toHaveBeenCalledWith(null);
  });

  it("offers no action to turn all on when each item is on", async () => {
    const turnAllOn = () => [...document.querySelectorAll<HTMLButtonElement>('[role="menu"] [role="menuitem"]')].find(button => button.textContent === "Turn all on");
    for (const tools of [
      codexTools(),
      { ...codexTools(), choice: { harness: "codex", account: null, serversOff: [], codexAppsOff: false, codexSkillsOff: [] } },
    ]) {
      vi.spyOn(bridgeApi, "chatTools").mockResolvedValue(tools);
      act(() => { root.render(<ChatToolsControl sessionId="s-1" harness="codex" />); });
      await settle();
      await openPanel();
      expect(switchOf("github")).not.toBeNull();
      expect(turnAllOn()).toBeUndefined();
      act(() => root.unmount());
      root = createRoot(container);
    }
    act(() => { root.render(<ChatToolsControl draft={{ harness: "codex", account: null, workspaceId: null }} choice={null} onChoiceChange={() => undefined} />); });
    await settle();
    await openPanel();
    expect(switchOf("github")).not.toBeNull();
    expect(turnAllOn()).toBeUndefined();
  });

  it("shows a token figure with its source and none when absent", async () => {
    vi.spyOn(bridgeApi, "chatTools").mockResolvedValue(codexTools());
    act(() => { root.render(<ChatToolsControl sessionId="s-1" harness="codex" />); });
    await settle();
    await openPanel();
    expect(row("github")!.textContent).toContain("4,120 tokens · codex.mcpServerStatus bytes_div4_v1");
    // The row cuts a long source, so the full text stays on the element.
    expect(row("github")!.querySelector('[title="4,120 tokens · codex.mcpServerStatus bytes_div4_v1"]')?.textContent).toBe("4,120 tokens · codex.mcpServerStatus bytes_div4_v1");
    expect(row("linear")!.textContent).toBe("linear");
    expect(row("linear")!.textContent).not.toContain("tokens");
  });

  it("keeps a draft choice in view on a harness without units until the person clears it", async () => {
    const choice: ChatToolSelection = { harness: "codex", account: null, codexAppsOff: true };
    const onChoiceChange = vi.fn();
    const draft = (selection: ChatToolSelection | null) =>
      <ChatToolsControl draft={{ harness: "cursor", account: null, workspaceId: null }} choice={selection} onChoiceChange={onChoiceChange} />;
    act(() => { root.render(draft(choice)); });
    await settle();
    expect(chip()?.getAttribute("aria-label")).toBe("Tools: not applied");
    await openPanel();
    expect(document.querySelector('[role="menu"] [role="note"]')?.textContent).toBe("Chosen for Codex (own login), not applied");
    const clear = [...document.querySelectorAll<HTMLButtonElement>('[role="menu"] [role="menuitem"]')].find(button => button.textContent === "Clear this choice");
    act(() => { clear!.click(); });
    expect(onChoiceChange).toHaveBeenCalledWith(null);

    act(() => { root.render(draft(null)); });
    await settle();
    expect(container.innerHTML).toBe("");
  });

  it("hides the control for a harness without units", async () => {
    act(() => { root.render(<ChatToolsControl draft={{ harness: "cursor", account: null, workspaceId: null }} choice={null} onChoiceChange={() => undefined} />); });
    await settle();
    expect(container.innerHTML).toBe("");

    act(() => { root.render(<ChatToolsControl draft={{ harness: "codex", account: null, workspaceId: null }} choice={null} onChoiceChange={() => undefined} />); });
    await settle();
    expect(chip()).not.toBeNull();
  });
});

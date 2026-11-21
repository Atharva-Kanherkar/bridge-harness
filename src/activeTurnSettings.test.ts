// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { bridgeApi } from "./api";
import { LEGACY_ACTIVE_TURN_INPUT_KEY, migrateLegacyActiveTurnInput, resetActiveTurnInputForTests, resolveActiveTurnInput } from "./activeTurnSettings";
import type { ChatDefaultsView, ChatSessionSettings } from "./protocol/generated/protocol";

const session = (revision: number, activeTurnInput: "steer" | "queue"): ChatSessionSettings => ({ revision, settings: { activeTurnInput } } as unknown as ChatSessionSettings);
const view = (defaults: ChatDefaultsView["application"]["defaults"], revision = 4): ChatDefaultsView => ({ application: { revision, defaults }, project: null });

beforeEach(() => { localStorage.clear(); resetActiveTurnInputForTests(); });
afterEach(() => { vi.restoreAllMocks(); });

describe("resolveActiveTurnInput", () => {
  it("uses a configured chat's own value over the application default", () => {
    expect(resolveActiveTurnInput(session(2, "steer"), "queue")).toBe("steer");
    expect(resolveActiveTurnInput(session(2, "queue"), null)).toBe("queue");
  });

  it("falls back to the application default for a legacy chat, then to Steer", () => {
    expect(resolveActiveTurnInput(session(0, "steer"), "queue")).toBe("queue");
    expect(resolveActiveTurnInput(undefined, "queue")).toBe("queue");
    expect(resolveActiveTurnInput(undefined, null)).toBe("steer");
  });
});

describe("migrateLegacyActiveTurnInput", () => {
  it("moves a legacy queue into an unset application default once, then forgets the key", async () => {
    localStorage.setItem(LEGACY_ACTIVE_TURN_INPUT_KEY, "queue");
    const save = vi.spyOn(bridgeApi, "saveChatDefaults").mockImplementation(async params => view(params.defaults, 5));
    const next = await migrateLegacyActiveTurnInput(view({ access: "full" }));
    expect(save).toHaveBeenCalledWith({ projectId: null, expectedRevision: 4, defaults: { access: "full", activeTurnInput: "queue" } });
    expect(next.application.defaults.activeTurnInput).toBe("queue");
    expect(localStorage.getItem(LEGACY_ACTIVE_TURN_INPUT_KEY)).toBeNull();
    await migrateLegacyActiveTurnInput(next);
    expect(save).toHaveBeenCalledTimes(1);
  });

  it("never overwrites a default the person already set", async () => {
    localStorage.setItem(LEGACY_ACTIVE_TURN_INPUT_KEY, "queue");
    const save = vi.spyOn(bridgeApi, "saveChatDefaults");
    const next = await migrateLegacyActiveTurnInput(view({ activeTurnInput: "steer" }));
    expect(save).not.toHaveBeenCalled();
    expect(next.application.defaults.activeTurnInput).toBe("steer");
    expect(localStorage.getItem(LEGACY_ACTIVE_TURN_INPUT_KEY)).toBeNull();
  });

  it("drops a legacy steer without writing anything", async () => {
    localStorage.setItem(LEGACY_ACTIVE_TURN_INPUT_KEY, "steer");
    const save = vi.spyOn(bridgeApi, "saveChatDefaults");
    await migrateLegacyActiveTurnInput(view({}));
    expect(save).not.toHaveBeenCalled();
    expect(localStorage.getItem(LEGACY_ACTIVE_TURN_INPUT_KEY)).toBeNull();
  });

  it("keeps the legacy key when the save fails, so the next launch retries", async () => {
    localStorage.setItem(LEGACY_ACTIVE_TURN_INPUT_KEY, "queue");
    vi.spyOn(bridgeApi, "saveChatDefaults").mockRejectedValue(new Error("Defaults changed in another window"));
    const next = await migrateLegacyActiveTurnInput(view({}));
    expect(next.application.defaults.activeTurnInput).toBeUndefined();
    expect(localStorage.getItem(LEGACY_ACTIVE_TURN_INPUT_KEY)).toBe("queue");
  });
});

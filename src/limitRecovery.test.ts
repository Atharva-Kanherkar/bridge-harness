import { expect, it } from "vitest";
import type { LimitRecoveryView } from "./protocol/generated/protocol";
import type { AdapterDescriptor } from "./types";
import { limitRecoveryNotice, targetLabel, untilText } from "./limitRecovery";

const adapters = [{ id: "codex", label: "Codex", models: [{ id: "gpt-5.5", label: "GPT-5.5" }] }] as unknown as AdapterDescriptor[];
const now = new Date("2026-11-15T10:00:00Z");
const view = (patch: Partial<LimitRecoveryView>): LimitRecoveryView => ({
  sessionId: "s", mode: "resume", state: "pending", signal: "usage limit", resetAt: "2026-11-15T13:20:00Z",
  dueAt: "2026-11-15T13:21:30Z", target: { harness: "claude", model: "opus", effort: null }, switchesModel: false,
  attempts: 1, outcome: null, updatedAt: "2026-11-15T10:00:00Z", ...patch,
});

it("counts down in plain units", () => {
  expect(untilText(new Date(now.getTime() + 30_000), now)).toBe("in under a minute");
  expect(untilText(new Date(now.getTime() - 60_000), now)).toBe("in under a minute");
  expect(untilText(new Date(now.getTime() + 4 * 60_000), now)).toBe("in 4 min");
  expect(untilText(new Date(now.getTime() + 60 * 60_000), now)).toBe("in 1 h");
  expect(untilText(new Date(now.getTime() + 201 * 60_000), now)).toBe("in 3 h 21 min");
});

it("names the backup from the catalog, falling back to its ids", () => {
  expect(targetLabel({ harness: "codex", model: "gpt-5.5" }, adapters)).toBe("Codex · GPT-5.5");
  expect(targetLabel({ harness: "codex", model: "unknown" }, adapters)).toBe("Codex · unknown");
  expect(targetLabel({ harness: "pi", model: null }, adapters)).toBe("pi · default model");
});

it("says when a waiting chat continues and why, with actions", () => {
  const resume = limitRecoveryNotice(view({}), adapters, now, "en-US")!;
  expect(resume).toEqual({ tone: "waiting", actionable: true, text: expect.stringContaining("(in 3 h 22 min), when the limit resets.") });
  const probe = limitRecoveryNotice(view({ resetAt: null }), adapters, now, "en-US")!;
  expect(probe.text).toContain("to check whether the limit has cleared");
  const backup = limitRecoveryNotice(view({ switchesModel: true, target: { harness: "codex", model: "gpt-5.5" } }), adapters, now)!;
  expect(backup.text).toBe("Usage limit reached. Switching to Codex · GPT-5.5…");
});

it("explains failures without offering actions, and hides finished rows", () => {
  expect(limitRecoveryNotice(view({ state: "acting" }), adapters, now)).toMatchObject({ actionable: false, tone: "waiting" });
  expect(limitRecoveryNotice(view({ state: "failed", outcome: "provider is not running" }), adapters, now)).toMatchObject({ tone: "problem", actionable: false, text: expect.stringContaining("provider is not running") });
  expect(limitRecoveryNotice(view({ state: "gave_up", attempts: 6 }), adapters, now)!.text).toContain("6 times");
  for (const state of ["done", "cancelled", "expired"] as const) expect(limitRecoveryNotice(view({ state }), adapters, now)).toBeNull();
  expect(limitRecoveryNotice(null, adapters, now)).toBeNull();
});

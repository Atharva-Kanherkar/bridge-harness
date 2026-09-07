import { describe, expect, it } from "vitest";
import { canTransition, formatElapsed, harnessLabel, safeSlug, slashOwnershipBadge, tierRuntimeLabel } from "./utils";

describe("slashOwnershipBadge", () => {
  it("marks only the bridge catalog as local", () => {
    expect(slashOwnershipBadge("bridge")).toBe("this Mac");
    expect(slashOwnershipBadge("claude")).toBe("Claude");
    expect(slashOwnershipBadge("codex")).toBe("Codex");
  });
  it("treats a harness it has never heard of as provider-owned, not local", () => {
    expect(slashOwnershipBadge("futureagent")).toBe("Futureagent");
  });
});

describe("safeSlug", () => {
  it("normalizes unsafe branch text", () => expect(safeSlug(" Add OAuth / callbacks! ")).toBe("add-oauth-callbacks"));
  it("never returns an empty slug", () => expect(safeSlug("!!!")).toBe("task"));
});

describe("session transitions", () => {
  it("accepts a supervised waiting cycle", () => { expect(canTransition("working", "waiting")).toBe(true); expect(canTransition("waiting", "working")).toBe(true); });
  it("accepts the durable worker resume path", () => {
    expect(canTransition("stopped", "resuming")).toBe(true);
    expect(canTransition("resuming", "restored")).toBe(true);
    expect(canTransition("restored", "working")).toBe(true);
  });
  it("keeps worker terminal states terminal", () => {
    expect(canTransition("completed", "working")).toBe(false);
    expect(canTransition("cancelled", "working")).toBe(false);
  });
  it("rejects impossible idle-to-ready state", () => expect(canTransition("idle", "ready")).toBe(false));
});

describe("formatElapsed", () => {
  it("formats supervised runtime without fake precision", () => expect(formatElapsed("2026-07-12T10:00:00Z", Date.parse("2026-07-12T12:05:00Z"))).toBe("2h 05m"));
  it("labels sessions that have not started", () => expect(formatElapsed(null)).toBe("—"));
});

describe("tierRuntimeLabel", () => {
  it("leads with durable tier and keeps provider model as runtime detail", () => {
    expect(tierRuntimeLabel("strong", "fable", "high")).toBe("STRONG TIER · high · runtime Fable");
  });
  it("handles unknown runtime models without changing the tier semantic", () => {
    expect(tierRuntimeLabel("standard", "provider-next")).toBe("STANDARD TIER · runtime provider-next");
  });
});

describe("harnessLabel", () => {
  it("names the built-in harnesses", () => {
    expect(harnessLabel("claude")).toBe("Claude");
    expect(harnessLabel("codex")).toBe("Codex");
    expect(harnessLabel("opencode")).toBe("OpenCode");
    expect(harnessLabel("shell")).toBe("Shell");
  });
  it("shows an installed agent under the id it was installed by", () => {
    expect(harnessLabel("gemini")).toBe("Gemini");
    expect(harnessLabel("github-copilot-cli")).toBe("Github-copilot-cli");
  });
  it("gives a registry agent sharing a built-in name the one built-in label", () => {
    expect(harnessLabel("opencode")).toBe("OpenCode");
  });
  it("renders a harness this build cannot interpret under its own id", () => {
    expect(harnessLabel("acp:gemini")).toBe("Acp:gemini");
  });
  it("falls back only when there is no harness at all", () => {
    expect(harnessLabel(null)).toBe("Agent");
    expect(harnessLabel(undefined)).toBe("Agent");
    expect(harnessLabel("")).toBe("Agent");
  });
});

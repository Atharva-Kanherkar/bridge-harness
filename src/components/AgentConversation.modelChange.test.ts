import { describe, expect, it } from "vitest";
import { modelChangeDetail } from "./AgentConversation";

describe("modelChangeDetail", () => {
  it("names the window change and what a fresh thread carried", () => {
    expect(modelChangeDetail({
      previousWindowTokens: 200_000, windowTokens: 1_000_000, freshProviderSession: true,
      carriedContext: { summary: true, decisions: 2, filesTouched: 3, recentEntries: 4 },
    })).toBe("window 200k → 1M · fresh thread · carried summary + 2 decisions + 3 files");
  });

  it("says the thread survived a native switch", () => {
    expect(modelChangeDetail({ previousWindowTokens: 200_000, windowTokens: 200_000, freshProviderSession: false }))
      .toBe("window 200k · same thread");
  });

  it("singularizes and skips empty carries", () => {
    expect(modelChangeDetail({
      previousWindowTokens: 400_000, windowTokens: 128_000, freshProviderSession: true,
      carriedContext: { summary: false, decisions: 1, filesTouched: 0 },
    })).toBe("window 400k → 128k · fresh thread · carried 1 decision");
  });

  it("leaves entries written before window sizes existed untouched", () => {
    expect(modelChangeDetail({ freshProviderSession: true, carriedContext: { summary: true, decisions: 2 } })).toBeNull();
  });
});

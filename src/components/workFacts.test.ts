import { describe, expect, it } from "vitest";
import type { WorkFact, WorkFactAction } from "../protocol/generated/protocol";
import {
  actionLabel,
  bandFacts,
  detailIsDimmed,
  factAnnouncement,
  freshnessText,
  needsYouCount,
  stalenessClause,
  relativeTime,
  SOURCE_BY_KIND,
  SOURCE_LABEL,
} from "./workFacts";

const NOW = new Date("2026-08-19T12:00:00.000Z");

function fact(overrides: Partial<WorkFact> = {}): WorkFact {
  return {
    kind: "failed_completion_check",
    dedupeKey: "check:a-1:cargo-test",
    severity: "blocking",
    title: "cargo-test failed on Kyoto",
    detail: "A required check failed.",
    target: { kind: "completionAttempt", sessionId: "s-1", attemptId: "a-1" },
    actionableAt: "2026-08-19T11:00:00.000Z",
    observedAt: "2026-08-19T11:59:50.000Z",
    freshness: "live",
    action: { kind: "reviewCompletionCheck", sessionId: "s-1", attemptId: "a-1", checkId: "cargo-test" },
    ...overrides,
  };
}

describe("banding", () => {
  it("bands facts by severity without reordering within a band", () => {
    const facts = [
      fact({ dedupeKey: "b1", severity: "blocking", title: "first blocking" }),
      fact({ dedupeKey: "a1", severity: "attention", title: "first attention" }),
      fact({ dedupeKey: "b2", severity: "blocking", title: "second blocking" }),
      fact({ dedupeKey: "i1", severity: "info", title: "only info" }),
    ];
    const bands = bandFacts(facts);
    expect(bands.map(band => band.severity)).toEqual(["blocking", "attention", "info"]);
    // The backend already ordered these; banding must not become a second opinion.
    expect(bands[0].facts.map(item => item.title)).toEqual(["first blocking", "second blocking"]);
    expect(bands[1].facts.map(item => item.title)).toEqual(["first attention"]);
  });

  it("leaves an empty band out rather than rendering it empty", () => {
    const bands = bandFacts([fact({ severity: "attention" })]);
    expect(bands).toHaveLength(1);
    expect(bands[0].severity).toBe("attention");
  });

  it("bands nothing when there is nothing", () => {
    expect(bandFacts([])).toEqual([]);
  });
});

describe("the rail count", () => {
  it("excludes info, so it can actually reach zero", () => {
    const facts = [
      fact({ severity: "blocking" }),
      fact({ severity: "attention" }),
      fact({ severity: "info" }),
      fact({ severity: "info" }),
    ];
    expect(needsYouCount(facts)).toBe(2);
    expect(needsYouCount([fact({ severity: "info" })])).toBe(0);
    expect(needsYouCount([])).toBe(0);
  });
});

describe("relative time", () => {
  it("reads in whole units", () => {
    const at = (iso: string) => relativeTime(iso, NOW);
    expect(at("2026-08-19T11:59:50.000Z")).toBe("just now");
    expect(at("2026-08-19T11:56:00.000Z")).toBe("4 min ago");
    expect(at("2026-08-19T10:00:00.000Z")).toBe("2 h ago");
    expect(at("2026-08-16T12:00:00.000Z")).toBe("3 d ago");
  });

  it("does not render an unparseable timestamp as an invalid date", () => {
    expect(relativeTime("whenever", NOW)).toBe("at an unknown time");
    expect(relativeTime("", NOW)).toBe("at an unknown time");
  });

  it("does not report a future reading as negative", () => {
    expect(relativeTime("2026-08-19T12:05:00.000Z", NOW)).toBe("just now");
  });
});

describe("freshness", () => {
  it("says stale when a reading is stale, and says nothing special when it is live", () => {
    const live = fact({ kind: "workspace_behind_base", freshness: "live", observedAt: "2026-08-19T11:58:00.000Z" });
    const stale = fact({ kind: "workspace_behind_base", freshness: "stale", observedAt: "2026-08-19T11:36:00.000Z" });
    expect(freshnessText(live, NOW)).toBe("measured 2 min ago");
    expect(freshnessText(stale, NOW)).toBe("measured 24 min ago · stale");
    expect(freshnessText(live, NOW)).not.toContain("stale");
  });

  it("claims no numbers when nothing was measured", () => {
    const unknown = fact({ kind: "workspace_behind_base", freshness: "unknown" });
    expect(freshnessText(unknown, NOW)).toBe("not measured");
    expect(freshnessText(unknown, NOW)).not.toMatch(/\d/);
  });

  it("states the tense for a stale reading, because the backend's title does not", () => {
    // The backend says "has drifted" whether the reading is current or not, so the
    // tense tell has to be added here — as a clause, not by rewriting its sentence.
    const stale = fact({ kind: "workspace_behind_base", freshness: "stale", observedAt: "2026-08-19T11:36:00.000Z" });
    expect(stalenessClause(stale, NOW)).toBe("As of 24 min ago. These numbers describe the past.");
    expect(stalenessClause(fact({ freshness: "live" }), NOW)).toBeNull();
    // Unknown has no numbers to qualify.
    expect(stalenessClause(fact({ freshness: "unknown" }), NOW)).toBeNull();
  });

  it("dims a stale detail and only a stale detail", () => {
    expect(detailIsDimmed(fact({ freshness: "stale" }))).toBe(true);
    expect(detailIsDimmed(fact({ freshness: "live" }))).toBe(false);
    // Unknown renders no numbers at all, so there is nothing to dim.
    expect(detailIsDimmed(fact({ freshness: "unknown" }))).toBe(false);
  });

  it("uses 'seen' for a fact Bridge observed and 'measured' for one it computed", () => {
    expect(freshnessText(fact({ kind: "actionable_approval" }), NOW)).toMatch(/^seen /);
    expect(freshnessText(fact({ kind: "workspace_behind_base" }), NOW)).toMatch(/^measured /);
  });
});

describe("the accessible name", () => {
  it("carries severity, source and freshness as words, never colour", () => {
    const announcement = factAnnouncement(fact(), NOW);
    expect(announcement).toBe(
      "Blocking, Completion check: cargo-test failed on Kyoto. seen just now.",
    );
    for (const colour in { red: 1, amber: 1, destructive: 1, warning: 1 }) {
      expect(announcement.toLowerCase()).not.toContain(colour);
    }
  });

  it("says nothing is known for an unmeasured fact", () => {
    const announcement = factAnnouncement(
      fact({ kind: "workspace_behind_base", severity: "attention", freshness: "unknown", title: "Kyoto could not be measured" }),
      NOW,
    );
    expect(announcement).toContain("nothing is known");
  });
});

describe("actions", () => {
  it("labels every action variant", () => {
    // Exhaustive by construction: the switch in actionLabel has no default, so a
    // fifth variant added to the contract fails to compile rather than rendering
    // an unlabelled button.
    const actions: WorkFactAction[] = [
      { kind: "reviewCompletionCheck", sessionId: "s", attemptId: "a", checkId: "c" },
      { kind: "answerApproval", sessionId: "s", approvalSequence: 4 },
      { kind: "refreshWorkspaceBase", sessionId: "s", workspaceId: "w" },
      { kind: "refreshBaseObservation", sessionId: "s", workspaceId: "w" },
    ];
    expect(actions.map(actionLabel)).toEqual([
      "Review check",
      "Answer",
      "Fast-forward",
      "Measure again",
    ]);
  });

  it("gives the two divergence actions distinguishable labels", () => {
    // Only what this function can know. Which of the two a stale fact carries is the
    // backend's choice, and the tell that a stale row offers no fast-forward is
    // asserted on the rendered row in WorkView.test.tsx, where it is observable —
    // this assertion would pass either way and must not claim otherwise.
    expect(actionLabel({ kind: "refreshWorkspaceBase", sessionId: "s", workspaceId: "w" })).toBe("Fast-forward");
    expect(actionLabel({ kind: "refreshBaseObservation", sessionId: "s", workspaceId: "w" })).toBe("Measure again");
  });
});

describe("sources", () => {
  it("gives every fact kind its own glyph and a name in text", () => {
    const kinds: WorkFact["kind"][] = [
      "failed_completion_check",
      "blocked_worker_queue_item",
      "actionable_approval",
      "workspace_behind_base",
    ];
    const sources = kinds.map(kind => SOURCE_BY_KIND[kind]);
    expect(new Set(sources).size).toBe(kinds.length);
    for (const source of sources) {
      expect(SOURCE_LABEL[source]).toBeTruthy();
    }
  });
});

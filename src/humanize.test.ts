import { describe, expect, it } from "vitest";
import {
  LAUNCH_STEPS,
  humanizeApprovalReason,
  humanizeCheckKind,
  humanizeCheckStatus,
  humanizeForestBadge,
  humanizeLaunchFailure,
  humanizeResolution,
  humanizeSteerOutcome,
  humanizeToken,
  launchFailureStep,
  stripBridgeFences,
} from "./humanize";

describe("humanizeToken", () => {
  it("turns snake_case into sentence case", () => {
    expect(humanizeToken("owned_path_provenance_required")).toBe("Owned path provenance required");
  });

  it("splits camelCase boundaries", () => {
    expect(humanizeToken("acceptForSession")).toBe("Accept for session");
  });

  it("brings SCREAMING_CASE down to sentence case", () => {
    expect(humanizeToken("USAGE_LIMIT_EXCEEDED")).toBe("Usage limit exceeded");
  });

  it("leaves an already-plain word alone but capitalized", () => {
    expect(humanizeToken("failed")).toBe("Failed");
  });
});

describe("humanizeApprovalReason", () => {
  it("maps the known owned-path-provenance code to plain language", () => {
    const { title, detail } = humanizeApprovalReason("owned_path_provenance_required");
    expect(title).toBe("These paths weren't pre-approved by you.");
    expect(detail).toContain("one-time authorization");
  });

  it("degrades an unknown code to sentence case instead of raw", () => {
    const { title, detail } = humanizeApprovalReason("some_future_policy_code");
    expect(title).toBe("Some future policy code");
    expect(title).not.toContain("_");
    expect(detail).toBeUndefined();
  });
});

describe("humanizeResolution", () => {
  it("maps acceptForSession to the session-scoped phrase", () => {
    expect(humanizeResolution("acceptForSession")).toBe("Allowed for this session");
  });

  it("maps accept/allow variants to a single once-off phrase", () => {
    expect(humanizeResolution("accept")).toBe("Allowed once");
    expect(humanizeResolution("accepted")).toBe("Allowed once");
    expect(humanizeResolution("allow")).toBe("Allowed once");
  });

  it("maps decline/deny variants to Declined", () => {
    expect(humanizeResolution("decline")).toBe("Declined");
    expect(humanizeResolution("declined")).toBe("Declined");
    expect(humanizeResolution("deny")).toBe("Declined");
  });

  it("degrades an unrecognized decision token to sentence case", () => {
    expect(humanizeResolution("auto_expired")).toBe("Auto expired");
  });
});

describe("humanizeCheckKind and humanizeCheckStatus", () => {
  it("fixes every underscore in a check kind", () => {
    expect(humanizeCheckKind("build_and_test")).toBe("Build and test");
    expect(humanizeCheckKind("lint_only_fast_path")).not.toContain("_");
  });

  it("renders a check status in sentence case, not raw uppercase", () => {
    expect(humanizeCheckStatus("STRONG_TIER")).toBe("Strong tier");
    expect(humanizeCheckStatus("PASSED")).toBe("Passed");
  });
});

describe("humanizeSteerOutcome", () => {
  it("maps known outcome flags to plain words", () => {
    expect(humanizeSteerOutcome("orchestrator_not_told")).toBe("Sent straight to the worker");
    expect(humanizeSteerOutcome("not_delivered")).toBe("Not delivered");
    expect(humanizeSteerOutcome("at_next_step")).toBe("Delivered at the next step");
  });

  it("is tolerant of spaced or differently-cased flags", () => {
    expect(humanizeSteerOutcome("AT NEXT STEP")).toBe("Delivered at the next step");
  });

  it("degrades an unknown flag to sentence case", () => {
    expect(humanizeSteerOutcome("queued_for_retry")).toBe("Queued for retry");
  });
});

describe("humanizeForestBadge", () => {
  it("renders no badge at all for the ordinary durable state", () => {
    expect(humanizeForestBadge("durable")).toBeUndefined();
  });

  it("renders other lifecycle states as plain words", () => {
    expect(humanizeForestBadge("compacted")).toBe("Compacted");
    expect(humanizeForestBadge("checkpoint_pending")).toBe("Checkpoint pending");
  });
});

describe("stripBridgeFences", () => {
  it("removes a closed bridge-delegate fence", () => {
    const text = "Delegating now.\n```bridge-delegate\n{\"objective\":\"add rotation\"}\n```\nWatch the panel below.";
    expect(stripBridgeFences(text)).toBe("Delegating now.\nWatch the panel below.");
  });

  it("removes a closed bridge-worker-result fence", () => {
    const text = "Finished the work.\n```bridge-worker-result\n{\"schemaVersion\":1,\"status\":\"completed\"}\n```\nReview the summary.";
    expect(stripBridgeFences(text)).toBe("Finished the work.\nReview the summary.");
  });

  it("removes any bridge-* tag, not just known ones", () => {
    const text = "before\n```bridge-peek\n{\"foo\":1}\n```\nafter";
    expect(stripBridgeFences(text)).toBe("before\nafter");
    const steer = "before\n```bridge-steer\n{\"foo\":1}\n```\nafter";
    expect(stripBridgeFences(steer)).toBe("before\nafter");
  });

  it("drops an unclosed fence streamed at the end of the text", () => {
    const text = "Delegating now.\n```bridge-delegate\n{\"objective\": \"add rot";
    expect(stripBridgeFences(text)).toBe("Delegating now.");
  });

  it("is tolerant of whitespace around the fence markers", () => {
    const text = "before\n   ```   bridge-delegate  \n{\"a\":1}\n   ```   \nafter";
    expect(stripBridgeFences(text)).toBe("before\nafter");
  });

  it("is tolerant of CRLF line endings", () => {
    const text = "before\r\n```bridge-delegate\r\n{\"a\":1}\r\n```\r\nafter";
    expect(stripBridgeFences(text)).toBe("before\nafter");
  });

  it("leaves text with no fence completely intact", () => {
    expect(stripBridgeFences("just plain prose")).toBe("just plain prose");
  });

  it("does not touch an ordinary code fence with no bridge- tag", () => {
    const text = "before\n```json\n{\"a\":1}\n```\nafter";
    expect(stripBridgeFences(text)).toBe(text);
  });

  it("collapses to empty when the whole message is one fence", () => {
    const text = "```bridge-worker-result\n{\"a\":1}\n```";
    expect(stripBridgeFences(text)).toBe("");
  });
});

describe("worker launch failures", () => {
  it("maps every backend phase onto a launch step and leaves unknown phases off the track", () => {
    expect(LAUNCH_STEPS).toEqual(["Policy", "Route", "Workspace", "Start"]);
    for (const phase of ["parent", "policy", "capability", "settlement"]) expect(launchFailureStep(phase)).toBe(0);
    for (const phase of ["routing", "model_resolution", "verification_target"]) expect(launchFailureStep(phase)).toBe(1);
    expect(launchFailureStep("worktree")).toBe(2);
    expect(launchFailureStep("spawn")).toBe(3);
    expect(launchFailureStep("something_new")).toBe(-1);
    expect(launchFailureStep("")).toBe(-1);
  });

  it("leads with the cause in plain words and keeps the remedy as a hint", () => {
    expect(humanizeLaunchFailure(
      "opencode cannot run a read_only worker, so this delegation was not started. Re-delegate with a compatible harness (claude, codex) or change the write mode.",
    )).toEqual({
      summary: "OpenCode cannot run a read-only worker.",
      hint: "Re-delegate with a compatible harness (Claude, Codex) or change the write mode.",
    });
    expect(humanizeLaunchFailure("router schema mismatch")).toEqual({ summary: "Router schema mismatch" });
    expect(humanizeLaunchFailure("Could not create worktree: fatal: bad ref\n  at git worktree add")).toEqual({
      summary: "Could not create worktree: fatal: bad ref",
    });
    // Ordinary words that happen to be harness ids elsewhere stay as written.
    expect(humanizeLaunchFailure("the cursor moved").summary).toBe("The cursor moved");
    expect(humanizeLaunchFailure("").summary).toBe("");
    // Literal wire values in backticks are what the agent must write: untouched.
    expect(humanizeLaunchFailure("unknown variant `none`, expected one of `read_only`, `shared` for codex").summary)
      .toBe("Unknown variant `none`, expected one of `read_only`, `shared` for Codex");
  });
});

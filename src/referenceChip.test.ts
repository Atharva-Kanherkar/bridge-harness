import { describe, expect, it, vi } from "vitest";
import {
  findReferences,
  referenceAlias,
  referencePullText,
  toPublicAlias,
} from "./referenceChip";
import type { ResolveReferenceResult } from "./protocol/generated/protocol";

type SessionRef = Extract<ResolveReferenceResult, { kind: "session" }>;
const sessionRef = (overrides: Partial<SessionRef> = {}): SessionRef => ({
  kind: "session",
  sessionId: "11111111-aaaa-4aaa-8aaa-aaaaaaaaaaaa",
  label: "Kyoto",
  harness: "codex",
  workspaceId: null,
  parentSessionId: "22222222-bbbb-4bbb-8bbb-bbbbbbbbbbbb",
  depth: 1,
  restorationMode: "checkpoint_restored",
  continuationFidelity: "projected_at_boundary",
  activeEntryId: null,
  latestCheckpointEntryId: "chk-1",
  updatedAt: "now",
  authorized: true,
  ...overrides,
});

describe("reference chips", () => {
  it("mints the public alias from the first 8 hex chars of the uuid", () => {
    expect(toPublicAlias("11111111-aaaa-4aaa-8aaa-aaaaaaaaaaaa")).toBe("brio_11111111");
  });

  it("detects alias and mention tokens in prose, deduplicated", () => {
    expect(findReferences("see brio_11111111 and @session:brio_22222222 and brio_11111111 again"))
      .toEqual(["brio_11111111", "@session:brio_22222222"]);
    expect(findReferences("plain text with nothing")).toEqual([]);
  });

  it("derives the alias from mention spellings", () => {
    expect(referenceAlias("@session:brio_11111111")).toBe("brio_11111111");
    expect(referenceAlias("@session:33333333-cccc-4ccc-8ccc-cccccccccccc")).toBe("brio_33333333");
    expect(referenceAlias("brio_11111111")).toBe("brio_11111111");
  });

  it("pulls a checkpoint summary into the draft for sessions", () => {
    expect(referencePullText(sessionRef({ latestCheckpointEntryId: "chk-1" })))
      .toContain("checkpoint present");
    expect(referencePullText(sessionRef({ latestCheckpointEntryId: null })))
      .toContain("checkpoint none");
  });

  it("pulls entry summaries as quotes and unknown references pull nothing", async () => {
    const entry: ResolveReferenceResult = {
      kind: "entry",
      sessionId: "s",
      entryId: "44444444-4444-4444-8444-444444444444",
      entryKind: "user.message",
      sequence: 1,
      summary: "A question about the rail",
      createdAt: "now",
      authorized: true,
    };
    expect(referencePullText(entry)).toBe("> A question about the rail");
    expect(referencePullText({ kind: "unknown", authorized: false })).toBe("");
    const resolved = sessionRef();
    
    expect(resolved.parentSessionId).toBeTruthy();
  });
});
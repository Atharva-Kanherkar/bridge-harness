import { describe, expect, it, vi } from "vitest";
import {
  chipDetail,
  findReferences,
  insertMention,
  mentionToken,
  referenceAlias,
  removeReferenceToken,
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

  it("tells the reader that history attaches on send instead of naming a restoration mode", () => {
    expect(chipDetail(sessionRef())).toBe("codex · fork · history attaches on send");
    expect(chipDetail(sessionRef({ parentSessionId: null }))).toBe("codex · history attaches on send");
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
    expect(chipDetail(entry)).toBe("user.message #1 · attaches on send");
    expect(chipDetail({ kind: "unknown", authorized: false })).toBe("no such chat — sent as plain text");
  });

  it("inserts a mention once, spaced from the surrounding draft", () => {
    const id = "11111111-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
    expect(mentionToken(id)).toBe("@session:brio_11111111");
    expect(insertMention("", id)).toBe("@session:brio_11111111 ");
    expect(insertMention("continue", id)).toBe("continue @session:brio_11111111 ");
    expect(insertMention("continue ", id)).toBe("continue @session:brio_11111111 ");
    expect(insertMention("see @session:brio_11111111 now", id)).toBe("see @session:brio_11111111 now");
  });

  it("removes a reference and one adjacent space without touching other whitespace", () => {
    const code = "def f():\n    if x:\n        return 1";
    expect(removeReferenceToken(`brio_11111111 ${code}`, "brio_11111111")).toBe(code);
    expect(removeReferenceToken(`${code} brio_11111111`, "brio_11111111")).toBe(code);
    expect(removeReferenceToken("compare with brio_11111111", "brio_11111111")).toBe("compare with");
    expect(removeReferenceToken("a brio_11111111 b brio_11111111", "brio_11111111")).toBe("a b");
    expect(removeReferenceToken("untouched  text", "brio_11111111")).toBe("untouched  text");
  });
});

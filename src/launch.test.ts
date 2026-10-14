import { describe, expect, it } from "vitest";
import { approvalTarget, parseLaunch } from "./launch";

describe("parseLaunch", () => {
  it("accepts every shape the shell sends", () => {
    expect(parseLaunch({ kind: "open", sessionId: "s1" })).toEqual({ kind: "open", sessionId: "s1" });
    expect(parseLaunch({ kind: "new-chat", project: "/home/me/Work" })).toEqual({ kind: "new-chat", project: "/home/me/Work", prompt: null });
    expect(parseLaunch({ kind: "new-chat", project: null, prompt: "  fix it  " })).toEqual({ kind: "new-chat", project: null, prompt: "fix it" });
    expect(parseLaunch({ kind: "quick" })).toEqual({ kind: "quick" });
    expect(parseLaunch({ kind: "approvals" })).toEqual({ kind: "approvals" });
    expect(parseLaunch({ kind: "mission-control" })).toEqual({ kind: "mission-control" });
  });

  it("rejects anything else", () => {
    expect(parseLaunch(null)).toBeNull();
    expect(parseLaunch({ kind: "open" })).toBeNull();
    expect(parseLaunch({ kind: "open", sessionId: "" })).toBeNull();
    expect(parseLaunch({ kind: "omarchy", action: "setup" })).toBeNull();
    expect(parseLaunch("open")).toBeNull();
  });
});

describe("approvalTarget", () => {
  it("is the first waiting session", () => {
    expect(approvalTarget([{ id: "a", status: "working" }, { id: "b", status: "waiting" }, { id: "c", status: "waiting" }])).toBe("b");
    expect(approvalTarget([{ id: "a", status: "working" }])).toBeNull();
  });
});

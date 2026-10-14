import { describe, expect, it } from "vitest";
import { parseNotificationAction, pendingApprovalEventId } from "./nativeNotify";

describe("pendingApprovalEventId", () => {
  it("is the newest unresolved approval", () => {
    expect(pendingApprovalEventId([
      { sequence: 3, kind: "approval.requested" },
      { sequence: 4, kind: "approval.resolved", data: { requestEventId: 3 } },
      { sequence: 9, kind: "approval.requested" },
      { sequence: 10, kind: "tool.started" },
    ])).toBe(9);
  });

  it("is null when everything is resolved or nothing was asked", () => {
    expect(pendingApprovalEventId([{ sequence: 3, kind: "approval.requested" }, { sequence: 4, kind: "approval.resolved", data: { requestEventId: 3 } }])).toBeNull();
    expect(pendingApprovalEventId([{ sequence: 1, kind: "message.completed" }])).toBeNull();
    expect(pendingApprovalEventId([{ sequence: 0, kind: "approval.requested" }])).toBeNull();
  });
});

describe("parseNotificationAction", () => {
  it("accepts the shell's actions", () => {
    expect(parseNotificationAction({ action: "allow-once", sessionId: "s", eventId: 9 })).toEqual({ action: "allow-once", sessionId: "s", eventId: 9 });
    expect(parseNotificationAction({ action: "decline", sessionId: "s", eventId: 9 })).toEqual({ action: "decline", sessionId: "s", eventId: 9 });
    expect(parseNotificationAction({ action: "open", sessionId: "s", eventId: null })).toEqual({ action: "open", sessionId: "s", eventId: null });
  });

  it("refuses to resolve without an approval to resolve", () => {
    expect(parseNotificationAction({ action: "allow-once", sessionId: "s", eventId: null })).toBeNull();
    expect(parseNotificationAction({ action: "accept-all", sessionId: "s", eventId: 9 })).toBeNull();
    expect(parseNotificationAction({ action: "open" })).toBeNull();
  });
});

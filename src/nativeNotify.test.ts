import { describe, expect, it } from "vitest";
import { parseNotificationAction } from "./nativeNotify";

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

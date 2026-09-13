import { describe, expect, it } from "vitest";
import type { ConnectorDescriptor, ConnectorInboxItem } from "./protocol/generated/protocol";
import {
  emptyStateFor,
  hasAttention,
  isDegraded,
  itemSubtitle,
  primaryConnector,
  reduceToasts,
  relativeTime,
  toastFromArrival,
  toastKey,
  unavailableHint,
  unreadCount,
} from "./connectorSurface";

const arrival = {
  family: "slack",
  itemKey: "slack:D0BV4LADFGB:1757000000.000100",
  headline: "Nina Alvarez sent you a direct message",
  channelLabel: "Nina Alvarez",
  author: "Nina Alvarez",
};

function item(overrides: Partial<ConnectorInboxItem> = {}): ConnectorInboxItem {
  return {
    itemKey: arrival.itemKey,
    family: "slack",
    channelId: "D0BV4LADFGB",
    channelLabel: "Nina Alvarez",
    author: "Nina Alvarez",
    kind: "directMessage",
    text: "can you look at the release checklist before standup?",
    permalink: null,
    receivedAt: "2026-09-13T09:14:00Z",
    state: "rendered",
    card: null,
    renderRejection: null,
    resolution: null,
    ...overrides,
  };
}

describe("toast identity", () => {
  it("keys on the item, so an arrival and its card are one notification", () => {
    expect(toastKey(arrival)).toBe(arrival.itemKey);
    const arrived = reduceToasts([], { type: "arrived", payload: arrival });
    const upgraded = reduceToasts(arrived, {
      type: "card",
      payload: {
        family: "slack",
        itemKey: arrival.itemKey,
        headline: "Nina wants the release checklist reviewed",
        harnessRendered: true,
      },
    });
    // One toast, sharper wording — not two toasts for one DM.
    expect(upgraded).toHaveLength(1);
    expect(upgraded[0].headline).toBe("Nina wants the release checklist reviewed");
    expect(upgraded[0].settled).toBe(true);
  });

  it("starts unsettled so the card can land later", () => {
    expect(toastFromArrival(arrival).settled).toBe(false);
    expect(toastFromArrival(arrival).detail).toBe("Nina Alvarez");
  });

  it("ignores a redelivered arrival", () => {
    const once = reduceToasts([], { type: "arrived", payload: arrival });
    expect(reduceToasts(once, { type: "arrived", payload: arrival })).toHaveLength(1);
  });

  it("stacks genuinely different messages", () => {
    const first = reduceToasts([], { type: "arrived", payload: arrival });
    const second = reduceToasts(first, {
      type: "arrived",
      payload: { ...arrival, itemKey: "slack:C0A469VRHMH:1757000100.000200" },
    });
    expect(second).toHaveLength(2);
  });

  it("drops a toast once its item is answered", () => {
    const shown = reduceToasts([], { type: "arrived", payload: arrival });
    const resolved = reduceToasts(shown, {
      type: "resolved",
      payload: { family: "slack", itemKey: arrival.itemKey, succeeded: true },
    });
    expect(resolved).toHaveLength(0);
  });

  it("dismisses only the toast that was dismissed", () => {
    const two = reduceToasts(reduceToasts([], { type: "arrived", payload: arrival }), {
      type: "arrived",
      payload: { ...arrival, itemKey: "slack:C1:2.0" },
    });
    const left = reduceToasts(two, { type: "dismiss", key: arrival.itemKey });
    expect(left.map(toast => toast.itemKey)).toEqual(["slack:C1:2.0"]);
  });

  it("ignores a card for an item with no toast up", () => {
    const result = reduceToasts([], {
      type: "card",
      payload: { family: "slack", itemKey: "slack:C9:9.9", headline: "x", harnessRendered: true },
    });
    expect(result).toEqual([]);
  });
});

describe("unread accounting", () => {
  it("counts unresolved items only", () => {
    expect(
      unreadCount([item(), item({ itemKey: "b", state: "pending" }), item({ itemKey: "c", state: "resolved" })]),
    ).toBe(2);
  });

  it("treats a degraded poll as something worth a badge even with nothing unread", () => {
    expect(hasAttention({ unreadCount: 0, poll: [] })).toBe(false);
    expect(
      hasAttention({
        unreadCount: 0,
        poll: [{ family: "slack", lastAttemptAt: "x", lastSuccessAt: null, degraded: "timed out" }],
      }),
    ).toBe(true);
  });
});

describe("the empty state never lies about a failed read", () => {
  it("says caught up only after a successful check found nothing", () => {
    const state = emptyStateFor({
      items: [],
      poll: [{ family: "slack", lastAttemptAt: "t", lastSuccessAt: "t", degraded: null }],
    });
    expect(state?.tone).toBe("caught-up");
  });

  it("says it could not read rather than showing an empty inbox", () => {
    const state = emptyStateFor({
      items: [],
      poll: [{ family: "slack", lastAttemptAt: "t", lastSuccessAt: "earlier", degraded: "provider timed out" }],
    });
    expect(state?.tone).toBe("degraded");
    expect(state?.detail).toContain("timed out");
    // The most misleading thing this surface could do is claim calm here.
    expect(state?.title).not.toContain("caught up");
  });

  it("distinguishes 'not checked yet' from 'checked and empty'", () => {
    const state = emptyStateFor({
      items: [],
      poll: [{ family: "slack", lastAttemptAt: null, lastSuccessAt: null, degraded: null }],
    });
    expect(state?.tone).toBe("waiting");
  });

  it("shows no empty state at all when there are items", () => {
    expect(emptyStateFor({ items: [item()], poll: [] })).toBeNull();
  });

  it("reports degradation per family", () => {
    const poll = [{ family: "slack", lastAttemptAt: "t", lastSuccessAt: null, degraded: "down" }];
    expect(isDegraded(poll, "slack")).toBe(true);
    expect(isDegraded(poll, "gmail")).toBe(false);
  });
});

describe("connector selection and hints", () => {
  const descriptor = (overrides: Partial<ConnectorDescriptor>): ConnectorDescriptor => ({
    family: "slack",
    displayName: "Slack",
    server: "claude.ai Slack",
    harness: "claude",
    hasInbox: true,
    available: true,
    reason: null,
    explanation: null,
    ...overrides,
  });

  it("prefers an available connector", () => {
    const chosen = primaryConnector([
      descriptor({ family: "gmail", displayName: "Gmail", available: false, reason: "authRequired" }),
      descriptor({}),
    ]);
    expect(chosen?.family).toBe("slack");
  });

  it("falls back to a family this build has an inbox for, so the pane can explain itself", () => {
    const chosen = primaryConnector([
      descriptor({ family: "gmail", displayName: "Gmail", hasInbox: false, available: false, reason: "noResolver" }),
      descriptor({ available: false, reason: "notConfigured" }),
    ]);
    // No family is named in the implementation — "has an inbox here" is the
    // property that makes a connector worth explaining.
    expect(chosen?.family).toBe("slack");
  });

  it("names no family of its own when nothing has an inbox", () => {
    const chosen = primaryConnector([descriptor({ family: "gmail", displayName: "Gmail", hasInbox: false, available: false })]);
    expect(chosen?.family).toBe("gmail");
  });

  it("passes the host's explanation through, because the fix is usually not in Bridge", () => {
    const hint = unavailableHint(
      descriptor({ available: false, reason: "authRequired", explanation: "Sign in from your harness." }),
    );
    expect(hint).toBe("Sign in from your harness.");
  });

  it("still says something when a build shipped no explanation", () => {
    expect(unavailableHint(descriptor({ available: false, explanation: null }))).toContain("Slack");
  });
});

describe("relative time", () => {
  const now = new Date("2026-09-13T10:00:00Z");
  it("reads a fresh message as now and 60s as 1m", () => {
    expect(relativeTime("2026-09-13T09:59:30Z", now)).toBe("now");
    expect(relativeTime("2026-09-13T09:59:00Z", now)).toBe("1m");
  });

  it("steps through minutes, hours, and days", () => {
    expect(relativeTime("2026-09-13T09:01:00Z", now)).toBe("59m");
    expect(relativeTime("2026-09-13T09:00:00Z", now)).toBe("1h");
    expect(relativeTime("2026-09-12T10:00:00Z", now)).toBe("1d");
  });

  it("falls back to a date past a week and never crashes on junk", () => {
    expect(relativeTime("2026-08-01T10:00:00Z", now)).not.toMatch(/[hmd]$/);
    expect(relativeTime("not a date", now)).toBe("");
  });

  it("never renders a negative age from a clock skew", () => {
    expect(relativeTime("2026-09-13T10:05:00Z", now)).toBe("now");
  });
});

describe("item subtitle", () => {
  const now = new Date("2026-09-13T09:20:00Z");
  it("names the kind, the channel, and the age", () => {
    expect(itemSubtitle(item(), now)).toBe("DM · Nina Alvarez · 6m");
    expect(itemSubtitle(item({ kind: "mention", channelLabel: "#eng-alerts" }), now)).toContain("Mention · #eng-alerts");
    expect(itemSubtitle(item({ kind: "threadReply" }), now)).toContain("Thread");
  });
});

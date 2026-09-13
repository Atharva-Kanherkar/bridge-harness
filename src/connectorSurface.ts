import type {
  ConnectorDescriptor,
  ConnectorInboxItem,
  ConnectorInboxResult,
  ConnectorPollStatus,
} from "./protocol/generated/protocol";

// Pure logic for the connector surface: toast identity, notification wording,
// and the two rules that decide what the pane is allowed to claim. Kept out of
// the components so "a failed read is not an empty inbox" and "a toast upgrades
// in place" are assertable without a DOM.

/** The `connectors/item_arrived` wire payload. */
export type ConnectorItemArrivedPayload = {
  family: string;
  itemKey: string;
  headline: string;
  channelLabel: string;
  author: string;
};

/** The `connectors/card_ready` wire payload. */
export type ConnectorCardReadyPayload = {
  family: string;
  itemKey: string;
  headline: string;
  harnessRendered: boolean;
};

/** The `connectors/item_resolved` wire payload. */
export type ConnectorItemResolvedPayload = {
  family: string;
  itemKey: string;
  succeeded: boolean;
};

/**
 * The toast's identity. Deliberately the item key alone and not the headline:
 * an arrival and the card that follows it are the same notification at two
 * levels of polish, so keying on text would stack a second toast the moment the
 * card sharpened the wording.
 */
export function toastKey(payload: { itemKey: string }): string {
  return payload.itemKey;
}

export type ConnectorToast = {
  key: string;
  itemKey: string;
  family: string;
  headline: string;
  detail: string;
  /** False until the harness-rendered card lands. Drives the shimmer. */
  settled: boolean;
};

export function toastFromArrival(payload: ConnectorItemArrivedPayload): ConnectorToast {
  return {
    key: toastKey(payload),
    itemKey: payload.itemKey,
    family: payload.family,
    headline: payload.headline,
    detail: payload.channelLabel,
    settled: false,
  };
}

/**
 * Fold an event into the toast stack.
 *
 * An arrival adds. A card *upgrades the existing toast in place* rather than
 * pushing a new one — the user is being told about one message, and replacing
 * the card would make a single DM look like two. A resolution removes: an item
 * you already replied to is not still asking for you.
 */
export function reduceToasts(
  toasts: ConnectorToast[],
  event:
    | { type: "arrived"; payload: ConnectorItemArrivedPayload }
    | { type: "card"; payload: ConnectorCardReadyPayload }
    | { type: "resolved"; payload: ConnectorItemResolvedPayload }
    | { type: "dismiss"; key: string },
): ConnectorToast[] {
  switch (event.type) {
    case "arrived": {
      const next = toastFromArrival(event.payload);
      // A redelivered arrival (reconnect, mock replay) is the same notification.
      if (toasts.some(toast => toast.key === next.key)) return toasts;
      return [...toasts, next];
    }
    case "card":
      return toasts.map(toast =>
        toast.itemKey === event.payload.itemKey
          ? { ...toast, headline: event.payload.headline, settled: true }
          : toast,
      );
    case "resolved":
      return toasts.filter(toast => toast.itemKey !== event.payload.itemKey);
    case "dismiss":
      return toasts.filter(toast => toast.key !== event.key);
  }
}

/** Unresolved items only — a read message is not an unread one. */
export function unreadCount(items: ConnectorInboxItem[]): number {
  return items.filter(item => item.state !== "resolved").length;
}

export function isUnresolved(item: ConnectorInboxItem): boolean {
  return item.state !== "resolved";
}

/**
 * What the pane says when it has nothing to show.
 *
 * The distinction this encodes is the whole point: an inbox that could not be
 * read is not an inbox that is empty, and showing "You're all caught up" over a
 * failed poll is the single most misleading thing this surface could do.
 */
export function emptyStateFor(result: Pick<ConnectorInboxResult, "items" | "poll">): {
  tone: "caught-up" | "degraded" | "waiting";
  title: string;
  detail: string;
} | null {
  if (result.items.length) return null;
  const degraded = result.poll.find(status => status.degraded);
  if (degraded) {
    return {
      tone: "degraded",
      title: "Couldn’t read your inbox",
      detail: degraded.degraded ?? "The last check failed.",
    };
  }
  if (!result.poll.some(status => status.lastSuccessAt)) {
    return { tone: "waiting", title: "Checking…", detail: "Bridge hasn’t completed a check yet." };
  }
  return { tone: "caught-up", title: "You’re all caught up", detail: "Nothing new since the last check." };
}

/** The one connector family this build surfaces, when the harness has it. */
export function primaryConnector(connectors: ConnectorDescriptor[]): ConnectorDescriptor | undefined {
  return connectors.find(connector => connector.available) ?? connectors.find(connector => connector.family === "slack");
}

/** Whether anything is worth showing a dock badge for. */
export function hasAttention(result: Pick<ConnectorInboxResult, "unreadCount" | "poll">): boolean {
  return result.unreadCount > 0 || result.poll.some(status => status.degraded);
}

// ── Relative time ────────────────────────────────────────────────────────────
// Slack timestamps are the item's own; a message is "now" for a minute, then
// minutes, then hours, then a date. Boundaries are inclusive-below so 60
// seconds reads as "1m" and never as "60s".

export function relativeTime(iso: string, now: Date = new Date()): string {
  const then = Date.parse(iso);
  if (Number.isNaN(then)) return "";
  const seconds = Math.max(0, Math.round((now.getTime() - then) / 1000));
  if (seconds < 60) return "now";
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes}m`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h`;
  const days = Math.floor(hours / 24);
  if (days < 7) return `${days}d`;
  return new Date(then).toLocaleDateString(undefined, { month: "short", day: "numeric" });
}

/** The line under a card's headline. */
export function itemSubtitle(item: ConnectorInboxItem, now?: Date): string {
  const kind = item.kind === "directMessage" ? "DM" : item.kind === "mention" ? "Mention" : "Thread";
  const when = relativeTime(item.receivedAt, now);
  return when ? `${kind} · ${item.channelLabel} · ${when}` : `${kind} · ${item.channelLabel}`;
}

/**
 * The sentence the connector strip shows when a family is unavailable.
 *
 * Bridge holds no connector credential, so for every reason except "unknown"
 * the fix lives in the harness. The host already writes that sentence; this only
 * guards against a build where it did not.
 */
export function unavailableHint(connector: ConnectorDescriptor): string {
  return connector.explanation ?? `${connector.displayName} is not available.`;
}

/** True when the degraded badge should show for this family. */
export function isDegraded(poll: ConnectorPollStatus[], family: string): boolean {
  return poll.some(status => status.family === family && !!status.degraded);
}

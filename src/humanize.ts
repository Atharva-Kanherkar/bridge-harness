// Shared humanization layer for chat-facing strings.
//
// Every user-visible label in the transcript must pass through here rather
// than falling back to a wire enum, hash, camelCase decision token, or
// envelope prose. Both the live reducer and the durable projection use the
// same functions so a transcript reads identically before and after a reload.

/** Approval policy reason codes → plain language shown on approval cards. */
export function humanizeApprovalReason(code: string): {
  title: string;
  detail?: string;
} {
  switch (code) {
    case "owned_path_provenance_required":
      return {
        title: "These paths weren't pre-approved by you.",
        detail:
          "The worker proposed them itself, so Bridge needs a one-time authorization.",
      };
    default:
      return { title: humanizeToken(code) };
  }
}

/** Resolved approval/permission decisions → human outcome labels. */
export function humanizeResolution(status: string): string {
  switch (status) {
    case "accept":
    case "accepted":
    case "allow":
      return "Allowed once";
    case "acceptForSession":
      return "Allowed for this session";
    case "decline":
    case "declined":
    case "deny":
      return "Declined";
    default:
      return humanizeToken(status);
  }
}

/** Verification check kinds (snake_case wire values) → readable labels. */
export function humanizeCheckKind(kind: string): string {
  return humanizeToken(kind);
}

/** Verification check statuses → readable labels (not raw UPPERCASE). */
export function humanizeCheckStatus(status: string): string {
  return humanizeToken(status);
}

/** Steer delivery outcomes → plain words instead of caps flags. */
export function humanizeSteerOutcome(flag: string): string {
  switch (flag.toLowerCase().replaceAll(" ", "_")) {
    case "orchestrator_not_told":
      return "Sent straight to the worker";
    case "not_delivered":
      return "Not delivered";
    case "at_next_step":
      return "Delivered at the next step";
    default:
      return humanizeToken(flag);
  }
}

/** Forest lifecycle badges ("durable" etc.) → plain phrases or nothing. */
export function humanizeForestBadge(status: string): string | undefined {
  if (status === "durable") return undefined;
  return humanizeToken(status);
}

/**
 * Remove every `bridge-*` control fence (bridge-delegate, bridge-peek,
 * bridge-steer, bridge-worker-result, …) from assistant prose. Applied on
 * both the live and durable projection paths.
 */
export function stripBridgeFences(text: string): string {
  // Refined by the projection work; conservative fallback keeps text intact
  // when no fence is present.
  return text.replace(
    /```[ \t]*bridge-[A-Za-z0-9_-]*[^\n]*\n[\s\S]*?```[ \t]*\n?/g,
    "",
  );
}

/** Lowercase snake_case / camelCase / SCREAMING tokens → sentence words. */
export function humanizeToken(token: string): string {
  const spaced = token
    .replaceAll("_", " ")
    .replace(/([a-z])([A-Z])/g, "$1 $2")
    .toLowerCase()
    .trim();
  return spaced.charAt(0).toUpperCase() + spaced.slice(1);
}

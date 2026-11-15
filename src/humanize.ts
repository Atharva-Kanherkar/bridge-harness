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
 * both the live and durable projection paths, so an envelope stripped while
 * streaming does not regrow after a reload.
 *
 * Line-based rather than one regex: a fence still streaming in has no
 * closing ``` yet, and a single non-multiline pass either fails to match an
 * unclosed fence (leaking the raw envelope) or, with a greedy `[\s\S]*`,
 * swallows real prose past it. Scanning line by line lets an open-but-not-yet-
 * closed fence drop straight through to the end of the text instead.
 */
export function stripBridgeFences(text: string): string {
  const lines = text.split(/\r\n|\r|\n/);
  const kept: string[] = [];
  let index = 0;
  while (index < lines.length) {
    const trimmed = lines[index].trim();
    const opensFence = /^`{3,}[ \t]*bridge-[A-Za-z0-9_-]*/i.test(trimmed);
    if (opensFence) {
      let closing = -1;
      for (let cursor = index + 1; cursor < lines.length; cursor++) {
        if (lines[cursor].trim().startsWith("```")) {
          closing = cursor;
          break;
        }
      }
      // No closing fence yet: the rest of the streamed text is still inside
      // the envelope, so drop it all rather than let raw JSON leak through.
      index = closing >= 0 ? closing + 1 : lines.length;
      continue;
    }
    kept.push(lines[index]);
    index += 1;
  }
  return kept.join("\n").trim();
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

/** The steps a worker launch passes through, in order, as the failure card draws them. */
export const LAUNCH_STEPS = ["Policy", "Route", "Workspace", "Start"] as const;

/**
 * The launch step a `delegation.rejected` phase stopped at, as an index into
 * {@link LAUNCH_STEPS}, or -1 for a phase this build does not know (the card
 * then draws no track rather than a wrong one).
 */
export function launchFailureStep(phase: string): number {
  switch (phase) {
    case "parent":
    case "policy":
    case "capability":
    case "settlement":
      return 0;
    case "routing":
    case "model_resolution":
    case "verification_target":
      return 1;
    case "worktree":
      return 2;
    case "spawn":
      return 3;
    default:
      return -1;
  }
}

// Only ids that are never ordinary English words: "cursor" and "grok" are, so
// they stay as written.
const LAUNCH_HARNESS_IDS = /\b(claude|codex|opencode)\b/g;
const LAUNCH_HARNESS_LABELS: Record<string, string> = { claude: "Claude", codex: "Codex", opencode: "OpenCode" };
const LAUNCH_SANDBOX_MODES = /\b(read_only|workspace_write|danger_full_access)\b/g;
// The card's title already says no worker started.
const NOT_STARTED = /,?\s*so this delegation was not started(?=[.;:]|$)/i;

/**
 * A worker launch failure reason → a sentence to lead with and the rest as
 * guidance. Display only: wire ids become names (`opencode` → OpenCode,
 * `read_only` → read-only) and the card keeps the raw reason to copy.
 */
export function humanizeLaunchFailure(reason: string): { summary: string; hint?: string } {
  const firstLine = reason.trim().split(/\r?\n/, 1)[0] ?? "";
  // Backticked spans are literal wire values ("expected one of `read_only`")
  // and stay exactly as the agent must write them; only prose is humanized.
  const readable = firstLine
    .split(/(`[^`]*`)/)
    .map(part => part.startsWith("`") ? part : part
      .replace(LAUNCH_SANDBOX_MODES, mode => mode.replaceAll("_", "-"))
      .replace(LAUNCH_HARNESS_IDS, id => LAUNCH_HARNESS_LABELS[id] ?? id))
    .join("")
    .replace(NOT_STARTED, "");
  const sentence = /^(.+?[.!?])\s+(\S.*)$/.exec(readable);
  const [summary, hint] = sentence ? [sentence[1], sentence[2]] : [readable, undefined];
  const capitalized = summary.charAt(0).toUpperCase() + summary.slice(1);
  return hint ? { summary: capitalized, hint } : { summary: capitalized };
}

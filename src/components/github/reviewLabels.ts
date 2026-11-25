import type { ReviewJobStatus, ReviewSeverity } from "../../protocol/generated/protocol";

// Words and colours the review pane, its draft and its toasts share.

const HARNESS_LABELS: Record<string, string> = { claude: "Claude", codex: "Codex", opencode: "OpenCode" };

export const harnessLabel = (id: string) => HARNESS_LABELS[id] ?? id.charAt(0).toUpperCase() + id.slice(1);

export const SEVERITY_TEXT: Record<ReviewSeverity, string> = {
  critical: "text-destructive",
  major: "text-warning",
  minor: "text-info",
  nit: "text-muted-foreground",
};

export type ReviewStatusTone = "running" | "waiting" | "done" | "failed" | "quiet";

export const REVIEW_STATUS: Record<ReviewJobStatus, { label: string; tone: ReviewStatusTone }> = {
  queued: { label: "Queued", tone: "running" },
  preparing: { label: "Preparing the checkout", tone: "running" },
  reviewing: { label: "Reviewing", tone: "running" },
  drafted: { label: "Draft ready", tone: "waiting" },
  posting: { label: "Posting", tone: "running" },
  posted: { label: "Posted", tone: "done" },
  failed: { label: "Failed", tone: "failed" },
  cancelled: { label: "Cancelled", tone: "quiet" },
  interrupted: { label: "Interrupted", tone: "failed" },
};

export const SEVERITIES: ReadonlyArray<ReviewSeverity> = ["critical", "major", "minor", "nit"];

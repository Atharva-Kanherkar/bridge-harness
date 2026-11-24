import type { ReviewSeverity } from "../../protocol/generated/protocol";

// Words and colours the review pane, its draft and its toasts share.

const HARNESS_LABELS: Record<string, string> = { claude: "Claude", codex: "Codex", opencode: "OpenCode" };

export const harnessLabel = (id: string) => HARNESS_LABELS[id] ?? id.charAt(0).toUpperCase() + id.slice(1);

export const SEVERITY_TEXT: Record<ReviewSeverity, string> = {
  critical: "text-destructive",
  major: "text-warning",
  minor: "text-info",
  nit: "text-muted-foreground",
};
